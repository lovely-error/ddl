// `match` lowering, and inlining of user-defined function calls.
//
// Both live here rather than in ir.rs only to keep that file readable; they
// are part of the same lowering pass and share its `Lowerer` and `Env`.


use crate::diag::{Diag, DiagSink};
use crate::lex::{AlphanumSpan, BindingPattern};
use crate::parse::{BuiltinOp, MatchStmt, PrecResExpr, anumspan_to_str};
use crate::ty::Ty;

use crate::ir::{Binding, Env, Lowerer, Op, ValueId, lower_branch, lower_stmts};

/// The first named variant in a pattern, for anchoring diagnostics.
fn first_variant_span(pattern: &BindingPattern) -> Option<AlphanumSpan> {
    match pattern {
        BindingPattern::EnumCase { base, .. } => Some(*base),
        BindingPattern::AnyOf(alts) => alts.iter().find_map(first_variant_span),
        BindingPattern::Alphanum(_) => None,
    }
}

/// What one arm of a `match` selects on, and what it binds.
///
/// Purely syntactic plus a symbol lookup: no environment, no lowering. That is
/// what lets the same analysis serve both readings of a `match` -- the
/// combinational one, which folds the arms into a `case` over VALUES, and the
/// scheduled one, where the arms are states and the `case` is over the next
/// state. The two used to be one function and only the first existed, which is
/// why a `match` could not hold a wait.
pub struct CasePlan {
    /// The discriminants that select this arm. Empty for a catch-all.
    pub labels: Vec<u128>,
    /// `_` or a plain name: binds the whole scrutinee.
    pub catch_all: Option<AlphanumSpan>,
    /// `Read(a)` -- the variant, and the name its payload takes.
    pub payload: Option<(AlphanumSpan, AlphanumSpan)>,
    /// `_ => @unreachable`: selects nothing and supplies nothing.
    pub is_unreachable: bool,
}

/// Every arm of a `match`, checked for coverage.
pub struct MatchShape {
    pub enum_name: String,
    pub tag_width: u32,
    pub payload_width: u32,
    pub total_width: u32,
    /// One per case of the source `match`, in order.
    pub cases: Vec<CasePlan>,
    /// Where to blame the `match` as a whole.
    pub span: crate::diag::Span,
}

impl MatchShape {
    /// The arms that can be selected, in source order, paired with the index
    /// of the case they came from.
    pub fn selectable(&self) -> impl Iterator<Item = (usize, &CasePlan)> {
        self.cases.iter().enumerate().filter(|(_, c)| !c.is_unreachable)
    }
}

/// Resolves the patterns of a `match` and checks that it covers its scrutinee.
pub fn plan_match(
    low: &mut Lowerer,
    stmt: &MatchStmt,
    scrutinee_ty: &Ty,
    sink: &mut DiagSink,
) -> Option<MatchShape> {
    let has_one_scrutinee = stmt.scrutinees.len() == 1;
    if !has_one_scrutinee {
        sink.err_span(
            low.here(),
            format!(
                "`match` takes one scrutinee here, found {}",
                stmt.scrutinees.len()
            ),
        );
        return None;
    }

    let (enum_name, total_width) = match scrutinee_ty {
        Ty::Enum { name, width } => (name.clone(), *width),
        other => {
            sink.push(
                Diag::error(
                    low.here(),
                    format!("`match` needs an enum, found `{}`", other.display()),
                )
                .with_note("compare with `==` instead, or give the value an enum type"),
            );
            return None;
        }
    };
    let variants = low.enum_variants(&enum_name)?;
    let (tag_width, payload_width) = match low.syms.enums.get(&enum_name) {
        Some(def) => (def.tag_width, def.payload_width),
        None => (total_width, 0),
    };

    let mut covered: Vec<String> = Vec::new();
    let mut saw_value_catch_all = false;
    let mut saw_unreachable = false;
    // `match` itself carries no span, so the first pattern stands in for it.
    // Pointing at line 1 of the file was useless when a module had three.
    let mut match_span = low.here();
    for case in &stmt.cases {
        let first_named = case.binding_patterns.first().and_then(first_variant_span);
        if let Some(base) = first_named {
            match_span = low.span_of(&base);
            break;
        }
    }

    let mut cases: Vec<CasePlan> = Vec::new();
    for case in &stmt.cases {
        let has_one_pattern = case.binding_patterns.len() == 1;
        if !has_one_pattern {
            sink.err_span(
                low.here(),
                "this arm has the wrong number of patterns for one scrutinee",
            );
            return None;
        }
        let already_total = saw_value_catch_all || saw_unreachable;
        if already_total {
            sink.err_span(
                match_span,
                "this arm is unreachable: an earlier arm already matches everything",
            );
            return None;
        }

        // `_ => @unreachable` declares that the remaining bit patterns cannot
        // occur. That is what SystemVerilog's `unique case` asserts, and it is
        // what lets a sparse enum be matched on exhaustively by name: the
        // author states the undeclared patterns are impossible, so which arm
        // would serve them stops being a question anyone can answer wrongly.
        // Synthesis is free to treat them as don't-care.
        let declares_rest_impossible =
            matches!(&case.rhs, PrecResExpr::Builtin(BuiltinOp::Unreachable));
        if declares_rest_impossible {
            let is_catch_all =
                matches!(&case.binding_patterns[0], BindingPattern::Alphanum(_));
            if !is_catch_all {
                sink.err_span(
                    match_span,
                    "`@unreachable` belongs on a `_` arm, not on a named variant",
                );
                return None;
            }
            saw_unreachable = true;
            cases.push(CasePlan {
                labels: Vec::new(),
                catch_all: None,
                payload: None,
                is_unreachable: true,
            });
            continue;
        }

        // An or-pattern offers several alternatives for ONE scrutinee
        // position. A lone pattern is the one-alternative case, so both take
        // the same path.
        let alternatives: Vec<&BindingPattern> = match &case.binding_patterns[0] {
            BindingPattern::AnyOf(alts) => alts.iter().collect(),
            other => vec![other],
        };

        let mut catch_all_binding: Option<AlphanumSpan> = None;
        let mut named: Vec<AlphanumSpan> = Vec::new();
        let mut payload_bind: Option<(AlphanumSpan, AlphanumSpan)> = None;
        for alt in &alternatives {
            match alt {
                BindingPattern::EnumCase { base, subbinding } => {
                    named.push(*base);
                    if let Some(bind) = subbinding {
                        payload_bind = Some((*base, *bind));
                    }
                }
                BindingPattern::Alphanum(name) => catch_all_binding = Some(*name),
                // The parser never nests one inside another.
                BindingPattern::AnyOf(_) => unreachable!("or-patterns do not nest"),
            }
        }

        // An irrefutable alternative matches everything and leaves the others
        // dead, so mixing one in is a mistake rather than a shorthand.
        let mixes_binding_with_variants = catch_all_binding.is_some() && !named.is_empty();
        if mixes_binding_with_variants {
            sink.err_span(
                match_span,
                "an alternative cannot mix a catch-all binding with named variants",
            );
            return None;
        }

        let labels = if catch_all_binding.is_some() {
            saw_value_catch_all = true;
            Vec::new()
        } else {
            // Every alternative contributes a case label. Each also counts
            // towards coverage, which is the whole point: grouping opcodes
            // must not cost the exhaustiveness check.
            let mut labels: Vec<u128> = Vec::new();
            for base in &named {
                let variant = anumspan_to_str(base).to_string();
                let belongs_to_this_enum = variants.iter().any(|(n, _)| *n == variant);
                if !belongs_to_this_enum {
                    sink.push(
                        Diag::error(
                            low.span_of(base),
                            format!("`{}` is not a variant of `{}`", variant, enum_name),
                        )
                        .with_note(format!(
                            "expected one of: {}",
                            variants
                                .iter()
                                .map(|(n, _)| n.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )),
                    );
                    return None;
                }
                let already_covered = covered.contains(&variant);
                if already_covered {
                    sink.err_at(base, format!("`{}` is matched twice", variant));
                    return None;
                }
                let discriminant = variants
                    .iter()
                    .find(|(n, _)| *n == variant)
                    .map(|(_, d)| *d)
                    .expect("checked above");
                covered.push(variant);
                labels.push(discriminant);
            }
            labels
        };

        if let Some((variant_span, bind_span)) = payload_bind {
            let binds_one_variant = named.len() == 1;
            if !binds_one_variant {
                sink.err_at(
                    &bind_span,
                    "an alternative with several variants cannot bind a payload",
                );
                return None;
            }
            let carries_one = low
                .syms
                .enums
                .get(&enum_name)
                .and_then(|d| d.payload_of(anumspan_to_str(&variant_span)))
                .is_some();
            if !carries_one {
                let variant = anumspan_to_str(&variant_span).to_string();
                sink.push(
                    Diag::error(
                        low.span_of(&variant_span),
                        format!("`{}` carries no payload to bind", variant),
                    )
                    .with_note(format!("match it as `{} =>`", variant)),
                );
                return None;
            }
        }

        cases.push(CasePlan {
            labels,
            catch_all: catch_all_binding,
            payload: payload_bind,
            is_unreachable: false,
        });
    }

    let has_arms = cases.iter().any(|c| !c.is_unreachable);
    if !has_arms {
        sink.err_span(low.here(), "`match` needs at least one arm");
        return None;
    }

    // `@unreachable` waives the bit-pattern requirement but not the variant
    // one: a variant the author declared is a value that can occur.
    let covers_every_variant = covered.len() == variants.len();
    if !saw_value_catch_all && !covers_every_variant {
        let missing: Vec<&str> = variants
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| !covered.iter().any(|c| c == n))
            .collect();
        sink.push(
            Diag::error(
                match_span,
                format!("`match` does not cover {}", missing.join(", ")),
            )
            // Combinational logic has no memory, so an unmatched value would
            // have to hold its previous output -- that is a latch.
            .with_note("add the missing arms or a `_` catch-all; a partial match would be a latch"),
        );
        return None;
    }

    // Covering every VARIANT is not the same as covering every bit pattern.
    // A sparse enum -- fewer variants than its tag can hold -- leaves values
    // that match no arm, and the hardware can present them however the
    // declared type reads.
    //
    // Without this check the last arm silently became the fallback, so
    // reordering arms that were otherwise equivalent changed what an
    // undeclared pattern produced. Requiring a `_` makes that choice explicit
    // and independent of the order the arms happen to be written in.
    let pattern_count: u128 = if tag_width >= 127 { u128::MAX } else { 1u128 << tag_width };
    let tag_is_fully_populated = (variants.len() as u128) == pattern_count;
    let rest_is_accounted_for = saw_value_catch_all || saw_unreachable;
    if !rest_is_accounted_for && !tag_is_fully_populated {
        let spare = pattern_count - variants.len() as u128;
        sink.push(
            Diag::error(
                match_span,
                format!(
                    "`match` covers every variant of `{}`, but its {}-bit tag has {} pattern(s) that name no variant",
                    enum_name, tag_width, spare
                ),
            )
            .with_note(
                "add `_ =>` with a value, or `_ => @unreachable` if those patterns cannot occur; without one the last arm becomes the fallback, and reordering the arms would change the result",
            ),
        );
        return None;
    }

    Some(MatchShape {
        enum_name,
        tag_width,
        payload_width,
        total_width,
        cases,
        span: match_span,
    })
}

/// The tag a `match` selects on.
///
/// A tagged union is matched on its TAG, not on the whole value: the payload
/// is different from one item to the next and comparing it would mean no arm
/// ever fired. The tag sits in the high bits, so this is a slice, and for an
/// enum with no payloads it is the value itself.
pub fn match_tag(low: &mut Lowerer, shape: &MatchShape, scrutinee: ValueId) -> ValueId {
    if shape.payload_width == 0 {
        return scrutinee;
    }
    let t = low.emit(
        Ty::UInt(shape.tag_width),
        Op::Slice {
            arg: scrutinee,
            hi: shape.total_width - 1,
            lo: shape.payload_width,
        },
    );
    low.name_value_safe(t, format!("{}_tag", shape.enum_name));
    t
}

/// The payload an arm binds, as a value of the payload's own type.
///
/// The slice is a bag of bits; giving it back the payload's type is what keeps
/// a struct payload's fields reachable and a signed one signed -- the same
/// reason a struct field is retyped after its slice.
pub fn payload_value(
    low: &mut Lowerer,
    shape: &MatchShape,
    scrutinee: ValueId,
    variant: &AlphanumSpan,
) -> Option<(ValueId, Ty)> {
    let payload_ty = low
        .syms
        .enums
        .get(&shape.enum_name)
        .and_then(|d| d.payload_of(anumspan_to_str(variant)))
        .cloned()?;
    let bits = payload_ty.bit_width();
    let raw = low.emit(
        Ty::UInt(bits),
        Op::Slice {
            arg: scrutinee,
            hi: shape.payload_width - 1,
            lo: shape.payload_width - bits,
        },
    );
    let value = if low.ty_of(raw) == payload_ty {
        raw
    } else {
        low.emit(payload_ty.clone(), Op::Cast { arg: raw })
    };
    Some((value, payload_ty))
}

/// Lowers a `match` on an enum into a chain of muxes.
///
/// The arms are evaluated into separate environments -- exactly as `if`/`else`
/// does -- and then folded from the last arm backwards, so the first arm ends
/// up outermost and therefore highest priority. That matches the reading order
/// of the source and the `unique case` it replaces.
pub fn lower_match(
    low: &mut Lowerer,
    stmt: &MatchStmt,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    let scrutinee = crate::ir::lower_expr(low, &stmt.scrutinees[0], env, sink)?;
    let scrutinee_ty = low.ty_of(scrutinee);
    let shape = plan_match(low, stmt, &scrutinee_ty, sink)?;
    let tag = match_tag(low, &shape, scrutinee);

    struct Arm {
        labels: Vec<u128>,
        env: Env,
        /// How many write ports this arm took, per memory. Every arm starts
        /// from `base`, which is what makes two arms writing once share one
        /// port rather than asking for one each.
        mem_slots: Vec<usize>,
    }
    let base = low.mem_slot_counts();
    let mut arms: Vec<Arm> = Vec::new();

    for (ix, plan) in shape.cases.iter().enumerate() {
        if plan.is_unreachable {
            continue;
        }
        let case = &stmt.cases[ix];
        let mut arm_env = env.clone();
        low.set_mem_slot_counts(&base);

        // A plain name is an irrefutable binding: it matches anything and
        // binds the scrutinee, which is how the wildcard works too.
        if let Some(name) = plan.catch_all {
            arm_env.insert(
                anumspan_to_str(&name).to_string(),
                Binding {
                    is_mutable: false,
                    value: Some(scrutinee),
                    ty: scrutinee_ty.clone(),
                    is_output: false,
                },
            );
        }

        // The payload, if the arm asked for it. It starts below the tag of the
        // scrutinee -- the whole point of a fixed layout is that the arm knows
        // where to look once the tag has told it what is there.
        if let Some((variant_span, bind_span)) = plan.payload {
            let (value, payload_ty) = payload_value(low, &shape, scrutinee, &variant_span)?;
            let bound = anumspan_to_str(&bind_span).to_string();
            low.name_value_safe(value, bound.clone());
            arm_env.insert(bound, Binding::constant(value, payload_ty));
        }

        // An arm runs when the scrutinee carries one of its labels. The
        // catch-all arm runs when no EARLIER arm claimed the value, which is
        // what the `case` default does, so its guard is the negation of every
        // label taken so far.
        let depth = if plan.labels.is_empty() {
            let claimed: Vec<u128> =
                arms.iter().flat_map(|a: &Arm| a.labels.iter().copied()).collect();
            low.push_labels(tag, claimed, false)
        } else {
            low.push_labels(tag, plan.labels.clone(), true)
        };
        lower_branch(low, &case.rhs, &mut arm_env, sink)?;
        low.pop_path(depth);
        arms.push(Arm {
            labels: plan.labels.clone(),
            env: arm_env,
            mem_slots: low.mem_slot_counts(),
        });
    }

    // The last arm is the default: either it is the catch-all, or coverage is
    // complete and it is reached by elimination.
    let default_arm = arms.last().expect("plan_match rejects a match with no arms");
    // Write ports are joined separately, below: they are created as the arms
    // run, so the keys an arm added are not in `env` here to be walked, and
    // the arms can have added different numbers of them.
    let names: Vec<String> = env.keys().filter(|k| !k.contains('#')).cloned().collect();

    for name in names {
        let fallback = match default_arm.env.get(&name).and_then(|b| b.value) {
            Some(v) => v,
            None => continue,
        };

        // Only arms that actually changed this binding become case labels.
        // Without this every arm contributed an entry for every name in
        // scope, which for a module with a few matches is most of the output.
        let mut case_arms: Vec<(Vec<u128>, ValueId)> = Vec::new();
        let mut ty_conflict = false;

        for arm in arms.iter().take(arms.len() - 1) {
            let this = match arm.env.get(&name).and_then(|b| b.value) {
                Some(v) => v,
                None => continue,
            };
            let arm_agrees = this == fallback;
            if arm_agrees {
                continue;
            }
            let arm_ty = low.ty_of(this);
            let acc_ty = low.ty_of(fallback);
            if arm_ty != acc_ty {
                sink.err_span(
                    low.here(),
                    format!(
                        "`{}` is `{}` on one arm and `{}` on another",
                        name,
                        arm_ty.display(),
                        acc_ty.display()
                    ),
                );
                ty_conflict = true;
                break;
            }
            case_arms.push((arm.labels.clone(), this));
        }
        if ty_conflict {
            return None;
        }
        if case_arms.is_empty() {
            continue;
        }

        let ty = low.ty_of(fallback);
        let joined = low.emit(
            ty,
            Op::Case { scrutinee: tag, arms: case_arms, default: fallback },
        );
        if let Some(b) = env.get_mut(&name) {
            b.value = Some(joined);
        }
    }

    let port_arms: Vec<(Vec<u128>, Env, Vec<usize>)> = arms
        .into_iter()
        .map(|a| (a.labels, a.env, a.mem_slots))
        .collect();
    low.join_write_slots_case(tag, &base, &port_arms, env);
    Some(())
}

/// Inlines a call to a user-defined combinational function.
///
/// Inlining rather than instantiating a submodule: everything here is
/// combinational, so a call is just more of the caller's own value graph, and
/// this keeps the Verilog free of hierarchy the source did not ask for.
pub fn inline_call(
    low: &mut Lowerer,
    callee: &AlphanumSpan,
    args: &[PrecResExpr],
    env: &Env,
    sink: &mut DiagSink,
) -> Option<ValueId> {
    let name = anumspan_to_str(callee).to_string();
    let sig = signature_of(low, callee, sink)?;
    let outputs: Vec<_> = sig.outputs().cloned().collect();

    // An expression is one value. A function with several `out` parameters is
    // called with a tuple binding instead -- see `inline_call_multi`.
    let has_exactly_one_output = outputs.len() == 1;
    if !has_exactly_one_output {
        sink.push(
            Diag::error(
                low.span_of(callee),
                format!(
                    "`{}` has {} outputs, so it cannot be used as an expression",
                    name,
                    outputs.len()
                ),
            )
            .with_note(
                "a function used in an expression needs exactly one `out` parameter; bind several with `let (a, b) = f(..)`",
            ),
        );
        return None;
    }

    let callee_env = inline_body(low, callee, args, &sig, env, sink)?;
    let (out_name, _, _) = &outputs[0];
    match callee_env.get(out_name).and_then(|b| b.value) {
        Some(v) => Some(v),
        None => {
            sink.err_at(
                callee,
                format!("`{}` never assigns its output `{}`", name, out_name),
            );
            None
        }
    }
}

/// `let (a, b, c) = f(x, y)` -- one name per `out` parameter, in declaration
/// order.
///
/// The same inlining as the expression form; only what happens to the results
/// differs. Statement position rather than expression position because a call
/// producing several values has nowhere to sit inside a larger expression --
/// which is also why the two verified helpers this exists for, `k2g_alu` with
/// five outputs and `k2g_shift` with three, were unreachable until now.
/// `f(a, b)` as a statement, for a function whose only results are `inout`.
///
/// There is nothing to bind -- the results went back into the arguments -- so
/// this is a call made for its effect on them.
pub fn inline_call_effect(
    low: &mut Lowerer,
    callee: &AlphanumSpan,
    args: &[PrecResExpr],
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    let name = anumspan_to_str(callee).to_string();
    let sig = signature_of(low, callee, sink)?;
    let has_plain_outputs = sig.outputs().any(|(_, d, _)| *d == crate::symbols::ParamDir::Out);
    if has_plain_outputs {
        sink.push(
            Diag::error(
                low.span_of(callee),
                format!("`{}` has `out` parameters, so its results need binding", name),
            )
            .with_note("write `let (a, b) = f(..)`, or make the parameters `inout`"),
        );
        return None;
    }
    if sig.inouts().next().is_none() {
        sink.push(
            Diag::error(
                low.span_of(callee),
                format!("`{}` produces nothing, so calling it does nothing", name),
            )
            .with_note("give it an `out` parameter to return a value, or an `inout` one to update its argument"),
        );
        return None;
    }
    let callee_env = inline_body(low, callee, args, &sig, env, sink)?;
    write_back_inouts(low, callee, args, &sig, &callee_env, env, sink)
}

pub fn inline_call_multi(
    low: &mut Lowerer,
    callee: &AlphanumSpan,
    args: &[PrecResExpr],
    results: &[AlphanumSpan],
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    let name = anumspan_to_str(callee).to_string();
    let sig = signature_of(low, callee, sink)?;
    let outputs: Vec<_> = sig.outputs().cloned().collect();

    if outputs.is_empty() {
        sink.push(
            Diag::error(
                low.span_of(callee),
                format!("`{}` has no `out` parameters, so it produces nothing to bind", name),
            )
            .with_note("give it an `out` parameter, or call it for its effect if it had one"),
        );
        return None;
    }
    let bound_outputs: Vec<_> = outputs
        .iter()
        .filter(|(_, d, _)| *d == crate::symbols::ParamDir::Out)
        .cloned()
        .collect();
    let outputs = bound_outputs;
    let names_match_outputs = results.len() == outputs.len();
    if !names_match_outputs {
        let out_names: Vec<&str> = outputs.iter().map(|(n, _, _)| n.as_str()).collect();
        sink.push(
            Diag::error(
                low.span_of(callee),
                format!(
                    "`{}` has {} outputs but {} name(s) were bound",
                    name,
                    outputs.len(),
                    results.len()
                ),
            )
            .with_note(format!("its outputs, in order: {}", out_names.join(", "))),
        );
        return None;
    }

    let callee_env = inline_body(low, callee, args, &sig, env, sink)?;

    for (bind_span, (out_name, _, out_ty)) in results.iter().zip(outputs.iter()) {
        let v = match callee_env.get(out_name).and_then(|b| b.value) {
            Some(v) => v,
            None => {
                sink.err_at(
                    callee,
                    format!("`{}` never assigns its output `{}`", name, out_name),
                );
                return None;
            }
        };
        let bound = anumspan_to_str(bind_span).to_string();
        low.name_value_safe(v, bound.clone());
        env.insert(bound, Binding::constant(v, out_ty.clone()));
    }
    write_back_inouts(low, callee, args, &sig, &callee_env, env, sink)
}

fn signature_of(
    low: &Lowerer,
    callee: &AlphanumSpan,
    sink: &mut DiagSink,
) -> Option<crate::symbols::FuncSig> {
    let name = anumspan_to_str(callee).to_string();
    match low.syms.funcs.get(&name) {
        Some(s) => Some(s.clone()),
        None => {
            sink.err_at(callee, format!("`{}` is not a function", name));
            None
        }
    }
}

/// Checks arity and recursion, binds the arguments into a fresh scope, lowers
/// the callee body, and hands back the environment it left behind.
///
/// The caller decides what to do with the outputs; everything before that is
/// the same whether one value is wanted or five.
/// Copies each `inout` result back into the caller's variable.
///
/// This is what "by reference" means here: the call is inlined, so there is no
/// pointer to write through -- the caller's binding is simply replaced with
/// what the callee left. The argument therefore has to be a name; an
/// expression has nowhere for the answer to go, and saying so beats silently
/// discarding it.
fn write_back_inouts(
    low: &mut Lowerer,
    callee: &AlphanumSpan,
    args: &[PrecResExpr],
    sig: &crate::symbols::FuncSig,
    callee_env: &Env,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    let name = anumspan_to_str(callee).to_string();
    for (ix, (param_name, dir, _)) in sig.inputs().enumerate() {
        if *dir != crate::symbols::ParamDir::InOut {
            continue;
        }
        let target = match args.get(ix) {
            Some(PrecResExpr::Ref(n)) => *n,
            _ => {
                sink.push(
                    Diag::error(
                        low.span_of(callee),
                        format!(
                            "argument {} of `{}` is `inout`, so it has to be a variable",
                            ix + 1,
                            name
                        ),
                    )
                    .with_note(format!(
                        "`{}` is written back to whatever is passed for it",
                        param_name
                    )),
                );
                return None;
            }
        };
        let target_name = anumspan_to_str(&target).to_string();
        let produced = match callee_env.get(param_name).and_then(|b| b.value) {
            Some(v) => v,
            None => {
                sink.err_at(callee, format!("`{}` never assigns `{}`", name, param_name));
                return None;
            }
        };
        let binding = match env.get(&target_name) {
            Some(b) => b.clone(),
            None => {
                sink.err_at(&target, format!("`{}` is not declared", target_name));
                return None;
            }
        };
        if !(binding.is_mutable || binding.is_output) {
            sink.push(
                Diag::error(
                    low.span_of(&target),
                    format!("`{}` is a `let` binding and cannot be assigned", target_name),
                )
                .with_note("an `inout` argument is written to; declare it `var`"),
            );
            return None;
        }
        env.insert(
            target_name,
            Binding { value: Some(produced), ty: binding.ty, ..binding },
        );
    }
    Some(())
}

fn inline_body(
    low: &mut Lowerer,
    callee: &AlphanumSpan,
    args: &[PrecResExpr],
    sig: &crate::symbols::FuncSig,
    env: &Env,
    sink: &mut DiagSink,
) -> Option<Env> {
    let name = anumspan_to_str(callee).to_string();
    let inputs: Vec<_> = sig.inputs().cloned().collect();
    let outputs: Vec<_> = sig.outputs().cloned().collect();

    let arity_matches = args.len() == inputs.len();
    if !arity_matches {
        sink.err_at(
            callee,
            format!(
                "`{}` takes {} argument(s), found {}",
                name,
                inputs.len(),
                args.len()
            ),
        );
        return None;
    }

    let is_recursive = low.call_stack.contains(&name);
    if is_recursive {
        sink.push(
            Diag::error(
                low.span_of(callee),
                format!("`{}` calls itself", name),
            )
            .with_note("a recursive combinational function is a circuit that never settles"),
        );
        return None;
    }

    let body = match low.bodies.get(&name) {
        Some(b) => *b,
        None => {
            sink.err_at(callee, format!("`{}` has no body", name));
            return None;
        }
    };

    // Bind arguments into a fresh scope. The callee sees only its parameters,
    // never the caller's locals.
    let mut callee_env: Env = Env::new();
    for (arg_expr, (param_name, _, param_ty)) in args.iter().zip(inputs.iter()) {
        let mut value = crate::ir::lower_expr(low, arg_expr, env, sink)?;
        let have = low.ty_of(value);
        if have != *param_ty {
            match low.coerce_const_pub(value, param_ty) {
                Some(v) => value = v,
                None => {
                    sink.push(
                        Diag::error(
                            low.span_of(callee),
                            format!(
                                "argument `{}` of `{}` is `{}` but `{}` was given",
                                param_name,
                                name,
                                param_ty.display(),
                                have.display()
                            ),
                        )
                        .with_note(crate::ir::cast_hint_pub(&have, param_ty)),
                    );
                    return None;
                }
            }
        }
        callee_env.insert(param_name.clone(), Binding::constant(value, param_ty.clone()));
    }
    // An `inout` was just bound from its argument, which is what makes it
    // readable; the loop below must not overwrite that with `None`.
    for (out_name, dir, out_ty) in &outputs {
        if *dir == crate::symbols::ParamDir::InOut {
            if let Some(b) = callee_env.get_mut(out_name) {
                b.is_output = true;
            }
            continue;
        }
        callee_env.insert(
            out_name.clone(),
            Binding { value: None, ty: out_ty.clone(), is_output: true, is_mutable: false },
        );
    }

    low.call_stack.push(name.clone());
    let lowered = lower_stmts(low, &body.body, &mut callee_env, sink);
    low.call_stack.pop();
    lowered?;

    Some(callee_env)
}
