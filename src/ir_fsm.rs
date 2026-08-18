// Blocking channel operations, lowered to a state machine.
//
// `@rcv` and `@send` stall until the transfer happens, so a process using them
// is no longer one pass per cycle -- it is a sequential program with wait
// points, and the wait points are the states.
//
// The body must be a single `loop`. Its statements are split at each blocking
// operation: everything between two barriers is ordinary combinational logic
// belonging to one state, and the barrier itself is what the state waits on.
// A conditional CONTAINING a barrier is rejected rather than mis-scheduled --
// that needs a real control-flow graph, and guessing here would produce a
// machine that silently skips a wait.
//
// Channel rule 3 survives: in a send state `valid` is `state == i`, and state
// is a register, so `valid` still never depends combinationally on `ready`.

use std::collections::{HashMap, HashSet};

use crate::diag::DiagSink;
use crate::ir::{BinOp, Binding, CmpOp, Env, Lowerer, Op, Reg, UnOp, ValueId};
use crate::parse::{BuiltinOp, PrecResExpr, PrecResInnerStmt, anumspan_to_str};
use crate::ty::Ty;

/// A blocking operation: what the state it ends waits on.
pub struct Barrier {
    pub pipe_ix: usize,
    pub is_recv: bool,
    /// The name a receive binds.
    pub bind: Option<String>,
    /// The value a send offers.
    pub value: Option<PrecResExpr>,
}

/// One state: the combinational statements that run in it, then its barrier.
pub struct Segment<'a> {
    pub stmts: Vec<&'a PrecResInnerStmt>,
    pub barrier: Barrier,
}

/// Recognises `let x = @rcv(p)`.
fn as_blocking_recv(stmt: &PrecResInnerStmt) -> Option<(String, String)> {
    let decl = match stmt {
        PrecResInnerStmt::VarDecl(d) if d.rest.is_empty() => d,
        _ => return None,
    };
    let init = decl.assign_val.as_ref()?;
    let (base, args) = match init {
        PrecResExpr::Call { base, args } => (base, args),
        _ => return None,
    };
    match &**base {
        PrecResExpr::Builtin(BuiltinOp::BlockingRecieve) if args.len() == 1 => {}
        _ => return None,
    }
    let pipe = match &args[0] {
        PrecResExpr::Ref(n) => anumspan_to_str(n).to_string(),
        _ => return None,
    };
    Some((anumspan_to_str(&decl.name).to_string(), pipe))
}

/// Recognises `@send(p, v)`.
fn as_blocking_send(stmt: &PrecResInnerStmt) -> Option<(String, PrecResExpr)> {
    let call = match stmt {
        PrecResInnerStmt::CallStmt(c) => c,
        _ => return None,
    };
    match &call.base {
        PrecResExpr::Builtin(BuiltinOp::BlockingSend) if call.args.len() == 2 => {}
        _ => return None,
    }
    let pipe = match &call.args[0] {
        PrecResExpr::Ref(n) => anumspan_to_str(n).to_string(),
        _ => return None,
    };
    Some((pipe, call.args[1].clone()))
}

/// True if this statement contains a blocking operation anywhere inside it.
///
/// Used to reject a barrier nested in a conditional, which cannot be scheduled
/// without a control-flow graph.
fn contains_barrier(stmt: &PrecResInnerStmt) -> bool {
    fn in_expr(e: &PrecResExpr) -> bool {
        match e {
            PrecResExpr::Call { base, args } => {
                let is_blocking = matches!(
                    &**base,
                    PrecResExpr::Builtin(BuiltinOp::BlockingRecieve)
                        | PrecResExpr::Builtin(BuiltinOp::BlockingSend)
                );
                is_blocking || args.iter().any(in_expr)
            }
            PrecResExpr::StmtBlock(b) => b.components.iter().any(contains_barrier),
            PrecResExpr::FieldAccess { base, .. } => in_expr(base),
            PrecResExpr::SubscriptAccess(s) => in_expr(&s.base) || in_expr(&s.index),
            PrecResExpr::Splice(parts) => parts.iter().any(in_expr),
            _ => false,
        }
    }
    match stmt {
        PrecResInnerStmt::VarDecl(d) => d.assign_val.as_ref().is_some_and(in_expr),
        PrecResInnerStmt::CallStmt(c) => {
            let is_blocking = matches!(
                &c.base,
                PrecResExpr::Builtin(BuiltinOp::BlockingRecieve)
                    | PrecResExpr::Builtin(BuiltinOp::BlockingSend)
            );
            is_blocking || c.args.iter().any(in_expr)
        }
        PrecResInnerStmt::AssignStmt(a) => in_expr(&a.rvalue),
        PrecResInnerStmt::IfThenElse(i) => {
            in_expr(&i.condition)
                || in_expr(&i.then_case)
                || i.else_case.as_ref().is_some_and(in_expr)
        }
        PrecResInnerStmt::MatchStmt(m) => {
            m.scrutinees.iter().any(in_expr) || m.cases.iter().any(|c| in_expr(&c.rhs))
        }
        PrecResInnerStmt::TailVal(e) => in_expr(e),
        _ => false,
    }
}

/// Splits a loop body into one segment per blocking operation.
pub fn segment<'a>(
    body: &'a [PrecResInnerStmt],
    pipe_of: &dyn Fn(&str) -> Option<usize>,
    sink: &mut DiagSink,
) -> Option<Vec<Segment<'a>>> {
    let mut segments = Vec::new();
    let mut pending: Vec<&PrecResInnerStmt> = Vec::new();

    for stmt in body {
        if let Some((bind, pipe)) = as_blocking_recv(stmt) {
            let ix = match pipe_of(&pipe) {
                Some(i) => i,
                None => {
                    sink.err_span(
                        crate::driver::nowhere(),
                        format!("`{}` is not a pipe of this process", pipe),
                    );
                    return None;
                }
            };
            segments.push(Segment {
                stmts: std::mem::take(&mut pending),
                barrier: Barrier { pipe_ix: ix, is_recv: true, bind: Some(bind), value: None },
            });
            continue;
        }
        if let Some((pipe, value)) = as_blocking_send(stmt) {
            let ix = match pipe_of(&pipe) {
                Some(i) => i,
                None => {
                    sink.err_span(
                        crate::driver::nowhere(),
                        format!("`{}` is not a pipe of this process", pipe),
                    );
                    return None;
                }
            };
            segments.push(Segment {
                stmts: std::mem::take(&mut pending),
                barrier: Barrier { pipe_ix: ix, is_recv: false, bind: None, value: Some(value) },
            });
            continue;
        }
        if contains_barrier(stmt) {
            sink.err_span(
                crate::driver::nowhere(),
                "a blocking `@rcv` or `@send` inside a conditional is not scheduled yet",
            );
            return None;
        }
        pending.push(stmt);
    }

    if !pending.is_empty() {
        sink.err_span(
            crate::driver::nowhere(),
            "the last statement of a blocking loop must be an `@rcv` or `@send`",
        );
        return None;
    }
    if segments.is_empty() {
        sink.err_span(
            crate::driver::nowhere(),
            "a `loop` with no blocking operation would never advance",
        );
        return None;
    }
    Some(segments)
}

/// Names read by a statement, for deciding which bindings cross a state.
pub fn reads_of(stmt: &PrecResInnerStmt, out: &mut HashSet<String>) {
    fn in_expr(e: &PrecResExpr, out: &mut HashSet<String>) {
        match e {
            PrecResExpr::Ref(n) => {
                out.insert(anumspan_to_str(n).to_string());
            }
            PrecResExpr::Call { base, args } => {
                in_expr(base, out);
                for a in args {
                    in_expr(a, out);
                }
            }
            PrecResExpr::FieldAccess { base, .. } => in_expr(base, out),
            PrecResExpr::SubscriptAccess(s) => {
                in_expr(&s.base, out);
                in_expr(&s.index, out);
            }
            PrecResExpr::Splice(parts) => {
                for p in parts {
                    in_expr(p, out);
                }
            }
            PrecResExpr::StmtBlock(b) => {
                for c in &b.components {
                    reads_of(c, out);
                }
            }
            _ => {}
        }
    }
    match stmt {
        PrecResInnerStmt::VarDecl(d) => {
            if let Some(e) = &d.assign_val {
                in_expr(e, out);
            }
        }
        PrecResInnerStmt::CallStmt(c) => {
            for a in &c.args {
                in_expr(a, out);
            }
        }
        PrecResInnerStmt::AssignStmt(a) => {
            in_expr(&a.rvalue, out);
            in_expr(&a.lvalue, out);
        }
        PrecResInnerStmt::IfThenElse(i) => {
            in_expr(&i.condition, out);
            in_expr(&i.then_case, out);
            if let Some(e) = &i.else_case {
                in_expr(e, out);
            }
        }
        PrecResInnerStmt::MatchStmt(m) => {
            for s in &m.scrutinees {
                in_expr(s, out);
            }
            for c in &m.cases {
                in_expr(&c.rhs, out);
            }
        }
        PrecResInnerStmt::TailVal(e) => in_expr(e, out),
        _ => {}
    }
}

/// Names a segment defines: its plain `let`s plus whatever its barrier binds.
pub fn defines_of(seg: &Segment) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in &seg.stmts {
        if let PrecResInnerStmt::VarDecl(d) = stmt {
            out.insert(anumspan_to_str(&d.name).to_string());
        }
    }
    if let Some(b) = &seg.barrier.bind {
        out.insert(b.clone());
    }
    out
}

/// Bindings defined in one state and read in a later one.
///
/// These cannot be wires: the value has to survive a clock edge, so each gets
/// a register written in its defining state.
pub fn crossing(segments: &[Segment]) -> Vec<HashSet<String>> {
    let defines: Vec<HashSet<String>> = segments.iter().map(defines_of).collect();
    let mut later_reads: Vec<HashSet<String>> = Vec::with_capacity(segments.len());
    for seg in segments {
        let mut r = HashSet::new();
        for stmt in &seg.stmts {
            reads_of(stmt, &mut r);
        }
        if let Some(v) = &seg.barrier.value {
            let tmp = PrecResInnerStmt::TailVal(v.clone());
            reads_of(&tmp, &mut r);
        }
        later_reads.push(r);
    }

    (0..segments.len())
        .map(|i| {
            defines[i]
                .iter()
                .filter(|name| later_reads.iter().skip(i + 1).any(|r| r.contains(*name)))
                .cloned()
                .collect()
        })
        .collect()
}

/// `state == k`, for gating everything that belongs to one state.
pub fn in_state(low: &mut Lowerer, state: ValueId, ty: &Ty, k: u128) -> ValueId {
    let konst = low.emit(ty.clone(), Op::Const(k));
    low.emit(Ty::BOOL, Op::Cmp { op: CmpOp::Eq, lhs: state, rhs: konst })
}

/// ORs a list of conditions; false when the list is empty.
pub fn any_of(low: &mut Lowerer, conds: &[ValueId]) -> ValueId {
    let mut acc: Option<ValueId> = None;
    for c in conds {
        acc = Some(match acc {
            None => *c,
            Some(prev) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: prev, rhs: *c }),
        });
    }
    match acc {
        Some(v) => v,
        None => low.emit(Ty::BOOL, Op::Const(0)),
    }
}

/// The width a state register needs to count `n` states.
pub fn state_width(n: usize) -> u32 {
    let mut w = 1u32;
    while (1usize << w) < n {
        w += 1;
    }
    w
}

/// Finishes a process whose body is a blocking `loop`.
pub fn lower_blocking(
    map: &crate::diag::SourceMap,
    decl: &crate::parse::ProcessDecl,
    mut low: Lowerer,
    mut env: Env,
    out_ports: Vec<(crate::ir::PortId, String)>,
    reg_names: Vec<String>,
    reg_tys: Vec<Ty>,
    reg_resets: Vec<u128>,
    body: &[PrecResInnerStmt],
    sink: &mut DiagSink,
) -> Option<crate::ir::Module> {
    let pipe_names: Vec<String> = low.pipes.iter().map(|p| p.name.clone()).collect();
    let pipe_of = |n: &str| pipe_names.iter().position(|p| p == n);
    let segments = segment(body, &pipe_of, sink)?;
    let n_states = segments.len();

    // Reject a pipe used in a direction it was not declared for.
    for seg in &segments {
        let pipe = &low.pipes[seg.barrier.pipe_ix];
        if seg.barrier.is_recv != pipe.is_input {
            let what = if pipe.is_input { "received from" } else { "sent to" };
            sink.err_span(
                crate::driver::nowhere(),
                format!("`{}` can only be {}", pipe.name, what),
            );
            return None;
        }
    }

    let st_ty = Ty::UInt(state_width(n_states));
    let st_slot = reg_names.len();
    let state = low.emit(st_ty.clone(), Op::RegRead(st_slot as u32));
    low.name_value(state, "state".to_string());

    let mut in_st: Vec<ValueId> = Vec::with_capacity(n_states);
    for k in 0..n_states {
        let c = in_state(&mut low, state, &st_ty, k as u128);
        low.name_value(c, format!("in_s{}", k));
        in_st.push(c);
    }

    // A receive state drives that pipe's ready; a send state drives its valid.
    // `valid` is `state == i` and state is a register, so channel rule 3 holds
    // here exactly as it does for the non-blocking form.
    let mut fires: Vec<ValueId> = Vec::with_capacity(n_states);
    for (k, seg) in segments.iter().enumerate() {
        let pipe = low.pipes[seg.barrier.pipe_ix].clone();
        let other = match if seg.barrier.is_recv { Some(pipe.valid_port) } else { pipe.ready_port } {
            Some(p) => p,
            None => {
                sink.err_span(
                    map.span_of(&decl.name),
                    "a blocking `@send` needs a `buffer` pipe; a `stream` send never blocks",
                );
                return None;
            }
        };
        let handshake = low.emit(Ty::BOOL, Op::Port(other));
        let fire = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: in_st[k], rhs: handshake });
        low.name_value(fire, format!("fire_s{}", k));
        fires.push(fire);
    }

    let mut drivers: Vec<(crate::ir::PortId, ValueId)> = Vec::new();
    for ix in 0..low.pipes.len() {
        let states: Vec<ValueId> = segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.barrier.pipe_ix == ix)
            .map(|(k, _)| in_st[k])
            .collect();
        let active = any_of(&mut low, &states);
        let pipe = low.pipes[ix].clone();
        if pipe.is_input {
            if let Some(ready) = pipe.ready_port {
                drivers.push((ready, active));
            }
        } else {
            drivers.push((pipe.valid_port, active));
        }
    }

    // Allocate a register for every binding that crosses a state, before any
    // segment runs, so a later segment can read the registered copy.
    let cross = crossing(&segments);
    let mut slot_of: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut cross_order: Vec<String> = Vec::new();
    let mut next_slot = st_slot + 1;
    for set in &cross {
        let mut names: Vec<String> = set.iter().cloned().collect();
        names.sort();
        for n in names {
            if !slot_of.contains_key(&n) {
                slot_of.insert(n.clone(), next_slot);
                cross_order.push(n);
                next_slot += 1;
            }
        }
    }

    let mut send_values: Vec<(usize, usize, ValueId)> = Vec::new();
    let mut cross_writes: std::collections::HashMap<String, (usize, ValueId, Ty)> =
        std::collections::HashMap::new();

    for (k, seg) in segments.iter().enumerate() {
        for stmt in &seg.stmts {
            crate::ir::lower_stmt_pub(&mut low, stmt, &mut env, sink)?;
        }
        if let Some(bind) = &seg.barrier.bind {
            let pipe = low.pipes[seg.barrier.pipe_ix].clone();
            let data = pipe.data_value.expect("an input pipe has a data value");
            env.insert(
                bind.clone(),
                Binding { value: Some(data), ty: pipe.ty.clone(), is_output: false },
            );
        }
        // What this state leaves behind for later states.
        for name in &cross[k] {
            if let Some(b) = env.get(name).cloned() {
                if let Some(v) = b.value {
                    cross_writes.insert(name.clone(), (k, v, b.ty.clone()));
                }
            }
        }
        if let Some(expr) = &seg.barrier.value {
            let pipe = low.pipes[seg.barrier.pipe_ix].clone();
            let v = crate::ir::lower_expr(&mut low, expr, &env, sink)?;
            let have = low.ty_of(v);
            if have != pipe.ty {
                sink.err_span(
                    crate::driver::nowhere(),
                    format!(
                        "`{}` carries `{}` but `{}` was sent",
                        pipe.name,
                        pipe.ty.display(),
                        have.display()
                    ),
                );
                return None;
            }
            send_values.push((seg.barrier.pipe_ix, k, v));
        }
        // From here on the name means its registered copy.
        for name in &cross[k] {
            if let Some(slot) = slot_of.get(name).copied() {
                if let Some((_, _, ty)) = cross_writes.get(name).cloned() {
                    let r = low.emit(ty.clone(), Op::RegRead(slot as u32));
                    low.name_value(r, format!("{}_r", name));
                    env.insert(name.clone(), Binding { value: Some(r), ty, is_output: false });
                }
            }
        }
    }

    for ix in 0..low.pipes.len() {
        let pipe = low.pipes[ix].clone();
        if pipe.is_input {
            continue;
        }
        let mine: Vec<(usize, ValueId)> = send_values
            .iter()
            .filter(|(p, _, _)| *p == ix)
            .map(|(_, k, v)| (*k, *v))
            .collect();
        let mut acc = match mine.first() {
            Some((_, v)) => *v,
            None => {
                sink.err_span(
                    map.span_of(&decl.name),
                    format!("`{}` is never sent to", pipe.name),
                );
                return None;
            }
        };
        for (k, v) in mine.iter().skip(1) {
            acc = low.emit(
                pipe.ty.clone(),
                Op::Mux { cond: in_st[*k], then_val: *v, else_val: acc },
            );
        }
        drivers.push((pipe.data_port, acc));
    }

    let mut generated: Vec<Reg> = Vec::new();

    // The state advances only when the current state's barrier fires.
    let last = n_states - 1;
    let mut next_state = low.emit(st_ty.clone(), Op::Const(0));
    for k in (0..n_states).rev() {
        let target = if k == last { 0u128 } else { (k + 1) as u128 };
        let tv = low.emit(st_ty.clone(), Op::Const(target));
        next_state = low.emit(
            st_ty.clone(),
            Op::Mux { cond: fires[k], then_val: tv, else_val: next_state },
        );
    }
    let any_fire = any_of(&mut low, &fires);
    let held = low.emit(
        st_ty.clone(),
        Op::Mux { cond: any_fire, then_val: next_state, else_val: state },
    );
    generated.push(Reg { name: "state".to_string(), ty: st_ty, reset: 0, next: held });

    for name in &cross_order {
        let (k, v, ty) = match cross_writes.get(name).cloned() {
            Some(x) => x,
            None => continue,
        };
        let slot = slot_of[name];
        let cur = low.emit(ty.clone(), Op::RegRead(slot as u32));
        let next = low.emit(
            ty.clone(),
            Op::Mux { cond: fires[k], then_val: v, else_val: cur },
        );
        generated.push(Reg { name: format!("{}_r", name), ty, reset: 0, next });
    }

    for (port_id, name) in &out_ports {
        match env.get(name).and_then(|b| b.value) {
            Some(v) => drivers.push((*port_id, v)),
            None => {
                sink.err_span(
                    map.span_of(&decl.name),
                    format!("output `{}` is never assigned", name),
                );
                return None;
            }
        }
    }

    let mut regs: Vec<Reg> = Vec::new();
    for (ix, name) in reg_names.iter().enumerate() {
        let next = env.get(name).and_then(|b| b.value).expect("a register is bound");
        regs.push(Reg {
            name: name.clone(),
            ty: reg_tys[ix].clone(),
            reset: reg_resets[ix],
            next,
        });
    }
    regs.extend(generated);

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
    })
}
