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
use crate::ir::{BinOp, Binding, Env, Lowerer, Op, PortId, Reg, UnOp, ValueId};
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
        PrecResInnerStmt::VarDecl(d) if d.rest.is_empty() => d,
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
            anumspan_to_str(&decl.name).to_string(),
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
            out.insert(anumspan_to_str(&d.name).to_string());
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

    // The head reads; the tail sends. desc.md:46 -- only the first stage may
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
    // The pipeline moves when its sink can take what is leaving it.
    let mut regs: Vec<Reg> = Vec::new();
    let valid_base = 0usize;
    let v_last = low.emit(Ty::BOOL, Op::RegRead((valid_base + n - 1) as u32));
    low.name_value(v_last, format!("v{}", n - 1));
    let en = match low.pipes[out_ix].ready_port {
        Some(p) => {
            let out_ready = low.emit(Ty::BOOL, Op::Port(p));
            let not_last = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: v_last });
            low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: not_last, rhs: out_ready })
        }
        // A `stream` sink never refuses, so the pipeline never stalls -- which
        // is the point of choosing one. Written as a constant rather than
        // `(!v_last) | 1'b1`, which is the same thing and reads as an
        // oversight.
        None => low.emit(Ty::BOOL, Op::Const(1)),
    };
    low.name_value(en, "shift".to_string());

    // ---- stage bodies -----------------------------------------------------
    let in_data = low.pipes[in_ix].data_value.expect("an input pipe has data");
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

    let mut next_slot = n; // validity bits occupy 0..n
    let mut pending: Vec<(String, Ty, ValueId, usize)> = Vec::new();

    for (k, stage) in plain.iter().enumerate() {
        for stmt in stage {
            crate::ir::lower_stmt_pub(&mut low, stmt, &mut env, sink)?;
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

    // The last stage's result is registered like every other boundary, so it
    // leaves alongside the validity bit that belongs to it. Without this a
    // one-stage sequence would offer the current input while advertising the
    // validity of the previous one.
    let out_slot = next_slot;
    let out_held = low.emit(out_ty.clone(), Op::RegRead(out_slot as u32));
    low.name_value(out_held, "out_hold".to_string());
    let out_next = low.emit(
        out_ty.clone(),
        Op::Mux { cond: en, then_val: sent, else_val: out_held },
    );
    pending.push(("out_hold".to_string(), out_ty.clone(), out_next, out_slot));

    // ---- the validity chain ----------------------------------------------
    let in_valid = low.emit(Ty::BOOL, Op::Port(low.pipes[in_ix].valid_port));
    let mut valid_regs: Vec<Reg> = Vec::with_capacity(n);
    for k in 0..n {
        let cur = low.emit(Ty::BOOL, Op::RegRead((valid_base + k) as u32));
        low.name_value(cur, format!("v{}", k));
        let feed = if k == 0 {
            in_valid
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

    let mut drivers: Vec<(PortId, ValueId)> = Vec::new();
    if let Some(ready) = low.pipes[in_ix].ready_port {
        drivers.push((ready, en));
    }
    drivers.push((low.pipes[out_ix].valid_port, v_last));
    drivers.push((low.pipes[out_ix].data_port, out_held));

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

