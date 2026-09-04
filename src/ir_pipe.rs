// `sequence` lowered to a pipeline.
//
// `|||` cuts the body into stages. Everything inside a stage is combinational;
// every cut becomes a register bank, and a validity bit rides alongside the
// data -- desc.md:51's "implicit is_valid condition at each stage".
//
// The whole pipeline shifts together, gated on the sink having a slot
// (desc.md:48: "pipeline fires when all buffer sinks have slots"). Latency is
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

use std::collections::HashSet;

use crate::diag::DiagSink;
use crate::ir::{BinOp, Binding, Env, Lowerer, Op, PortId, Reg, SALT, UnOp, ValueId};
use crate::ir_fsm::reads_of;
use crate::parse::{
    BuiltinOp, PrecResExpr, PrecResInnerStmt, PrecSeqInnerStmt, anumspan_to_str,
};
use crate::ty::Ty;

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
fn as_recv(stmt: &PrecResInnerStmt) -> Option<(String, String)> {
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
        PrecResExpr::Ref(n) => Some((
            anumspan_to_str(&decl.head_name()).to_string(),
            anumspan_to_str(n).to_string(),
        )),
        _ => None,
    }
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

/// Names a stage defines.
fn defines(stage: &[&PrecResInnerStmt], recv_bind: Option<&String>) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in stage {
        if let PrecResInnerStmt::VarDecl(d) = stmt {
            out.extend(d.names().iter().map(|n| anumspan_to_str(n).to_string()));
        }
    }
    if let Some(b) = recv_bind {
        out.insert(b.clone());
    }
    out
}

/// Names a stage reads.
fn reads(stage: &[&PrecResInnerStmt], send: Option<&PrecResExpr>) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in stage {
        reads_of(stmt, &mut out);
    }
    if let Some(e) = send {
        let tmp = PrecResInnerStmt::TailVal(e.clone());
        reads_of(&tmp, &mut out);
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

    let stages = split_stages(&decl.body);
    let n = stages.len();

    // The head reads; the tail sends. desc.md:47 -- only the first stage may
    // block on a read, and the send belongs with the result.
    let mut head_recv: Option<(String, String)> = None;
    let mut tail_send: Option<(String, PrecResExpr)> = None;
    let mut plain: Vec<Vec<&PrecResInnerStmt>> = Vec::with_capacity(n);

    for (k, stage) in stages.iter().enumerate() {
        let mut keep = Vec::new();
        for stmt in stage {
            // The stage split runs before lowering, so there is no anchor
            // stack to read -- the statement in hand is the same place one
            // would have come from.
            let at = crate::ir::stmt_anchor(stmt)
                .map(|a| map.span_of(&a))
                .unwrap_or_else(crate::driver::nowhere);
            if let Some(r) = as_recv(stmt) {
                if k != 0 {
                    sink.err_span(
                        at,
                        "only the first stage of a sequence may block on a read",
                    );
                    return None;
                }
                head_recv = Some(r);
                continue;
            }
            if let Some(s) = as_send(stmt) {
                if k + 1 != n {
                    sink.err_span(at, "a sequence sends from its last stage");
                    return None;
                }
                tail_send = Some(s);
                continue;
            }
            keep.push(*stmt);
        }
        plain.push(keep);
    }

    let (recv_bind, recv_pipe) = match head_recv {
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
    let (send_pipe, send_expr) = match tail_send {
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
    env.insert(recv_bind.clone(), Binding::constant(in_data, low.pipes[in_ix].ty.clone()));

    let defs: Vec<HashSet<String>> = plain
        .iter()
        .enumerate()
        .map(|(k, st)| defines(st, if k == 0 { Some(&recv_bind) } else { None }))
        .collect();
    let rds: Vec<HashSet<String>> = plain
        .iter()
        .enumerate()
        .map(|(k, st)| reads(st, if k + 1 == n { Some(&send_expr) } else { None }))
        .collect();

    let mut pending: Vec<(String, Ty, ValueId, usize)> = Vec::new();

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

    for (k, stage) in plain.iter().enumerate() {
        for stmt in stage {
            crate::ir::lower_stmt_pub(&mut low, stmt, &mut env, sink)?;
        }
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
    }

    // ---- the output -------------------------------------------------------
    let sent = crate::ir::lower_expr(&mut low, &send_expr, &env, sink)?;
    let out_ty = low.pipes[out_ix].ty.clone();
    let have = low.ty_of(sent);
    if have != out_ty {
        sink.err_span(
            low.here(),
            format!(
                "`{}` carries `{}` but `{}` was sent",
                low.pipes[out_ix].name,
                out_ty.display(),
                have.display()
            ),
        );
        return None;
    }

    // The last stage's result is pushed into an entry rather than registered
    // into a head, which is the same flop count arranged differently: two
    // entries and a salt, instead of head, skid, and two occupancy bits.
    let in_empty = low.pipe_empty(in_ix, in_rsalt_q);
    let offered = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: in_empty });

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
    let (values, ports) = low.take_values();
    Some(crate::ir::Module {
        params,
        asserts,
        mems: Vec::new(),
        name: anumspan_to_str(&decl.name).to_string(),
        ports,
        values,
        drivers,
        regs,
        nets: Vec::new(),
        instances: Vec::new(),
    })
}

