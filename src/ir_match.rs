// `match` lowering, and inlining of user-defined function calls.
//
// Both live here rather than in ir.rs only to keep that file readable; they
// are part of the same lowering pass and share its `Lowerer` and `Env`.

use std::collections::HashMap;

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
    let has_one_scrutinee = stmt.scrutinees.len() == 1;
    if !has_one_scrutinee {
        sink.err_span(
            crate::driver::nowhere(),
            format!(
                "`match` takes one scrutinee here, found {}",
                stmt.scrutinees.len()
            ),
        );
        return None;
    }

    let scrutinee = crate::ir::lower_expr(low, &stmt.scrutinees[0], env, sink)?;
    let scrutinee_ty = low.ty_of(scrutinee);

    let (enum_name, tag_width) = match &scrutinee_ty {
        Ty::Enum { name, width } => (name.clone(), *width),
        other => {
            sink.push(
                Diag::error(
                    crate::driver::nowhere(),
                    format!("`match` needs an enum, found `{}`", other.display()),
                )
                .with_note("compare with `==` instead, or give the value an enum type"),
            );
            return None;
        }
    };
    let variants = low.enum_variants(&enum_name)?;

    // Each arm becomes (selector, environment-after-the-arm). A `None`
    // selector is a catch-all and ends the chain.
    struct Arm {
        /// The discriminants that select this arm. Empty for a catch-all.
        labels: Vec<u128>,
        env: Env,
    }
    let mut arms: Vec<Arm> = Vec::new();
    let mut covered: Vec<String> = Vec::new();
    // A `_` arm with a body supplies a value for anything unmatched. An
    // `@unreachable` arm supplies nothing -- it only asserts that nothing
    // reaches it -- so it waives the bit-pattern requirement but not the
    // requirement to handle every declared variant.
    let mut saw_value_catch_all = false;
    let mut saw_unreachable = false;
    // `match` itself carries no span, so the first pattern stands in for it.
    // Pointing at line 1 of the file was useless when a module had three.
    let mut match_span = crate::driver::nowhere();
    for case in &stmt.cases {
        let first_named = case.binding_patterns.first().and_then(first_variant_span);
        if let Some(base) = first_named {
            match_span = low.span_of(&base);
            break;
        }
    }

    for case in &stmt.cases {
        let has_one_pattern = case.binding_patterns.len() == 1;
        if !has_one_pattern {
            sink.err_span(
                crate::driver::nowhere(),
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
            continue;
        }

        let mut arm_env = env.clone();

        // An or-pattern offers several alternatives for ONE scrutinee
        // position. A lone pattern is the one-alternative case, so both take
        // the same path.
        let alternatives: Vec<&BindingPattern> = match &case.binding_patterns[0] {
            BindingPattern::AnyOf(alts) => alts.iter().collect(),
            other => vec![other],
        };

        let mut catch_all_binding: Option<AlphanumSpan> = None;
        let mut named: Vec<AlphanumSpan> = Vec::new();
        for alt in &alternatives {
            match alt {
                BindingPattern::EnumCase { base, subbinding } => {
                    if subbinding.is_some() {
                        sink.err_at(base, "enum variants carry no payload to bind");
                        return None;
                    }
                    named.push(*base);
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

        let labels = if let Some(name) = catch_all_binding {
            // A plain name is an irrefutable binding: it matches anything and
            // binds the scrutinee, which is how the wildcard works too.
            saw_value_catch_all = true;
            let bound = anumspan_to_str(&name).to_string();
            arm_env.insert(
                bound,
                Binding {
                    value: Some(scrutinee),
                    ty: scrutinee_ty.clone(),
                    is_output: false,
                },
            );
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

        // An arm runs when the scrutinee carries one of its labels. The
        // catch-all arm runs when no EARLIER arm claimed the value, which is
        // what the `case` default does, so its guard is the negation of every
        // label taken so far.
        let depth = if labels.is_empty() {
            let claimed: Vec<u128> =
                arms.iter().flat_map(|a: &Arm| a.labels.iter().copied()).collect();
            low.push_labels(scrutinee, claimed, false)
        } else {
            low.push_labels(scrutinee, labels.clone(), true)
        };
        lower_branch(low, &case.rhs, &mut arm_env, sink)?;
        low.pop_path(depth);
        arms.push(Arm { labels, env: arm_env });
    }

    if arms.is_empty() {
        sink.err_span(crate::driver::nowhere(), "`match` needs at least one arm");
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

    // The last arm is the default: either it is the catch-all, or coverage is
    // complete and it is reached by elimination.
    let default_arm = arms.last().expect("checked non-empty");
    let names: Vec<String> = env.keys().cloned().collect();

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
                    crate::driver::nowhere(),
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
            Op::Case { scrutinee, arms: case_arms, default: fallback },
        );
        if let Some(b) = env.get_mut(&name) {
            b.value = Some(joined);
        }
    }
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

    let sig = match low.syms.funcs.get(&name) {
        Some(s) => s.clone(),
        None => {
            sink.err_at(callee, format!("`{}` is not a function", name));
            return None;
        }
    };

    let outputs: Vec<_> = sig.outputs().cloned().collect();
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
            .with_note("a function used in an expression needs exactly one `out` parameter"),
        );
        return None;
    }

    let inputs: Vec<_> = sig.inputs().cloned().collect();
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
        callee_env.insert(
            param_name.clone(),
            Binding { value: Some(value), ty: param_ty.clone(), is_output: false },
        );
    }
    let (out_name, _, out_ty) = &outputs[0];
    callee_env.insert(
        out_name.clone(),
        Binding { value: None, ty: out_ty.clone(), is_output: true },
    );

    low.call_stack.push(name.clone());
    let lowered = lower_stmts(low, &body.body, &mut callee_env, sink);
    low.call_stack.pop();
    lowered?;

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
