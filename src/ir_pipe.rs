// `sequence` lowered to a pipeline.
//
// `|||` cuts the body into stages. Everything inside a stage is combinational;
// every cut becomes a register bank, and a validity bit rides alongside the
// data -- desc.md:80's "implicit is_valid condition at each stage".
//
// EACH STAGE SHIFTS ON ITS OWN. Stage k moves its item on when every sink it
// sends to has a slot and the cut below it is free -- empty, or emptying
// because the stage below is moving too:
//
//     shift_k = room_k & (!v_k | shift_{k+1})
//
// desc.md:77's "pipeline fires when all buffer sinks have slots", said per
// stage. Latency is one cycle per stage; throughput is one item per cycle
// while the sinks keep up.
//
// It used to be one `shift` for the whole pipeline, which was enough while
// every send sat in the last stage. It stops being enough the moment two sends
// sit in different ones. Send `x` in stage 0 and `y` in stage 2 to a consumer
// that joins them, and `x` fills with two items while the first one's `y` is
// still two stages up; one global shift then waits for `x` to drain, `x`
// waits for `y`, and `y` waits for the shift. Per stage, the stages below the
// full sink keep going, the late half arrives, and the join drains the early
// one. A stall also closes up the bubbles behind it rather than freezing them
// in place.
//
// What that costs is a chain: `shift_0` depends on every stage below it
// within the cycle. Every term in it is a register on this side -- a validity
// bit, or a slot's occupancy -- so the chain is local to the module and never
// reaches through a pipe into another one.
//
// A value defined in one stage and read two stages later needs TWO registers,
// not one. It has to arrive alongside the item it belongs to, and that item is
// two cycles further down the pipe -- registering it once would pair stage 2's
// item with stage 0's value. So a crossing value is registered at every
// boundary it spans, which is a shift register of exactly the right depth.
//
// Channel rule 3 holds as it does everywhere else: what an output offers is
// its slot's `wsalt`, which is a register, whichever stage wrote it.
//
// A `bram` READ is the one thing here that already had a register and did not
// need one of ours. It answers a cycle after the address, and a cycle is
// exactly what a stage cut is, so the memory's own output register IS the
// pipeline register at that boundary: the address goes out in stage k and the
// name means `<mem>_q` from stage k+1 on. What that costs to say is a read
// enable tied to the shift, and a rule that the value is not there yet in the
// stage that asked for it.
//
// A HEAD IS A JOIN AND A STAGE'S SENDS ARE A SCATTER. A sequence may name any
// number of `buffer in` and `buffer out` parameters. Every input is received
// exactly once in stage 0, and every output sent to exactly once in whichever
// stage the source puts its `@send`. The head moves as one: `offered` is the
// AND over the inputs that are offering, and one `take` serves all of them.
// So do the sends of one stage: its `room` is the AND over the sinks it sends
// to, and one `push` serves all of them. Every sink or none, for the reason
// ir_comb.rs:234 gives about `@split` -- ANDing the sinks' readys would put
// each consumer's logic in every other one's timing path, where asking whether
// each has a slot only reads registers on this side.
//
// One input may instead be received with `@try_rcv`, which makes it OPTIONAL:
// it is absent from `offered`, so it never holds the pipeline up, and its
// `rsalt` advances on `take & <p>_present` so it gives up an item only on the
// cycles it had one. `ok` is that same `present` bit and crosses the stage
// cuts like any other value. With no blocking input left, `offered` becomes
// the OR rather than the AND -- a cycle where nothing arrived carries nothing,
// which is a bubble the validity chain already expresses and `push` already
// respects, and what it must not be is a constant, which would make the head a
// free-running source.
//
// The widening reaches two things that do not otherwise change. A write and
// an assertion are gated on their stage being live and moving, so with several
// pipes they wait for every blocking input to have delivered and for the
// stage's own shift.
//
// MEMORY ORDER. A memory a stage writes is storage, and storage needs an
// order. The only one a pipeline can offer is the item stream, and it can
// offer that only if ONE stage does all the touching -- at any cycle stage j
// and stage k hold different items, so a write in one and a read in the other
// pair an item's read with a write belonging to an item (k - j) places away,
// and an item's own two writes would not even be adjacent. Confined to one
// stage the guarantee is the one the source reads as: every item sees every
// write of every item before it, plus its own, in source order.
//
// "Plus its own" is the part that needs hardware. The array is written on the
// same edge that fills `_q`, so `_q` holds the contents from BEFORE this
// item's write, and the write has to be forwarded around it: which addresses
// collided is decided back in the stage that asked, and one bit plus the data
// crosses the cut to answer the read on the far side. A memory nothing writes
// has no order to keep and may be read from any stage, any number of times.
//
// READ PORTS ARE PLURAL, unlike a process's. There, one state is current and
// several reads mux onto one port; here every stage is live at once, so two
// reads are two addresses in one cycle with nothing to mux them onto. An ASIC
// memory compiler answers that with a multi-read cell and an FPGA by
// replicating the array under the same write stream -- the tool's choice, and
// it can only make it from a template that asks for all of them together.

use std::collections::{HashMap, HashSet};

use crate::diag::{Diag, DiagSink};
use crate::ir::{
    BinOp, Binding, Env, Lowerer, Op, PortId, ReadPort, Reg, SALT, UnOp, ValueId,
};
use crate::ir_fsm::{as_sync_read, reads_of};
use crate::parse::{
    BuiltinOp, PrecResExpr, PrecResInnerStmt, PrecSeqInnerStmt, VarDeclStmt, anumspan_to_str,
};
use crate::ty::{MemKind, Ty, resolve_type_expr};

/// Splits a sequence body at `|||`.
fn split_stages(body: &[PrecSeqInnerStmt]) -> Vec<Vec<&PrecResInnerStmt>> {
    let mut stages = vec![Vec::new()];
    for item in body {
        match item {
            PrecSeqInnerStmt::SegmentSeparator => stages.push(Vec::new()),
            PrecSeqInnerStmt::Stmt(s) => stages.last_mut().expect("never empty").push(s),
        }
    }
    stages
}

/// `let x = @rcv(p)`, the head stage's input.
fn as_recv(stmt: &PrecResInnerStmt) -> Option<(&VarDeclStmt, String)> {
    let decl = match stmt {
        PrecResInnerStmt::VarDecl(d) if d.names().len() == 1 => d,
        _ => return None,
    };
    let (base, args) = match decl.assign_val.as_ref()? {
        PrecResExpr::Call { base, args } => (base, args),
        _ => return None,
    };
    match &**base {
        PrecResExpr::Builtin(BuiltinOp::BlockingRecieve) if args.len() == 1 => {}
        _ => return None,
    }
    match &args[0] {
        PrecResExpr::Ref(n) => Some((decl, anumspan_to_str(n).to_string())),
        _ => None,
    }
}

/// `let (x, ok) = @try_rcv(p)`, an OPTIONAL input of the head stage.
///
/// TWO names rather than one, which is what keeps this out of `as_recv`'s way:
/// the pair is the item and whether there was one. The head does not wait on a
/// pipe read this way -- it fires without it and says so in `ok` -- so the pipe
/// contributes nothing to `offered` and advances its own `rsalt` only on the
/// cycles it actually had something.
fn as_try_recv(stmt: &PrecResInnerStmt) -> Option<(&VarDeclStmt, String)> {
    let decl = match stmt {
        PrecResInnerStmt::VarDecl(d) if d.names().len() == 2 => d,
        _ => return None,
    };
    let (base, args) = match decl.assign_val.as_ref()? {
        PrecResExpr::Call { base, args } => (base, args),
        _ => return None,
    };
    match &**base {
        PrecResExpr::Builtin(BuiltinOp::TryRecieve) if args.len() == 1 => {}
        _ => return None,
    }
    match &args[0] {
        PrecResExpr::Ref(n) => Some((decl, anumspan_to_str(n).to_string())),
        _ => None,
    }
}

/// A channel or memory read has its producer's type; an annotation must agree.
/// These reads bypass ordinary initializer lowering to account for their
/// handshake or latency, but must still check the declaration they initialize.
fn check_read_type(
    low: &Lowerer,
    decl: &VarDeclStmt,
    have: &Ty,
    sink: &mut DiagSink,
) -> Option<()> {
    if let Some(t) = &decl.ty_expr {
        let want = match resolve_type_expr(t, low.syms) {
            Ok(ty) => ty,
            Err(e) => {
                sink.err_at(&decl.head_name(), e.message());
                return None;
            }
        };
        if want != *have {
            sink.err_at(&decl.head_name(), format!(
                "`{}` is declared `{}` but its initialiser is `{}`",
                anumspan_to_str(&decl.head_name()), want.display(), have.display(),
            ));
            return None;
        }
    }
    Some(())
}

/// `@send(p, v)`, an output of whichever stage it sits in.
fn as_send(stmt: &PrecResInnerStmt) -> Option<(String, PrecResExpr)> {
    let call = match stmt {
        PrecResInnerStmt::CallStmt(c) => c,
        _ => return None,
    };
    match &call.base {
        PrecResExpr::Builtin(BuiltinOp::BlockingSend) if call.args.len() == 2 => {}
        _ => return None,
    }
    match &call.args[0] {
        PrecResExpr::Ref(n) => Some((anumspan_to_str(n).to_string(), call.args[1].clone())),
        _ => None,
    }
}

/// A `bram` read that has had its address presented, waiting for the cut it
/// lands on.
///
/// `stage` is where the address went out; the value appears in `stage + 1`.
/// `forward` is the write this read collided with, captured AT THE READ so it
/// carries the writes that precede it in source order and not the ones after.
struct IssuedRead {
    mem_ix: usize,
    /// Which of the memory's read ports this one took, or `None` when it took
    /// none: a read whose address is unconditionally written above it in the
    /// same stage is answered by that write and never looks at the array.
    port: Option<usize>,
    bind: String,
    stage: usize,
    /// `(hit, data)` from `Lowerer::pending_write`, both stage-`stage` values.
    /// `hit` is `1` exactly when `port` is `None`.
    forward: Option<(ValueId, ValueId)>,
    /// `addr < len` when the address can overrun this memory, a stage-`stage`
    /// value like `forward`. `None` when the depth is a power of two and no
    /// address can name a missing element.
    in_bounds: Option<ValueId>,
}

/// Declares the memories at the top of a sequence body, and returns how many
/// statements that took.
///
/// Same place a `process` puts them, and for the same reason: storage is
/// something the module HAS rather than something a stage does. The rules
/// about which kinds may be reset are the process's rules, unchanged -- a
/// `bram` reset is a write to every element and does not infer a block RAM.
fn declare_memories(
    low: &mut Lowerer,
    stage0: &[&PrecResInnerStmt],
    env: &mut Env,
    map: &crate::diag::SourceMap,
    sink: &mut DiagSink,
) -> Option<usize> {
    let mut taken = 0;
    for stmt in stage0 {
        let decl = match stmt {
            PrecResInnerStmt::VarDecl(d) if d.is_mutable && d.names().len() == 1 => d,
            _ => break,
        };
        let ty = match &decl.ty_expr {
            Some(t) => match resolve_type_expr(t, low.syms) {
                Ok(t) => t,
                // Not this pass's error to report. Leaving it in the body
                // hands it to lowering, which says the same thing with an
                // anchor.
                Err(_) => break,
            },
            None => break,
        };
        let Ty::Mem { elem, len, kind } = ty else {
            break;
        };
        let name = anumspan_to_str(&decl.head_name()).to_string();
        let at = map.span_of(&decl.head_name());
        if kind == MemKind::BankedRam {
            sink.push(
                Diag::error(at, "`#[impl(bkram)]` is not supported yet").with_note(
                    "banking needs a conflict model; `lutram` and `bram` are available",
                ),
            );
            return None;
        }
        // An initialiser resets every element to the same constant, which for
        // a `bram` is a write to every bit and infers flip-flops rather than a
        // block RAM -- the same refusal a process makes, for the same measured
        // reason.
        let reset = match &decl.assign_val {
            None => None,
            Some(e) => {
                let v = crate::ir::lower_expr_expecting(low, e, Some(&elem), env, sink)?;
                match low.const_of(v) {
                    Some(k) => Some(k),
                    None => {
                        sink.push(
                            Diag::error(at, "a memory resets every element to the same constant")
                                .with_note(
                                    "write `@zeroed()`, or leave the initialiser off for a memory that powers up undefined",
                                ),
                        );
                        return None;
                    }
                }
            }
        };
        if kind == MemKind::BlockRam && reset.is_some() {
            sink.push(
                Diag::error(at, format!("`{}` is a block RAM, which cannot be reset", name))
                    .with_note(
                        "leave the initialiser off; a block RAM powers up from the bitstream, and resetting one costs a flip-flop per bit",
                    ),
            );
            return None;
        }
        low.declare_memory(name, *elem, len, kind, reset, env);
        taken += 1;
    }
    Some(taken)
}

/// Registers `v` across one cut, and answers with the register.
///
/// The same shape the crossing loop builds by hand for a named binding: a slot,
/// a `RegRead` off it, and a next-value that takes `v` when the stage above the
/// cut moves and holds otherwise. Pulled out because a forwarded write needs
/// two of these and is not a name in the environment, so the loop that walks
/// `env` cannot make them.
#[allow(clippy::too_many_arguments)]
fn cross(
    low: &mut Lowerer,
    pending: &mut Vec<(usize, String, Ty, ValueId)>,
    next_slot: &mut usize,
    name: String,
    ty: Ty,
    v: ValueId,
    shift: Option<ValueId>,
) -> ValueId {
    let slot = *next_slot;
    *next_slot += 1;
    let held = low.emit(ty.clone(), Op::RegRead(slot as u32));
    low.name_value(held, name.clone());
    let next = load_on(low, shift, ty.clone(), v, held);
    pending.push((slot, name, ty, next));
    held
}

/// `v` on the cycles `shift` says the stage moves, `held` on the others.
///
/// `None` is a stage nothing below can ever hold up -- no sink at or below it
/// -- which moves on every cycle, so there is no mux to build.
fn load_on(low: &mut Lowerer, shift: Option<ValueId>, ty: Ty, v: ValueId, held: ValueId) -> ValueId {
    match shift {
        Some(cond) => low.emit(ty, Op::Mux { cond, then_val: v, else_val: held }),
        None => v,
    }
}

/// Whether a stage is live AND moving: the one predicate a stage's writes,
/// assertions, reads and pushes all happen on.
fn moving(low: &mut Lowerer, live: ValueId, shift: Option<ValueId>) -> ValueId {
    match shift {
        Some(s) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: live, rhs: s }),
        None => live,
    }
}

/// `base` for a one-stage sequence, `base{k}` otherwise -- the rule
/// `<mem>_re` and `<mem>_re{p}` follow, so a single-stage pipeline keeps the
/// names it always had.
fn per_stage(base: &str, k: usize, of: usize) -> String {
    if of == 1 { base.to_string() } else { format!("{}{}", base, k) }
}

/// The head's gather: the `in` pipes, in declaration order.
///
/// A sequence takes one item from every blocking input on the same cycle, so
/// these travel together everywhere -- `offered` is the reduction over their
/// `present` bits, and one `take` advances all of them.
struct Head {
    ix: Vec<usize>,
    /// Whether the receive was a blocking `@rcv`. A `@try_rcv` input is
    /// OPTIONAL: it never holds the pipeline up, and it gives up an item only
    /// on the cycles it had one.
    blocking: Vec<bool>,
    rsalt_q: Vec<ValueId>,
    rsalt_slot: Vec<usize>,
    /// `data[ridx]`, emitted exactly once per pipe: `pipe_item_at` records the
    /// index ON the pipe, so a second call would overwrite it.
    item: Vec<ValueId>,
    /// `!empty` -- whether this pipe is offering. Emitted on demand and cached,
    /// because a `@try_rcv` binds its own as `ok` while `offered` wants them
    /// all. One fact, so one name.
    present: Vec<Option<ValueId>>,
}

/// The sends' scatter: the `wsalt` half of each `out` pipe's slot, which is all
/// the shift enables need and all that can exist before the stages run.
struct Tail {
    ix: Vec<usize>,
    wsalt_q: Vec<ValueId>,
    /// `e0`, `e1`, `wsalt_q` -- three consecutive slots, as `ir_comb::out_side`.
    base: Vec<usize>,
}

/// Whether input `j` is offering an item, emitted at most once.
fn head_present(low: &mut Lowerer, head: &mut Head, j: usize) -> ValueId {
    if let Some(v) = head.present[j] {
        return v;
    }
    let ix = head.ix[j];
    let empty = low.pipe_empty(ix, head.rsalt_q[j]);
    let v = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: empty });
    let name = format!("{}_present", low.pipes[ix].name);
    low.name_value_safe(v, name);
    head.present[j] = Some(v);
    v
}

/// Whether stage `k` is holding an item this cycle.
///
/// Stage 0 has one when the head gathered one; stage k has one when the cut
/// behind it passed one on. `offered` is emitted at most once and only for a
/// sequence that asks -- a pipeline with no memory in it computes the same
/// thing further down, and emitting it early would renumber every wire in
/// every module that never needed this. That laziness is worth more now than
/// it was, because the answer is a reduction over N pipes rather than one
/// compare.
fn stage_live(
    low: &mut Lowerer,
    k: usize,
    offered: &mut Option<ValueId>,
    head: &mut Head,
    valid_base: usize,
) -> ValueId {
    if k == 0 {
        if let Some(v) = offered {
            return *v;
        }
        // A JOIN. Stage 0 holds an item when every BLOCKING input delivered
        // one, because the head takes one from each and they have to be the
        // same item's inputs. One input short and nothing moves -- and nothing
        // is taken from the others either, because `take` is shared.
        //
        // With no blocking input there is nothing to wait for, and the question
        // becomes whether ANY input delivered. A cycle where none did carries
        // nothing, and a cycle carrying nothing is a bubble: it shifts through
        // and commits nothing, which the validity chain already expresses and
        // `push` already respects. What it must not be is a constant 1, which
        // would make the head a free-running source emitting an item per cycle
        // out of nothing.
        let blocking: Vec<usize> = (0..head.ix.len()).filter(|j| head.blocking[*j]).collect();
        let is_a_join = !blocking.is_empty();
        let which: Vec<usize> = if is_a_join { blocking } else { (0..head.ix.len()).collect() };
        let mut have = Vec::with_capacity(which.len());
        for j in which {
            have.push(head_present(low, head, j));
        }
        let v = if is_a_join {
            crate::ir_comb::all_of(low, &have)
        } else {
            crate::ir_comb::any_of(low, &have)
        };
        // With one term the reduction IS that term, and `{p}_present` is a
        // better name for it than `offered` would be.
        let is_a_reduction = have.len() > 1;
        if is_a_reduction {
            low.name_value_safe(v, "offered".to_string());
        }
        *offered = Some(v);
        return v;
    }
    let bit = low.emit(Ty::BOOL, Op::RegRead((valid_base + k - 1) as u32));
    low.name_value_safe(bit, format!("v{}", k - 1));
    bit
}

/// Gates every write stage `k` offered on the stage being live and moving.
#[allow(clippy::too_many_arguments)]
fn gate_writes(
    low: &mut Lowerer,
    k: usize,
    owner: &[Option<usize>],
    env: &mut Env,
    shift: Option<ValueId>,
    offered: &mut Option<ValueId>,
    head: &mut Head,
    valid_base: usize,
) {
    let mut gate: Option<ValueId> = None;
    // Once per memory, in the stage that owns it. Gating again in every stage
    // below would AND in their liveness too, and the write would then need
    // every stage of the pipeline occupied before it happened.
    for (ix, _) in owner.iter().enumerate().filter(|(_, o)| **o == Some(k)) {
        // Every port the stage took, not just the first: two writes in a row
        // are two ports and both belong to this stage's item.
        for slot in 0..low.mem_slots(ix) {
            let (we_key, _, _) = Lowerer::mem_port_keys(&low.mems[ix].name, slot);
            let Some(we) = env.get(&we_key).and_then(|b| b.value) else {
                continue;
            };
            // Nothing wrote it, so there is nothing to gate and no reason to
            // put an `and` in the graph.
            if low.const_of(we) == Some(0) {
                continue;
            }
            let g = match gate {
                Some(g) => g,
                None => {
                    let live = stage_live(low, k, offered, head, valid_base);
                    let g = moving(low, live, shift);
                    gate = Some(g);
                    g
                }
            };
            let gated = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: we, rhs: g });
            env.insert(we_key, Binding::constant(gated, Ty::BOOL));
        }
    }
}

/// Names a stage defines.
fn defines(stage: &[&PrecResInnerStmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in stage {
        if let PrecResInnerStmt::VarDecl(d) = stmt {
            out.extend(d.names().iter().map(|n| anumspan_to_str(n).to_string()));
        }
    }
    out
}

/// Names a stage reads.
fn reads(stage: &[&PrecResInnerStmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in stage {
        reads_of(stmt, &mut out);
    }
    out
}

/// The `in` pipes a sequence BLOCKS on, read straight off its source.
///
/// A `@try_rcv` input is not one of them: the head fires without it. That
/// difference decides whether a loop of pipes between sequences can ever carry
/// an item, which is `ir_graph::check_pipe_deadlock`'s question -- and it has
/// to be answerable from the syntax, because a parameter list cannot say it and
/// the answer is wanted before any graph is lowered.
///
/// A body too malformed to lower gives whatever it does say here. That is
/// harmless: the check runs only once nothing else has been reported.
pub fn blocking_inputs_of(decl: &crate::parse::SequenceDecl) -> Vec<String> {
    let stages = split_stages(&decl.body);
    let Some(head) = stages.first() else {
        return Vec::new();
    };
    head.iter()
        .filter_map(|stmt| as_recv(stmt).map(|(_, name)| name))
        .collect()
}

pub fn lower_sequence(
    map: &crate::diag::SourceMap,
    syms: &crate::symbols::Symbols,
    bodies: &std::collections::HashMap<String, &crate::parse::FunctionDecl>,
    decl: &crate::parse::SequenceDecl,
    sink: &mut DiagSink,
) -> Option<crate::ir::Module> {
    // Errors from EARLIER declarations are not this one's failure: the sink
    // is shared by the whole compilation, so `has_errors` would make every
    // declaration after the first bad one return `None` without a reason.
    let errors_before = sink.error_mark();
    let mut low = Lowerer::new(map, syms, bodies);
    let mut env: Env = Env::new();

    // Clock and reset are implicit, as for a process.
    for implicit in ["clk", "rst_n"] {
        let id = low.add_port(implicit.to_string(), crate::ir::PortDir::In, Ty::BOOL);
        let v = low.emit(Ty::BOOL, Op::Port(id));
        low.name_value(v, implicit.to_string());
        env.insert(implicit.to_string(), Binding::constant(v, Ty::BOOL));
    }
    low.declare_pipes(&decl.args, &mut env, sink)?;

    let inputs: Vec<usize> = (0..low.pipes.len()).filter(|i| low.pipes[*i].is_input).collect();
    let outputs: Vec<usize> = (0..low.pipes.len()).filter(|i| !low.pipes[*i].is_input).collect();
    // A pipeline advances on an item arriving and shifts because a sink has
    // room for what leaves. With neither there is no occasion for either, and
    // a block that produces without consuming -- or the reverse -- is a
    // `process` with a `loop`.
    let has_no_source = inputs.is_empty();
    if has_no_source {
        sink.push(
            Diag::error(
                map.span_of(&decl.name),
                "a sequence has no `in` pipe, so nothing ever enters it".to_string(),
            )
            .with_note(
                "a pipeline advances on an item arriving; a block that produces without consuming is a `process` with a `loop`",
            ),
        );
        return None;
    }
    let has_no_sink = outputs.is_empty();
    if has_no_sink {
        sink.push(
            Diag::error(
                map.span_of(&decl.name),
                "a sequence has no `out` pipe, so nothing ever leaves it".to_string(),
            )
            .with_note(
                "the pipeline shifts when its sinks have room, and with no sink there is nothing to shift for; a block that consumes without producing is a `process` with a `loop`",
            ),
        );
        return None;
    }

    // Cuts separate execution stages, not lexical scopes. Resolve the whole
    // statement stream first, then restore the cuts around those identities.
    let globals = env.keys().cloned()
        .chain(low.pipes.iter().map(|p| p.name.clone()))
        .chain(syms.funcs.keys().cloned())
        .chain(syms.structs.keys().cloned())
        .chain(syms.enums.keys().cloned())
        .chain(syms.enums.values().flat_map(|e| e.variants.iter().map(|(n, _)| n.clone())))
        .collect();
    let statements: Vec<_> = decl.body.iter().filter_map(|s| match s {
        PrecSeqInnerStmt::Stmt(s) => Some(s.clone()),
        PrecSeqInnerStmt::SegmentSeparator => None,
    }).collect();
    let scoped = crate::ir_scope::resolve(&statements, globals, sink)?;
    low.synthetic_spans = scoped.origins.clone();
    sink.set_synthetic_spans(scoped.origins);
    let mut resolved = scoped.stmts.into_iter();
    let resolved_body: Vec<_> = decl.body.iter().map(|s| match s {
        PrecSeqInnerStmt::Stmt(_) => PrecSeqInnerStmt::Stmt(
            resolved.next().expect("scope resolution preserves statements")),
        PrecSeqInnerStmt::SegmentSeparator => PrecSeqInnerStmt::SegmentSeparator,
    }).collect();
    let mut stages = split_stages(&resolved_body);
    let n = stages.len();

    // Memories first, so the stages below can be checked against what was
    // declared. A pipeline never writes one, which is what makes it safe to
    // have here at all.
    low.in_pipeline = true;
    let n_mems = declare_memories(&mut low, &stages[0], &mut env, map, sink)?;
    stages[0].drain(..n_mems);

    // Which memories answer a cycle late. A `lutram` does not -- its read is
    // combinational, and several of them in one stage are several read ports,
    // which is what distributed RAM is for.
    let sync_mems: HashMap<String, usize> = low
        .mems
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind != MemKind::LutRam)
        .map(|(ix, m)| (m.name.clone(), ix))
        .collect();
    let sync_mem_of = |n: &str| sync_mems.get(n).copied();

    // The head reads; any stage sends. desc.md:76 -- only the first stage may
    // block on a read, because a read below it would take an item on behalf
    // of a stage holding a different one. A send has no such problem: the
    // item it sends is the one its stage holds, and it leaves when that stage
    // shifts.
    //
    // Keyed by pipe index rather than held in a pair of Options: with several
    // of each, "was there one" is a question per pipe, and the answer has to
    // say WHICH.
    let mut recv_of: Vec<Option<(&VarDeclStmt, bool)>> = vec![None; low.pipes.len()];
    // The stage each output is sent from.
    let mut send_stage: Vec<Option<usize>> = vec![None; low.pipes.len()];
    let mut plain: Vec<Vec<&PrecResInnerStmt>> = Vec::with_capacity(n);

    for (k, stage) in stages.iter().enumerate() {
        let mut keep = Vec::new();
        for stmt in stage {
            // The stage split runs before lowering, so there is no anchor
            // stack to read -- the statement in hand is the same place one
            // would have come from.
            let at = crate::ir::stmt_anchor(stmt)
                .map(|a| low.span_of(&a))
                .unwrap_or_else(crate::driver::nowhere);
            // A blocking `@rcv` and an optional `@try_rcv` differ in what they
            // do to the head's liveness and in nothing else here: both name a
            // pipe, both belong to stage 0, and both may happen once.
            let received = as_recv(stmt)
                .map(|(d, name)| (d, name, true))
                .or_else(|| as_try_recv(stmt).map(|(d, name)| (d, name, false)));
            if let Some((declaration, name, blocking)) = received {
                let is_below_the_head = k != 0;
                if is_below_the_head {
                    if blocking {
                        sink.err_span(
                            at,
                            "only the first stage of a sequence may block on a read",
                        );
                    } else {
                        sink.push(
                            Diag::error(
                                at,
                                "a sequence receives from all of its `in` pipes in its first stage"
                                    .to_string(),
                            )
                            .with_note(
                                "a `@try_rcv` below the head would take an item on behalf of a stage that is holding a different one; move it above the first `|||`",
                            ),
                        );
                    }
                    return None;
                }
                let Some(ix) = low.pipes.iter().position(|p| p.name == name) else {
                    sink.err_span(at, format!("`{}` is not a pipe of this sequence", name));
                    return None;
                };
                if !low.pipes[ix].is_input {
                    sink.err_span(
                        at,
                        format!("`{}` is an `out` pipe; it cannot be received from", name),
                    );
                    return None;
                }
                let is_a_duplicate = recv_of[ix].is_some();
                if is_a_duplicate {
                    sink.err_span(at, format!(
                        "`{}` is received from twice; the head of a sequence takes exactly one item from each of its `in` pipes",
                        name,
                    ));
                    return None;
                }
                recv_of[ix] = Some((declaration, blocking));
            }
            if let Some((name, _)) = as_send(stmt) {
                let Some(ix) = low.pipes.iter().position(|p| p.name == name) else {
                    sink.err_span(at, format!("`{}` is not a pipe of this sequence", name));
                    return None;
                };
                if low.pipes[ix].is_input {
                    sink.err_span(
                        at,
                        format!("`{}` is an `in` pipe; it cannot be sent to", name),
                    );
                    return None;
                }
                // Across the whole body, not per stage: an item that went into
                // a pipe in stage 0 and again in stage 2 would be two items to
                // the consumer, and a consumer of a sequence is owed one each.
                if let Some(first) = send_stage[ix] {
                    let where_first = if first == k {
                        "the first is in this stage too".to_string()
                    } else {
                        format!("the first is in stage {}", first)
                    };
                    sink.push(
                        Diag::error(
                            at,
                            format!(
                                "`{}` is sent to twice; a sequence puts exactly one item into each of its `out` pipes",
                                name,
                            ),
                        )
                        .with_note(where_first),
                    );
                    return None;
                }
                send_stage[ix] = Some(k);
            }
            // Keep reads and sends at their source positions. Their values
            // and side effects must see only the statements preceding them.
            keep.push(*stmt);
        }
        plain.push(keep);
    }

    // EVERY pipe, both ways. A pipe left out is not an oversight the hardware
    // can absorb: an input nothing reads never drains, an output nothing
    // writes never fills, and whatever is on the far side of it waits forever.
    for ix in &inputs {
        let was_received = recv_of[*ix].is_some();
        if was_received {
            continue;
        }
        let name = low.pipes[*ix].name.clone();
        sink.push(
            Diag::error(
                map.span_of(&decl.name),
                format!("`{}` is never received from", name),
            )
            .with_note(format!(
                "a sequence starts by receiving from every one of its `in` pipes, in its first stage, and the pipeline advances only when all of them are offering; add `let x = @rcv({})` there, or `let (x, ok) = @try_rcv({})` to take from it only on the cycles it has something",
                name, name,
            )),
        );
        return None;
    }
    for ix in &outputs {
        let was_sent = send_stage[*ix].is_some();
        if was_sent {
            continue;
        }
        let name = low.pipes[*ix].name.clone();
        sink.push(
            Diag::error(
                map.span_of(&decl.name),
                format!("`{}` is never sent to", name),
            )
            .with_note(format!(
                "a sequence sends to every one of its `out` pipes exactly once, from whichever stage has the value, and that stage shifts only when all of its sinks have room; add `@send({}, ..)` to one",
                name,
            )),
        );
        return None;
    }

    // ---- the shift enables -----------------------------------------------
    // A stage that sends moves when there is room in EVERY slot it sends to --
    // and each of those slots is TWO entries deep, head and skid, exactly as a
    // `process` output is (ir.rs:1820).
    //
    // It used to be one entry, and `shift` was `(!v_last) | dst_ready`. That
    // satisfied channel rule 3 -- `dst_valid` is `v_last`, a register, and
    // never looked at `dst_ready` -- but `src_ready` was `shift`, so it was a
    // wire straight through to `dst_ready`, and `scaler` in
    // examples/pipeline_graph.ddl was one combinational path through three
    // modules. k3g_chan.sv:193 is explicit that `ready` must be "never a
    // function of the opposite side's handshake".
    //
    // The skid entry is what buys that: `ready` becomes "the skid is empty",
    // which is a register, and the item the producer had already committed to
    // has somewhere to go while the head is stalled. The head is the last
    // stage's own register bank -- `v{n-1}` and `out_hold` were already there
    // -- so the second entry costs one slot and adds no latency.
    let mut regs: Vec<Reg> = Vec::new();
    let valid_base = 0usize;

    // THE VALIDITY CHAIN IS ONE SHORTER THAN THE PIPELINE. `v{k}` is the cut
    // below stage k, and there is no cut below the last stage: what leaves it
    // goes into the slots it sends to, whose `wsalt`s already say they hold
    // it, or -- when the last stage sends nothing -- nowhere at all.
    let chain = n - 1;

    // Reserved before the stages are lowered: the shifts read every output's
    // `wsalt`, and the crossing registers discovered down there take their
    // slots after all of them. One slot per input for its `rsalt`, three per
    // output for `e0`, `e1` and `wsalt` -- the layout `ir_comb.rs:204` hands
    // out, walked in the same declaration order so the two files read alike.
    let mut next_slot = chain;
    let mut head = Head {
        ix: Vec::new(),
        blocking: Vec::new(),
        rsalt_q: Vec::new(),
        rsalt_slot: Vec::new(),
        item: Vec::new(),
        present: Vec::new(),
    };
    let mut tail = Tail { ix: Vec::new(), wsalt_q: Vec::new(), base: Vec::new() };
    for (ix, received) in recv_of.iter().enumerate() {
        if !low.pipes[ix].is_input {
            tail.ix.push(ix);
            tail.base.push(next_slot);
            next_slot += 3;
            continue;
        }
        let (_, blocking) = received.expect("every `in` pipe was received from above");
        head.ix.push(ix);
        head.blocking.push(blocking);
        head.rsalt_slot.push(next_slot);
        next_slot += 1;
    }
    for j in 0..head.ix.len() {
        let name = low.pipes[head.ix[j]].name.clone();
        let q = low.emit(SALT, Op::RegRead(head.rsalt_slot[j] as u32));
        low.name_value(q, format!("{}_rsalt_q", name));
        head.rsalt_q.push(q);
        head.present.push(None);
    }
    for j in 0..tail.ix.len() {
        let name = low.pipes[tail.ix[j]].name.clone();
        let q = low.emit(SALT, Op::RegRead((tail.base[j] + 2) as u32));
        low.name_value(q, format!("{}_wsalt_q", name));
        tail.wsalt_q.push(q);
    }

    // A stage moves when there is somewhere for everything leaving it to go.
    // EVERY SINK IT SENDS TO, OR NONE -- the policy `ir_comb.rs:234` states
    // for `@split`, and for its reason: ANDing the sinks' READYS would put each
    // consumer's logic in every other one's timing path, where this asks only
    // whether each has a slot, which is a register on this side.
    let mut full: Vec<ValueId> = Vec::with_capacity(tail.ix.len());
    let mut rooms_at: Vec<Vec<ValueId>> = vec![Vec::new(); n];
    for j in 0..tail.ix.len() {
        let ix = tail.ix[j];
        let name = low.pipes[ix].name.clone();
        let is_full = low.pipe_full(ix, tail.wsalt_q[j]);
        let room = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: is_full });
        low.name_value_safe(room, format!("{}_room", name));
        full.push(is_full);
        let k = send_stage[ix].expect("every `out` pipe was sent to above");
        rooms_at[k].push(room);
    }

    // And the cut below it is free: empty, or emptying because the stage
    // below moves on this same edge. Built from the last stage up, since each
    // stage's answer is the one below it plus its own sinks.
    //
    // `None` is a stage nothing can hold up, because no stage at or below it
    // sends: it moves every cycle, and every register it loads is a plain
    // register. `room_at[k]` and `open_at[k]` are kept apart because the
    // validity bit needs them apart -- see the chain at the bottom.
    let mut room_at: Vec<Option<ValueId>> = vec![None; n];
    let mut open_at: Vec<Option<ValueId>> = vec![None; chain];
    let mut shift_at: Vec<Option<ValueId>> = vec![None; n];
    for k in (0..n).rev() {
        let sends_here = !rooms_at[k].is_empty();
        let room = sends_here.then(|| crate::ir_comb::all_of(&mut low, &rooms_at[k]));
        let has_a_cut_below = k < chain;
        let below = if has_a_cut_below { shift_at[k + 1] } else { None };
        let open = below.map(|below| {
            let occupied = low.emit(Ty::BOOL, Op::RegRead((valid_base + k) as u32));
            let vacant = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: occupied });
            low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: vacant, rhs: below })
        });
        let shift = match (room, open) {
            (Some(r), Some(o)) => {
                // Both halves are signals of their own only here; elsewhere the
                // shift IS the one there is, and takes its name instead.
                if rooms_at[k].len() > 1 {
                    low.name_value_safe(r, per_stage("room", k, n));
                }
                low.name_value_safe(o, per_stage("open", k, n));
                Some(low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: r, rhs: o }))
            }
            (Some(r), None) => Some(r),
            (None, Some(o)) => Some(o),
            (None, None) => None,
        };
        if let Some(s) = shift {
            low.name_value(s, per_stage("shift", k, n));
        }
        room_at[k] = room;
        if has_a_cut_below {
            open_at[k] = open;
        }
        shift_at[k] = shift;
    }

    // ---- stage bodies -----------------------------------------------------
    for j in 0..head.ix.len() {
        let item = low.pipe_item_at(head.ix[j], head.rsalt_q[j]);
        head.item.push(item);
    }

    let defs: Vec<HashSet<String>> = plain
        .iter()
        .map(|st| defines(st))
        .collect();
    let rds: Vec<HashSet<String>> = plain
        .iter()
        .map(|st| reads(st))
        .collect();

    // ---- who owns each memory --------------------------------------------
    //
    // A memory a stage WRITES is storage, and storage has to have an order.
    // The order a pipeline can offer is the item stream, and it can only offer
    // it if one stage does all the touching: at any cycle, stage j and stage k
    // hold different items, so a write in one and a read in the other pairs an
    // item's read with an item's write that is (k - j) places away from it in
    // the stream. An item's own two writes would not even be adjacent -- the
    // next item's write lands between them.
    //
    // With one stage there is no distance to depend on, and the guarantee is
    // the one the source reads as: every item sees every write of every item
    // before it, plus its own, in source order within the stage.
    //
    // A memory NOTHING writes is a table, has no order to keep, and may be
    // read from anywhere.
    let mut written = HashSet::new();
    for stage in &plain {
        for stmt in stage {
            crate::ir_fsm::writes_of(stmt, &mut written);
        }
    }
    let mem_names: Vec<String> = low.mems.iter().map(|m| m.name.clone()).collect();
    let mut mem_owner: Vec<Option<usize>> = vec![None; mem_names.len()];
    for (ix, name) in mem_names.iter().enumerate() {
        if !written.contains(name) {
            continue;
        }
        // `reads_of` reports the base of `m[i] = v` as well as of `m[i]`,
        // which is what makes it the right question here: both are touches.
        let touching: Vec<usize> = (0..n).filter(|k| rds[*k].contains(name)).collect();
        if touching.len() > 1 {
            let stages: Vec<String> = touching.iter().map(|k| format!("stage {}", k)).collect();
            sink.push(
                Diag::error(
                    map.span_of(&decl.name),
                    format!("`{}` is written, and is touched in {}", name, stages.join(" and ")),
                )
                .with_note(
                    "every stage of a pipeline is live at once holding a different item, so a write belongs to one item and a read in another stage belongs to another; put every read and write of a memory that is written in one stage, or make it read-only and it may be read anywhere",
                ),
            );
            return None;
        }
        mem_owner[ix] = touching.first().copied();
    }

    // Emitted the first time something asks what stage 0 is holding, and
    // reused after -- including by the output logic below, which is where it
    // was always computed. A sequence with no memory therefore emits it in
    // exactly the place it used to, and its wire numbering does not move.
    let mut offered: Option<ValueId> = None;
    let mut pending: Vec<(usize, String, Ty, ValueId)> = Vec::new();
    // Every `bram` read, with the port it took, settled onto the memory's
    // read ports once all the stages are lowered and the validity chain the
    // enables refer to is known.
    let mut issued: Vec<IssuedRead> = Vec::new();

    // Per output: what its `@send` produced, captured at the send's own source
    // position so an assignment sitting between two sends is seen by the later
    // one and not by the earlier.
    let mut sent: Vec<Option<ValueId>> = vec![None; low.pipes.len()];

    for (k, stage) in plain.iter().enumerate() {
        let assertion_start = low.asserts.len();
        for stmt in stage {
            let received = as_recv(stmt)
                .map(|(d, name)| (d, name, true))
                .or_else(|| as_try_recv(stmt).map(|(d, name)| (d, name, false)));
            if let Some((declaration, name, blocking)) = received {
                let ix = low
                    .pipes
                    .iter()
                    .position(|p| p.name == name)
                    .expect("the head receives were resolved above");
                let j = head
                    .ix
                    .iter()
                    .position(|h| *h == ix)
                    .expect("an `in` pipe has a place in the head");
                let ty = low.pipes[ix].ty.clone();
                let item = head.item[j];
                if blocking {
                    check_read_type(&low, declaration, &ty, sink)?;
                    let binding = if declaration.is_mutable {
                        Binding::variable(item, ty)
                    } else {
                        Binding::constant(item, ty)
                    };
                    env.insert(anumspan_to_str(&declaration.head_name()).to_string(), binding);
                    continue;
                }
                // The pair: the entry this side is owed, and whether there was
                // one. When there was not, the item is whatever the buffer
                // still holds -- the same bargain `@try_rcv` makes in a
                // process (ir.rs:2836), and what `ok` is there to answer.
                let present = head_present(&mut low, &mut head, j);
                let names = declaration.names();
                env.insert(
                    anumspan_to_str(&names[0]).to_string(),
                    Binding::constant(item, ty),
                );
                env.insert(
                    anumspan_to_str(&names[1]).to_string(),
                    Binding::constant(present, Ty::BOOL),
                );
                continue;
            }
            if let Some((send_name, expr)) = as_send(stmt) {
                let out_ix = low
                    .pipes
                    .iter()
                    .position(|p| p.name == send_name)
                    .expect("the tail sends were resolved above");
                let out_ty = low.pipes[out_ix].ty.clone();
                // Capture the payload now, before later assignments. Nested
                // port sends and assertions are part of this stage as well,
                // before its transfers are collected and execution-gated.
                let at = crate::ir::stmt_anchor(stmt)
                    .map(|a| low.span_of(&a))
                    .unwrap_or_else(crate::driver::nowhere);
                let depth = low.push_anchor(at);
                let value = crate::ir::lower_expr_expecting(
                    &mut low, &expr, Some(&out_ty), &env, sink,
                );
                low.pop_anchor(depth);
                let mut value = value?;
                if low.ty_of(value) != out_ty
                    && let Some(coerced) = low.coerce_const_pub(value, &out_ty) {
                        value = coerced;
                    }
                let have = low.ty_of(value);
                if have != out_ty {
                    sink.err_span(at, format!(
                        "`{}` carries `{}` but `{}` was sent",
                        low.pipes[out_ix].name, out_ty.display(), have.display(),
                    ));
                    return None;
                }
                sent[out_ix] = Some(value);
                continue;
            }
            // `let x = mem[i]` on a `bram`, in its own place in the stage.
            //
            // The address goes out here and the value is bound past the cut
            // below, so this is not `lower_stmt`'s to do. Taken at this
            // position rather than hoisted, because `pending_write` reads the
            // write port as it stands NOW -- which is exactly the writes above
            // this line and none of the ones below it.
            if let Some(r) = as_sync_read(stmt, &sync_mem_of) {
                let PrecResInnerStmt::VarDecl(declaration) = stmt else {
                    unreachable!("a synchronous read is a declaration")
                };
                check_read_type(&low, declaration, &low.mems[r.mem_ix].elem, sink)?;
                let at = crate::ir::stmt_anchor(stmt)
                    .map(|a| low.span_of(&a))
                    .unwrap_or_else(crate::driver::nowhere);
                // The value arrives at the cut below. With no cut below there
                // is nowhere for the cycle to be spent.
                if k + 1 == n {
                    sink.push(
                        Diag::error(
                            at,
                            format!(
                                "`{}` is read in the last stage, and the value arrives a cycle later",
                                low.mems[r.mem_ix].name
                            ),
                        )
                        .with_note(
                            "put a `|||` after this line; the read is presented in the stage that names the address and lands in the one below",
                        ),
                    );
                    return None;
                }
                // The rule the whole feature rests on: the memory answers at
                // the clock edge that IS the cut, so in the stage that asked,
                // the name still means what `<mem>_q` held for the item ahead.
                if rds[k].contains(&r.bind) {
                    sink.push(
                        Diag::error(
                            at,
                            format!("`{}` is used in the stage that asked for it", r.bind),
                        )
                        .with_note(
                            "a `bram` answers a cycle after the address, and that cycle is the `|||` below; move what reads it past the cut",
                        ),
                    );
                    return None;
                }
                let addr_width = low.mems[r.mem_ix].addr_width;
                let raw = crate::ir::lower_expr(&mut low, r.addr, &env, sink)?;
                let addr = low.fit_address(raw, addr_width, &decl.name, sink)?;
                let forward = low.pending_write(r.mem_ix, addr, &env);
                let in_bounds = low.address_in_bounds(r.mem_ix, addr);
                // An UNCONDITIONAL write to this address, above this line in
                // this stage, is the answer. Asking the array as well would be
                // a read port and an output register for a value already in
                // hand, and a mux that can only choose one way.
                let settled = forward.is_some_and(|(hit, _)| low.const_of(hit) == Some(1));
                let port = if settled {
                    None
                } else {
                    // Reserved now so ports are numbered in source order; the
                    // enable needs the stage's liveness, settled further down.
                    let p = low.mems[r.mem_ix].read.len();
                    low.mems[r.mem_ix].read.push(ReadPort { addr, en: addr });
                    Some(p)
                };
                issued.push(IssuedRead {
                    mem_ix: r.mem_ix,
                    port,
                    bind: r.bind,
                    stage: k,
                    forward,
                    in_bounds,
                });
                continue;
            }
            crate::ir::lower_stmt_pub(&mut low, stmt, &mut env, sink)?;
        }
        if low.asserts.len() != assertion_start {
            let live = stage_live(&mut low, k, &mut offered, &mut head, valid_base);
            let executing = moving(&mut low, live, shift_at[k]);
            gate_assertions(&mut low, assertion_start, executing);
        }
        // A write belongs to the item in this stage, on the cycle the stage
        // moves it on. Ungated, a stall would rewrite every cycle it waited and
        // a bubble would write whatever the wires happened to hold.
        gate_writes(&mut low, k, &mem_owner, &mut env, shift_at[k], &mut offered, &mut head, valid_base);
        // What this stage offered, and then cleared so the next stage's answer
        // is its own. Without the reset a send in stage 0 would read as a send
        // in every stage after it.
        low.transfer_paths.clear();
        let is_last = k + 1 == n;
        if is_last {
            break;
        }
        // Everything defined so far and read later crosses this cut, and is
        // registered here -- once per boundary, so it stays in step with its
        // item. Loaded when THIS stage moves: the cut can also open under a
        // stage that is held, and then what it takes is a bubble, whose
        // values nothing reads.
        let mut crossing: Vec<String> = Vec::new();
        for name in env.keys() {
            let defined_by_now = defs.iter().take(k + 1).any(|d| d.contains(name));
            let read_later = rds.iter().skip(k + 1).any(|r| r.contains(name));
            if defined_by_now && read_later {
                crossing.push(name.clone());
            }
        }
        crossing.sort();
        for name in crossing {
            let b = match env.get(&name) {
                Some(b) => b.clone(),
                None => continue,
            };
            let v = match b.value {
                Some(v) => v,
                None => continue,
            };
            let slot = next_slot;
            next_slot += 1;
            let held = low.emit(b.ty.clone(), Op::RegRead(slot as u32));
            low.name_value(held, format!("{}_s{}", name, k + 1));
            let next = load_on(&mut low, shift_at[k], b.ty.clone(), v, held);
            pending.push((slot, format!("{}_s{}", name, k + 1), b.ty.clone(), next));
            let stays_mutable = b.is_mutable;
            env.insert(
                name,
                Binding { value: Some(held), ty: b.ty, is_output: false, is_mutable: stays_mutable },
            );
        }
        // AFTER the crossing registers, and that is the point: the name is not
        // in the environment while this cut is being decided, so nothing
        // registers it here. `<mem>_q` is already the register for this
        // boundary -- a second one would pair the value with the item behind.
        //
        // From the next cut on it is an ordinary name: read two stages down
        // and it crosses like anything else, which it has to, because `_q`
        // holds this item's answer for exactly one shift.
        for r in issued.iter().filter(|r| r.stage == k) {
            let elem = low.mems[r.mem_ix].elem.clone();
            let mem = low.mems[r.mem_ix].name.clone();
            // The write this read collided with happened on the SAME edge that
            // filled `_q`, so `_q` holds the contents from before it. Which
            // addresses collided was decided back in the stage that asked,
            // where both existed; only the answer crosses, one bit and the
            // data.
            let value = match (r.port, r.forward) {
                (Some(p), None) => {
                    low.emit(elem.clone(), Op::MemReadReg { mem: r.mem_ix as u32, port: p as u32 })
                }
                // Answered by the write above it; the array was never asked.
                (None, Some((_, wdata))) => cross(
                    &mut low,
                    &mut pending,
                    &mut next_slot,
                    format!("{}_s{}", r.bind, k + 1),
                    elem.clone(),
                    wdata,
                    shift_at[k],
                ),
                (Some(p), Some((hit, wdata))) => {
                    let q = low.emit(
                        elem.clone(),
                        Op::MemReadReg { mem: r.mem_ix as u32, port: p as u32 },
                    );
                    let hit_q = cross(
                        &mut low,
                        &mut pending,
                        &mut next_slot,
                        format!("{}_fwd{}_s{}", mem, p, k + 1),
                        Ty::BOOL,
                        hit,
                        shift_at[k],
                    );
                    let data_q = cross(
                        &mut low,
                        &mut pending,
                        &mut next_slot,
                        format!("{}_wdata{}_s{}", mem, p, k + 1),
                        elem.clone(),
                        wdata,
                        shift_at[k],
                    );
                    low.emit(elem.clone(), Op::Mux { cond: hit_q, then_val: data_q, else_val: q })
                }
                // `port` is `None` only when the forward is unconditional.
                (None, None) => unreachable!("a read with no port was answered by a write"),
            };
            // A depth the address can overrun answers zero outside it, as a
            // packed array does. Decided back in the stage that had the
            // address, and crossed like the forwarding decision beside it.
            let value = match r.in_bounds {
                None => value,
                Some(ok) => {
                    let ok_q = cross(
                        &mut low,
                        &mut pending,
                        &mut next_slot,
                        format!("{}_inrange{}_s{}", mem, r.port.unwrap_or(0), k + 1),
                        Ty::BOOL,
                        ok,
                        shift_at[k],
                    );
                    low.guarded_read(Some(ok_q), value, &elem)
                }
            };
            env.insert(r.bind.clone(), Binding::constant(value, elem));
        }
    }

    // ---- the output -------------------------------------------------------
    //
    // A sending stage's result is pushed into an entry rather than registered
    // into a head, which is the same flop count arranged differently: two
    // entries and a salt per sink, instead of head, skid, and two occupancy
    // bits.
    let offered = stage_live(&mut low, 0, &mut offered, &mut head, valid_base);

    // ---- the read ports ---------------------------------------------------
    //
    // Tied to the asking stage's shift, which is what keeps `<mem>_q` in step
    // with the item it belongs to: a stalled stage is holding, and a memory
    // that kept reading through a stall would answer the item behind by the
    // time the stall lifted. And gated on the stage having something in it, so
    // a bubble does not spend a read -- the value would be discarded, but a
    // block RAM read is not free and an X in simulation is worth not producing.
    let mut cached = Some(offered);
    for r in &issued {
        let Some(p) = r.port else { continue };
        let held = stage_live(&mut low, r.stage, &mut cached, &mut head, valid_base);
        let rd_en = moving(&mut low, held, shift_at[r.stage]);
        let label = if low.mems[r.mem_ix].read.len() > 1 {
            format!("{}_re{}", low.mems[r.mem_ix].name, p)
        } else {
            format!("{}_re", low.mems[r.mem_ix].name)
        };
        low.name_value_safe(rd_en, label);
        let addr = low.mems[r.mem_ix].read[p].addr;
        low.mems[r.mem_ix].read[p] = ReadPort { addr, en: rd_en };
    }
    // What the source left on each write port, gated above by the stage that
    // owns it. Nothing else touches these, so once is enough.
    low.settle_memories(&env);

    // Nothing leaves a stage on a cycle it does not shift. ONE `push` per
    // sending stage, for every sink that stage sends to: they were all given
    // room by its shift and they all take the item it holds, so naming it after
    // any one of them would be a lie about the rest. With one sending stage it
    // is `push`, whichever stage that is; with several, `push{k}` -- the
    // `<mem>_re` rule again.
    let sending: Vec<usize> = (0..n).filter(|k| room_at[*k].is_some()).collect();
    let mut push_at: Vec<Option<ValueId>> = vec![None; n];
    for &k in &sending {
        // Stage 0 holds what the head gathered; stage k what the cut above it
        // passed on.
        let live = if k == 0 {
            offered
        } else {
            low.emit(Ty::BOOL, Op::RegRead((valid_base + k - 1) as u32))
        };
        let shift = shift_at[k].expect("a stage that sends has a shift");
        let push = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: shift, rhs: live });
        let name = if sending.len() == 1 { "push".to_string() } else { format!("push{}", k) };
        low.name_value(push, name);
        push_at[k] = Some(push);
    }

    let mut drivers: Vec<(PortId, ValueId)> = Vec::new();
    // Each output's two entries and its salt, in the shape `ir_comb` gives a
    // combinator's -- and named the same way, after the pipe, because with
    // several of them `out_e0` could only ever name one.
    for (j, is_full) in full.iter().enumerate() {
        let ix = tail.ix[j];
        let name = low.pipes[ix].name.clone();
        let ty = low.pipes[ix].ty.clone();
        let base = tail.base[j];
        let e0 = low.emit(ty.clone(), Op::RegRead(base as u32));
        low.name_value(e0, format!("{}_e0", name));
        let e1 = low.emit(ty.clone(), Op::RegRead((base + 1) as u32));
        low.name_value(e1, format!("{}_e1", name));
        let widx = low.salt_idx(tail.wsalt_q[j], format!("{}_widx", name));
        let side = crate::ir_comb::OutSide {
            e0,
            e1,
            wsalt_q: tail.wsalt_q[j],
            widx,
            full: *is_full,
            base,
        };
        let item = sent[ix].expect("every `out` pipe was sent to");
        let stage = send_stage[ix].expect("every `out` pipe was sent to");
        let push = push_at[stage].expect("a sending stage has a push");
        crate::ir_comb::push_out(&mut low, ix, &side, item, push, &mut pending, &mut drivers);
    }

    // The head is taken when something is offered and stage 0 is moving. ONE
    // `take`, because the head is a join -- every blocking input gives up its
    // item on the same cycle, or none of them does.
    //
    // Stage 0 always has a shift: every sequence sends to something, and a
    // send at or below a stage is what gives it one.
    let head_shift = shift_at[0].expect("a sequence sends, so its head has a shift");
    let take = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: offered, rhs: head_shift });
    low.name_value(take, "take".to_string());
    for j in 0..head.ix.len() {
        let ix = head.ix[j];
        let name = low.pipes[ix].name.clone();
        // An OPTIONAL input gives up an item only on the cycles it had one.
        // `offered` never waited for it, so `take` on its own would step this
        // `rsalt` past an entry that was never there.
        let advance = if head.blocking[j] {
            take
        } else {
            let present = head_present(&mut low, &mut head, j);
            let v = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: take, rhs: present });
            low.name_value_safe(v, format!("{}_take", name));
            v
        };
        let ridx = low.pipes[ix].idx.expect("the input's index was emitted with its item");
        let next = low.salt_next(head.rsalt_q[j], ridx, advance);
        pending.push((head.rsalt_slot[j], format!("{}_rsalt_q", name), SALT, next));
        drivers.push((low.pipes[ix].rsalt_port, head.rsalt_q[j]));
    }

    // ---- the validity chain ----------------------------------------------
    // `chain` bits, not `n`: nothing is below the last stage.
    //
    // `v{k}` changes when its cut OPENS, which is not the same as stage k
    // moving. A stage whose sinks are full holds its item while the stage below
    // leaves, and the cut then has to take a bubble -- keeping its old `1`
    // would hand the item below a second life. So what enters is the item only
    // if stage k is live AND its sends went through.
    let mut valid_regs: Vec<Reg> = Vec::with_capacity(chain);
    for k in 0..chain {
        let cur = low.emit(Ty::BOOL, Op::RegRead((valid_base + k) as u32));
        low.name_value(cur, format!("v{}", k));
        let live = if k == 0 {
            offered
        } else {
            low.emit(Ty::BOOL, Op::RegRead((valid_base + k - 1) as u32))
        };
        let leaving = match room_at[k] {
            Some(room) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: live, rhs: room }),
            None => live,
        };
        let next = load_on(&mut low, open_at[k], Ty::BOOL, leaving, cur);
        valid_regs.push(Reg { name: format!("v{}", k), ty: Ty::BOOL, reset: 0, next });
    }
    // Every slot reserved above has exactly one next value below. The layout is
    // no longer four fixed offsets off `chain`, and a mis-sized one would show
    // up not as a crash but as a register quietly driven by the wrong
    // expression -- so it is worth saying out loud where the two meet.
    debug_assert_eq!(
        valid_regs.len() + pending.len(),
        next_slot,
        "every reserved slot has a next value",
    );
    regs.extend(valid_regs);
    // Slots were handed out assuming validity bits come first, so the pipeline
    // registers must follow in the order they were allocated.
    pending.sort_by_key(|(slot, _, _, _)| *slot);
    for (_, name, ty, next) in pending {
        regs.push(Reg { name, ty, reset: 0, next });
    }

    // A `port out` would be driven from the stage that sent to it, on
    // `stage_live(k) & shift` -- the expression the assertion gate above
    // already builds. There is nothing to do here yet: a plain `out` parameter
    // is refused by `classify_param` (ir.rs:639) long before it reaches this
    // file, and results leave a sequence through a `buffer out` pipe.

    if sink.errored_since(errors_before) {
        return None;
    }
    let asserts = std::mem::take(&mut low.asserts);
    let params = std::mem::take(&mut low.params);
    let mems = std::mem::take(&mut low.mems);
    let calls: Vec<String> = low.calls.iter().cloned().collect();
    let (values, ports) = low.take_values();
    Some(crate::ir::Module {
        calls,
        params,
        asserts,
        mems,
        name: anumspan_to_str(&decl.name).to_string(),
        ports,
        values,
        drivers,
        regs,
        nets: Vec::new(),
        instances: Vec::new(),
    })
}

fn gate_assertions(low: &mut Lowerer, start: usize, executing: ValueId) {
    let inactive = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: executing });
    for ix in start..low.asserts.len() {
        let cond = low.asserts[ix].cond;
        low.asserts[ix].cond = low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: inactive, rhs: cond });
    }
}

