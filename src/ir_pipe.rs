// `sequence` lowered to a pipeline.
//
// `|||` cuts the body into stages. Everything inside a stage is combinational;
// every cut becomes a register bank, and a validity bit rides alongside the
// data -- desc.md:80's "implicit is_valid condition at each stage".
//
// The whole pipeline shifts together, gated on the sink having a slot
// (desc.md:77: "pipeline fires when all buffer sinks have slots"). Latency is
// one cycle per stage; throughput is one item per cycle while the sink keeps
// up.
//
// A value defined in one stage and read two stages later needs TWO registers,
// not one. It has to arrive alongside the item it belongs to, and that item is
// two cycles further down the pipe -- registering it once would pair stage 2's
// item with stage 0's value. So a crossing value is registered at every
// boundary it spans, which is a shift register of exactly the right depth.
//
// Channel rule 3 holds as it does everywhere else: the output's `valid` is the
// last validity bit, which is a register.
//
// A `bram` READ is the one thing here that already had a register and did not
// need one of ours. It answers a cycle after the address, and a cycle is
// exactly what a stage cut is, so the memory's own output register IS the
// pipeline register at that boundary: the address goes out in stage k and the
// name means `<mem>_q` from stage k+1 on. What that costs to say is a read
// enable tied to the shift, and a rule that the value is not there yet in the
// stage that asked for it.
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

/// `@send(p, v)`, the tail stage's output.
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
/// a `RegRead` off it, and a next-value that takes `v` when the pipeline moves
/// and holds otherwise. Pulled out because a forwarded write needs two of these
/// and is not a name in the environment, so the loop that walks `env` cannot
/// make them.
#[allow(clippy::too_many_arguments)]
fn cross(
    low: &mut Lowerer,
    pending: &mut Vec<(String, Ty, ValueId, usize)>,
    next_slot: &mut usize,
    name: String,
    ty: Ty,
    v: ValueId,
    en: ValueId,
) -> ValueId {
    let slot = *next_slot;
    *next_slot += 1;
    let held = low.emit(ty.clone(), Op::RegRead(slot as u32));
    low.name_value(held, name.clone());
    let next = low.emit(ty.clone(), Op::Mux { cond: en, then_val: v, else_val: held });
    pending.push((name, ty, next, slot));
    held
}

/// Whether stage `k` is holding an item this cycle.
///
/// Stage 0 has one when the input is offering; stage k has one when the cut
/// behind it passed one on. `offered` is emitted at most once and only for a
/// sequence that asks -- a pipeline with no memory in it computes the same
/// thing further down, and emitting it early would renumber every wire in
/// every module that never needed this.
fn stage_live(
    low: &mut Lowerer,
    k: usize,
    offered: &mut Option<ValueId>,
    in_ix: usize,
    in_rsalt_q: ValueId,
    valid_base: usize,
) -> ValueId {
    if k == 0 {
        if let Some(v) = offered {
            return *v;
        }
        let empty = low.pipe_empty(in_ix, in_rsalt_q);
        let v = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: empty });
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
    en: ValueId,
    offered: &mut Option<ValueId>,
    in_ix: usize,
    in_rsalt_q: ValueId,
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
                    let live = stage_live(low, k, offered, in_ix, in_rsalt_q, valid_base);
                    let g = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: live, rhs: en });
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

pub fn lower_sequence(
    map: &crate::diag::SourceMap,
    syms: &crate::symbols::Symbols,
    bodies: &std::collections::HashMap<String, &crate::parse::FunctionDecl>,
    decl: &crate::parse::SequenceDecl,
    sink: &mut DiagSink,
) -> Option<crate::ir::Module> {
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
    let shape_is_supported = inputs.len() == 1 && outputs.len() == 1;
    if !shape_is_supported {
        sink.err_span(
            map.span_of(&decl.name),
            "a sequence takes exactly one `in` pipe and one `out` pipe for now",
        );
        return None;
    }
    let in_ix = inputs[0];
    let out_ix = outputs[0];

    // Cuts separate execution stages, not lexical scopes. Resolve the whole
    // statement stream first, then restore the cuts around those identities.
    let globals = env.keys().cloned()
        .chain(low.pipes.iter().map(|p| p.name.clone()))
        .chain(low.port_ins.iter().map(|p| p.name.clone()))
        .chain(low.port_outs.iter().map(|p| p.name.clone()))
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

    // The head reads; the tail sends. desc.md:76 -- only the first stage may
    // block on a read, and the send belongs with the result.
    let mut head_recv: Option<(&VarDeclStmt, String)> = None;
    let mut tail_send: Option<(String, PrecResExpr)> = None;
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
            if let Some(r) = as_recv(stmt) {
                if k != 0 {
                    sink.err_span(
                        at,
                        "only the first stage of a sequence may block on a read",
                    );
                    return None;
                }
                if head_recv.is_some() {
                    sink.err_span(at, "a sequence may receive from its input buffer only once; duplicate `@rcv`");
                    return None;
                }
                head_recv = Some(r);
            }
            if let Some(s) = as_send(stmt) {
                if k + 1 != n {
                    sink.err_span(at, "a sequence sends from its last stage");
                    return None;
                }
                if tail_send.is_some() {
                    sink.err_span(at, "a sequence may send to its output buffer only once; duplicate `@send`");
                    return None;
                }
                tail_send = Some(s);
            }
            // Keep reads and sends at their source positions. Their values
            // and side effects must see only the statements preceding them.
            keep.push(*stmt);
        }
        plain.push(keep);
    }

    let (_, recv_pipe) = match head_recv {
        Some(r) => r,
        None => {
            sink.err_span(
                map.span_of(&decl.name),
                "a sequence starts by receiving from its `in` pipe",
            );
            return None;
        }
    };
    if recv_pipe != low.pipes[in_ix].name {
        sink.err_span(
            map.span_of(&decl.name),
            format!("`{}` is not this sequence's `in` pipe", recv_pipe),
        );
        return None;
    }
    let (send_pipe, _) = match tail_send {
        Some(s) => s,
        None => {
            sink.err_span(
                map.span_of(&decl.name),
                "a sequence ends by sending to its `out` pipe",
            );
            return None;
        }
    };
    if send_pipe != low.pipes[out_ix].name {
        sink.err_span(
            map.span_of(&decl.name),
            format!("`{}` is not this sequence's `out` pipe", send_pipe),
        );
        return None;
    }

    // ---- the shift enable -------------------------------------------------
    // The pipeline moves when there is room in the output slot -- and that
    // slot is TWO entries deep, head and skid, exactly as a `process` output
    // is (ir.rs:1820).
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

    // THE VALIDITY CHAIN IS ONE SHORTER THAN THE PIPELINE, because the last
    // stage's validity bit and the output slot's occupancy were always the
    // same fact told twice. `wsalt` is that fact now. Keeping `v{n-1}` beside
    // it would be two representations of occupancy that can disagree.
    let chain = n - 1;

    // Reserved before the stages are lowered: `shift` reads `out_wsalt_q`, and
    // the crossing registers discovered down there take their slots after it.
    let in_rsalt_slot = chain;
    let e0_slot = chain + 1;
    let e1_slot = chain + 2;
    let wsalt_slot = chain + 3;
    let mut next_slot = chain + 4;

    let in_rsalt_q = low.emit(SALT, Op::RegRead(in_rsalt_slot as u32));
    low.name_value(in_rsalt_q, "src_rsalt_q".to_string());
    let wsalt_q = low.emit(SALT, Op::RegRead(wsalt_slot as u32));
    low.name_value(wsalt_q, "out_wsalt_q".to_string());

    // The pipeline moves when there is somewhere for what leaves it to go.
    let out_full = low.pipe_full(out_ix, wsalt_q);
    let en = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: out_full });
    low.name_value(en, "shift".to_string());

    // ---- stage bodies -----------------------------------------------------
    let in_data = low.pipe_item_at(in_ix, in_rsalt_q);

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
    let mut pending: Vec<(String, Ty, ValueId, usize)> = Vec::new();
    // Every `bram` read, with the port it took, settled onto the memory's
    // read ports once all the stages are lowered and the validity chain the
    // enables refer to is known.
    let mut issued: Vec<IssuedRead> = Vec::new();

    // A `port out` in a pipeline belongs to the STAGE that assigns it.
    //
    // A process drives one from the state that assigned it, gated on that
    // state firing. There are no states here -- every stage is live at once,
    // each holding a different item -- so what stands in for "the state fired"
    // is "the stage has a valid item and the pipeline is moving". Which stage
    // that is has to be recorded as the stages are lowered, because afterwards
    // the environment holds one value and not which cut it came from.
    //
    // Per port: the stage that sent to it, what it offered and on which
    // branch.
    let mut port_sends: Vec<Vec<(usize, ValueId, Option<ValueId>)>> =
        vec![Vec::new(); low.port_outs.len()];
    let out_ty = low.pipes[out_ix].ty.clone();
    let mut sent = None;

    for (k, stage) in plain.iter().enumerate() {
        let assertion_start = low.asserts.len();
        for stmt in stage {
            if let Some((declaration, _)) = as_recv(stmt) {
                let ty = low.pipes[in_ix].ty.clone();
                check_read_type(&low, declaration, &ty, sink)?;
                let binding = if declaration.is_mutable {
                    Binding::variable(in_data, ty)
                } else {
                    Binding::constant(in_data, ty)
                };
                env.insert(anumspan_to_str(&declaration.head_name()).to_string(), binding);
                continue;
            }
            if let Some((_, expr)) = as_send(stmt) {
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
                sent = Some(value);
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
                });
                continue;
            }
            crate::ir::lower_stmt_pub(&mut low, stmt, &mut env, sink)?;
        }
        if low.asserts.len() != assertion_start {
            let live = stage_live(&mut low, k, &mut offered, in_ix, in_rsalt_q, valid_base);
            let executing = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: live, rhs: en });
            gate_assertions(&mut low, assertion_start, executing);
        }
        // A write belongs to the item in this stage, on the cycle the pipeline
        // moves it on. Ungated, a stall would rewrite every cycle it waited and
        // a bubble would write whatever the wires happened to hold.
        gate_writes(
            &mut low, k, &mem_owner, &mut env, en, &mut offered, in_ix, in_rsalt_q, valid_base,
        );
        // What this stage offered, and then cleared so the next stage's answer
        // is its own. Without the reset a send in stage 0 would read as a send
        // in every stage after it.
        for (port, sends) in low.port_outs.iter_mut().zip(port_sends.iter_mut()) {
            if let Some(v) = port.sent {
                sends.push((k, v, port.send_guard));
            }
            port.sent = None;
            port.send_guard = None;
        }
        low.transfer_paths.clear();
        let is_last = k + 1 == n;
        if is_last {
            break;
        }
        // Everything defined so far and read later crosses this cut, and is
        // registered here -- once per boundary, so it stays in step with its
        // item.
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
            let next = low.emit(
                b.ty.clone(),
                Op::Mux { cond: en, then_val: v, else_val: held },
            );
            pending.push((format!("{}_s{}", name, k + 1), b.ty.clone(), next, slot));
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
                    en,
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
                        en,
                    );
                    let data_q = cross(
                        &mut low,
                        &mut pending,
                        &mut next_slot,
                        format!("{}_wdata{}_s{}", mem, p, k + 1),
                        elem.clone(),
                        wdata,
                        en,
                    );
                    low.emit(elem.clone(), Op::Mux { cond: hit_q, then_val: data_q, else_val: q })
                }
                // `port` is `None` only when the forward is unconditional.
                (None, None) => unreachable!("a read with no port was answered by a write"),
            };
            env.insert(r.bind.clone(), Binding::constant(value, elem));
        }
    }

    // ---- the output -------------------------------------------------------
    let sent = sent.expect("the validated tail send was lowered in its stage");

    // The last stage's result is pushed into an entry rather than registered
    // into a head, which is the same flop count arranged differently: two
    // entries and a salt, instead of head, skid, and two occupancy bits.
    let offered = stage_live(&mut low, 0, &mut offered, in_ix, in_rsalt_q, valid_base);

    // ---- the read ports ---------------------------------------------------
    //
    // Tied to the shift, which is what keeps `<mem>_q` in step with the item
    // it belongs to: a stalled pipeline is holding, and a memory that kept
    // reading through a stall would answer the item behind by the time the
    // stall lifted. And gated on the stage having something in it, so a bubble
    // does not spend a read -- the value would be discarded, but a block RAM
    // read is not free and an X in simulation is worth not producing.
    let mut cached = Some(offered);
    for r in &issued {
        let Some(p) = r.port else { continue };
        let held = stage_live(&mut low, r.stage, &mut cached, in_ix, in_rsalt_q, valid_base);
        let rd_en = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: held, rhs: en });
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

    // What arrives at the output this cycle comes from the stage behind it,
    // which for a one-stage sequence is the input itself.
    let feed = if n == 1 {
        offered
    } else {
        low.emit(Ty::BOOL, Op::RegRead((valid_base + n - 2) as u32))
    };
    // Nothing leaves the last stage on a cycle the pipeline does not shift.
    let push = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: en, rhs: feed });
    low.name_value(push, "out_push".to_string());

    let e0 = low.emit(out_ty.clone(), Op::RegRead(e0_slot as u32));
    low.name_value(e0, "out_e0".to_string());
    let e1 = low.emit(out_ty.clone(), Op::RegRead(e1_slot as u32));
    low.name_value(e1, "out_e1".to_string());
    let widx = low.salt_idx(wsalt_q, "out_widx".to_string());
    let not_widx = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: widx });
    let to_e0 = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: push, rhs: not_widx });
    let to_e1 = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: push, rhs: widx });
    let e0_next = low.emit(out_ty.clone(), Op::Mux { cond: to_e0, then_val: sent, else_val: e0 });
    let e1_next = low.emit(out_ty.clone(), Op::Mux { cond: to_e1, then_val: sent, else_val: e1 });
    let wsalt_next = low.salt_next(wsalt_q, widx, push);

    // The input is taken on the same predicate it always was: something is
    // offered and the pipeline is moving.
    let take = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: offered, rhs: en });
    low.name_value(take, "src_take".to_string());
    let in_ridx = low.pipes[in_ix].idx.expect("the input's index was emitted with its item");
    let in_rsalt_next = low.salt_next(in_rsalt_q, in_ridx, take);

    pending.push(("src_rsalt_q".to_string(), SALT, in_rsalt_next, in_rsalt_slot));
    pending.push(("out_e0".to_string(), out_ty.clone(), e0_next, e0_slot));
    pending.push(("out_e1".to_string(), out_ty.clone(), e1_next, e1_slot));
    pending.push(("out_wsalt_q".to_string(), SALT, wsalt_next, wsalt_slot));

    // ---- the validity chain ----------------------------------------------
    // `chain` bits, not `n`: the last stage's occupancy is `wsalt`.
    let mut valid_regs: Vec<Reg> = Vec::with_capacity(chain);
    for k in 0..chain {
        let cur = low.emit(Ty::BOOL, Op::RegRead((valid_base + k) as u32));
        low.name_value(cur, format!("v{}", k));
        let feed = if k == 0 {
            offered
        } else {
            low.emit(Ty::BOOL, Op::RegRead((valid_base + k - 1) as u32))
        };
        let next = low.emit(Ty::BOOL, Op::Mux { cond: en, then_val: feed, else_val: cur });
        valid_regs.push(Reg { name: format!("v{}", k), ty: Ty::BOOL, reset: 0, next });
    }
    regs.extend(valid_regs);
    // Slots were handed out assuming validity bits come first, so the pipeline
    // registers must follow in the order they were allocated.
    pending.sort_by_key(|(_, _, _, slot)| *slot);
    for (name, ty, next, _) in pending {
        regs.push(Reg { name, ty, reset: 0, next });
    }

    let pair = low.pack_entries(e0, e1, &out_ty);
    let mut drivers: Vec<(PortId, ValueId)> = vec![
        (low.pipes[in_ix].rsalt_port, in_rsalt_q),
        (low.pipes[out_ix].wsalt_port, wsalt_q),
        (low.pipes[out_ix].data_port, pair),
    ];

    // ---- `port out`, driven from the stage that sent to it ----------------
    //
    // Stage 0 holds an item when the input is offering one; stage k holds one
    // when `v{k-1}` says the cut behind it passed one on. The port's enable is
    // that, ANDed with the branch the send was written on and with the shift --
    // because a stalled pipeline is not producing anything, it is holding.
    for (port, sends) in low.port_outs.clone().into_iter().zip(port_sends.clone()) {
        // ONE stage, not several. Every stage is live at once and each holds a
        // different item, so two stages driving one port is two answers for
        // one wire -- and unlike a process, where only one state is current,
        // there is nothing to choose between them.
        if sends.len() > 1 {
            let stages: Vec<String> =
                sends.iter().map(|(k, _, _)| format!("stage {}", k)).collect();
            sink.push(
                crate::diag::Diag::error(
                    map.span_of(&decl.name),
                    format!("`{}` is sent to in {}", port.name, stages.join(" and ")),
                )
                .with_note(
                    "a `port out` is driven by the stage that sends to it, and every stage of a pipeline is live at once holding a different item; send in one stage, or use one port per stage",
                ),
            );
            return None;
        }
        let (value, enable) = match sends.first() {
            None => {
                let zero = low.emit(port.ty.clone(), Op::Const(0));
                let off = low.emit(Ty::BOOL, Op::Const(0));
                (zero, off)
            }
            Some((k, v, guard)) => {
                // The value is stage `k`'s own combinational result and needs
                // no gating; only the enable says which cycles it means
                // anything.
                let held = if *k == 0 {
                    offered
                } else {
                    let bit = low.emit(Ty::BOOL, Op::RegRead((valid_base + k - 1) as u32));
                    low.name_value_safe(bit, format!("v{}", k - 1));
                    bit
                };
                let live = match guard {
                    None => held,
                    Some(g) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: *g, rhs: held }),
                };
                let gated = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: live, rhs: en });
                (*v, gated)
            }
        };
        low.name_value_safe(value, port.name.clone());
        low.name_value_safe(enable, format!("{}_en", port.name));
        drivers.push((port.data_port, value));
        drivers.push((port.en_port, enable));
    }

    if sink.has_errors() {
        return None;
    }
    let asserts = std::mem::take(&mut low.asserts);
    let params = std::mem::take(&mut low.params);
    let mems = std::mem::take(&mut low.mems);
    let (values, ports) = low.take_values();
    Some(crate::ir::Module {
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

