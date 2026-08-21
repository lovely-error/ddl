// Blocking channel operations, lowered to a state machine.
//
// `@rcv` and `@send` stall until the transfer happens, so a process using them
// is no longer one pass per cycle -- it is a sequential program with wait
// points, and the wait points are the states.
//
// The body must be a single `loop`. Its statements are split at each blocking
// operation: everything between two barriers is ordinary combinational logic
// belonging to one state, and the barrier itself is what the state waits on.
//
// A conditional containing a barrier makes that a GRAPH rather than a chain.
// Each arm gets its own states and both rejoin at whatever followed the
// conditional, so the state register's next value is a branch rather than an
// increment. The condition is evaluated in the state that branches, which is
// why a barrier state has two statement lists: `stmts` run before its barrier
// fires, and `post` run after, in the same cycle, with the received value in
// scope. `let cmd = @rcv(ctl)` followed by `if cmd == WRITE` therefore costs no
// extra cycle -- the branch is decided in the cycle the receive completes.
//
// Channel rule 3 survives: in a send state `valid` is `state == i`, and state
// is a register, so `valid` still never depends combinationally on `ready`.
//
// A state may also have NO barrier, in which case it fires unconditionally and
// costs one cycle. That is what a conditional arm with work but no wait needs:
// its statements cannot run in the branching state, because they would then
// run on the other arm's path too.

use std::collections::HashSet;

use crate::diag::DiagSink;
use crate::ir::{BinOp, Binding, CmpOp, Env, Lowerer, Op, Reg, SALT, UnOp, ValueId};
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

/// Where control goes.
///
/// `Exit` is the end of the body: the first state again for a `loop`, and the
/// terminal state for a body that runs once. `Halt` is `break` -- the terminal
/// state regardless, which is what makes a process able to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    State(usize),
    Exit,
    Halt,
}

/// What a state does when it fires.
pub enum Next {
    Straight(Target),
    /// `cond` is lowered in the state's own scope, after its barrier binds, so
    /// a branch on a just-received value costs no extra cycle.
    Branch { cond: PrecResExpr, then_t: Target, else_t: Target },
}

/// A synchronous memory read: the address is presented in this state and the
/// value arrives in the next one, which is what a `bram` costs and where the
/// language says the cycle goes.
pub struct MemRead<'a> {
    pub mem_ix: usize,
    pub addr: &'a PrecResExpr,
    pub bind: String,
}

/// One state of the machine.
pub struct State<'a> {
    /// Combinational statements that run while waiting, before the barrier.
    pub stmts: Vec<&'a PrecResInnerStmt>,
    /// What this state waits on. `None` fires unconditionally.
    pub barrier: Option<Barrier>,
    /// A block RAM read this state presents the address for. It fires
    /// unconditionally, like any state with no barrier: what it buys is the
    /// clock edge, not a handshake.
    pub mem_read: Option<MemRead<'a>>,
    /// Statements that run in the cycle the barrier fires, with whatever it
    /// bound already in scope. Only a barrier state has these.
    pub post: Vec<&'a PrecResInnerStmt>,
    pub next: Next,
}

impl<'a> State<'a> {
    /// A state that exists only to hold a branch: no barrier of its own, so a
    /// barrier state immediately before it can absorb it and save the cycle.
    fn is_bare_branch(&self) -> bool {
        self.barrier.is_none() && matches!(self.next, Next::Branch { .. })
    }

    /// A placeholder left where an absorbed state used to be. Unreachable by
    /// construction, and dropped by the renumbering.
    fn dead() -> Self {
        State {
            stmts: Vec::new(),
            barrier: None,
            mem_read: None,
            post: Vec::new(),
            next: Next::Straight(Target::Exit),
        }
    }
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
pub fn contains_barrier(stmt: &PrecResInnerStmt) -> bool {
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

/// `let x = mem[i]` where `mem` reads synchronously.
///
/// Only in statement position, and only as the whole initialiser: a read whose
/// value arrives a cycle later cannot be a subexpression of anything, because
/// there is no way to say that the rest of the expression waits.
fn as_sync_read<'a>(
    stmt: &'a PrecResInnerStmt,
    sync_mem_of: &dyn Fn(&str) -> Option<usize>,
) -> Option<MemRead<'a>> {
    let decl = match stmt {
        PrecResInnerStmt::VarDecl(d) if d.rest.is_empty() && !d.is_mutable => d,
        _ => return None,
    };
    let sub = match decl.assign_val.as_ref()? {
        PrecResExpr::SubscriptAccess(s) => s,
        _ => return None,
    };
    let mem_name = match &sub.base {
        PrecResExpr::Ref(n) => anumspan_to_str(n),
        _ => return None,
    };
    let mem_ix = sync_mem_of(mem_name)?;
    Some(MemRead {
        mem_ix,
        addr: &sub.index,
        bind: anumspan_to_str(&decl.name).to_string(),
    })
}

/// Where to blame a statement the scheduler refuses.
///
/// The scheduler runs before lowering, so it has no `Lowerer` and no anchor
/// stack -- but it does have the statement in front of it, which is the same
/// place the anchor would have come from.
fn anchor_of(stmt: &PrecResInnerStmt, sink: &DiagSink) -> crate::diag::Span {
    anchor_span(stmt, sink).unwrap_or_else(crate::driver::nowhere)
}

fn anchor_span(stmt: &PrecResInnerStmt, sink: &DiagSink) -> Option<crate::diag::Span> {
    crate::ir::stmt_anchor(stmt).map(|at| sink.map().span_of(&at))
}

/// The `if` that a statement is, when its arms need states of their own.
///
/// Three things need them: a blocking operation, a `break`, and a synchronous
/// memory read. All three are "and then a cycle passes", which is what a state
/// is; anything else in an arm is combinational and lowers to a mux.
fn as_branching_if<'a>(
    stmt: &'a PrecResInnerStmt,
    sync_mem_of: &dyn Fn(&str) -> Option<usize>,
) -> Option<&'a crate::parse::ITEStmt> {
    match stmt {
        PrecResInnerStmt::IfThenElse(ite) if needs_states(stmt, sync_mem_of) => Some(ite),
        _ => None,
    }
}

/// Whether a statement has to be scheduled rather than muxed.
fn needs_states(stmt: &PrecResInnerStmt, sync_mem_of: &dyn Fn(&str) -> Option<usize>) -> bool {
    if contains_barrier(stmt) {
        return true;
    }
    fn walk(stmt: &PrecResInnerStmt, sync_mem_of: &dyn Fn(&str) -> Option<usize>) -> bool {
        if matches!(stmt, PrecResInnerStmt::Break) {
            return true;
        }
        if as_sync_read(stmt, sync_mem_of).is_some() {
            return true;
        }
        let in_arm = |e: &PrecResExpr| match e {
            PrecResExpr::StmtBlock(b) => b.components.iter().any(|c| walk(c, sync_mem_of)),
            _ => false,
        };
        match stmt {
            PrecResInnerStmt::IfThenElse(i) => {
                in_arm(&i.then_case) || i.else_case.as_ref().is_some_and(in_arm)
            }
            PrecResInnerStmt::MatchStmt(m) => m.cases.iter().any(|c| in_arm(&c.rhs)),
            _ => false,
        }
    }
    walk(stmt, sync_mem_of)
}

/// The statements of a branch arm.
fn arm_stmts(arm: &PrecResExpr) -> Option<&[PrecResInnerStmt]> {
    match arm {
        PrecResExpr::StmtBlock(b) => Some(&b.components),
        _ => None,
    }
}

/// Builds the state graph for one block of statements.
///
/// Walks BACKWARDS, so the continuation of every statement is already known by
/// the time that statement needs it -- which is what lets an arm rejoin the
/// main line without a fixup pass. `cont` is where control goes when the block
/// runs off its end; the returned target is where control enters it.
///
/// States are pushed in reverse order and renumbered afterwards.
fn schedule<'a>(
    stmts: &'a [PrecResInnerStmt],
    cont: Target,
    states: &mut Vec<State<'a>>,
    pipe_of: &dyn Fn(&str) -> Option<usize>,
    sync_mem_of: &dyn Fn(&str) -> Option<usize>,
    sink: &mut DiagSink,
) -> Option<Target> {
    let mut target = cont;
    // Collected in reverse order, flipped when placed.
    let mut pending: Vec<&'a PrecResInnerStmt> = Vec::new();

    for stmt in stmts.iter().rev() {
        if let Some(barrier) = as_barrier(stmt, pipe_of, sink)? {
            let mut post: Vec<&PrecResInnerStmt> = pending.drain(..).rev().collect();
            // A branch state sitting right after this barrier has no wait of
            // its own, so its work belongs in this state's post scope and its
            // branch becomes this state's. That is what makes
            //   let cmd = @rcv(ctl)
            //   if cmd then ...
            // cost one cycle rather than two.
            let next = match target {
                Target::State(ix) if states[ix].is_bare_branch() => {
                    let absorbed = std::mem::replace(&mut states[ix], State::dead());
                    post.extend(absorbed.stmts);
                    absorbed.next
                }
                other => Next::Straight(other),
            };
            states.push(State {
                stmts: Vec::new(),
                barrier: Some(barrier),
                mem_read: None,
                post,
                next,
            });
            target = Target::State(states.len() - 1);
            continue;
        }

        if let Some(read) = as_sync_read(stmt, sync_mem_of) {
            // Same absorption as a barrier: statements between this read and a
            // branch belong to the state that presents the address, because
            // the value they need arrives at its clock edge.
            let mut post: Vec<&PrecResInnerStmt> = pending.drain(..).rev().collect();
            let next = match target {
                Target::State(ix) if states[ix].is_bare_branch() => {
                    let absorbed = std::mem::replace(&mut states[ix], State::dead());
                    post.extend(absorbed.stmts);
                    absorbed.next
                }
                other => Next::Straight(other),
            };
            states.push(State {
                stmts: Vec::new(),
                barrier: None,
                mem_read: Some(read),
                post,
                next,
            });
            target = Target::State(states.len() - 1);
            continue;
        }

        if let Some(ite) = as_branching_if(stmt, sync_mem_of) {
            if contains_barrier_in_expr(&ite.condition) {
                sink.err_span(
                    anchor_of(stmt, sink),
                    "an `if` condition cannot contain a blocking `@rcv` or `@send`",
                );
                return None;
            }
            let then_stmts = match arm_stmts(&ite.then_case) {
                Some(s) => s,
                None => {
                    sink.err_span(anchor_of(stmt, sink), "expected statements in this branch");
                    return None;
                }
            };
            let then_t = schedule(then_stmts, target, states, pipe_of, sync_mem_of, sink)?;
            let else_t = match &ite.else_case {
                None => target,
                Some(arm) => {
                    let else_stmts = match arm_stmts(arm) {
                        Some(s) => s,
                        None => {
                            sink.err_span(
                                anchor_of(stmt, sink),
                                "expected statements in this branch",
                            );
                            return None;
                        }
                    };
                    schedule(else_stmts, target, states, pipe_of, sync_mem_of, sink)?
                }
            };
            states.push(State {
                stmts: pending.drain(..).rev().collect(),
                barrier: None,
                mem_read: None,
                post: Vec::new(),
                next: Next::Branch { cond: ite.condition.clone(), then_t, else_t },
            });
            target = Target::State(states.len() - 1);
            continue;
        }

        if matches!(stmt, PrecResInnerStmt::Break) {
            // Everything after a `break` on this path is unreachable, and
            // `pending` is exactly the statements after it -- the walk is
            // backwards. Dropping them is what makes `break` mean stop.
            pending.clear();
            target = Target::Halt;
            continue;
        }

        if needs_states(stmt, sync_mem_of) {
            sink.err_span(
                anchor_of(stmt, sink),
                "only an `if` can hold a blocking `@rcv`, a `@send`, a `break` or a `bram` read; a `match` cannot yet",
            );
            return None;
        }
        pending.push(stmt);
    }

    if pending.is_empty() {
        return Some(target);
    }

    // Statements before this block's first wait. They belong in the state that
    // wait created: statements run in a state BEFORE its barrier fires, and
    // that state is entered by whoever jumps here.
    //
    // Unless the entry is `cont`, which the other arm of a branch shares --
    // putting them there would run them on both paths. Those get a state of
    // their own, which costs a cycle and is the only way to guard them.
    let leading: Vec<&PrecResInnerStmt> = pending.drain(..).rev().collect();
    match target {
        Target::State(ix) if target != cont => {
            let mut merged = leading;
            merged.extend(std::mem::take(&mut states[ix].stmts));
            states[ix].stmts = merged;
            Some(Target::State(ix))
        }
        other => {
            states.push(State {
                stmts: leading,
                barrier: None,
                mem_read: None,
                post: Vec::new(),
                next: Next::Straight(other),
            });
            Some(Target::State(states.len() - 1))
        }
    }
}

/// The barrier a statement is, if it is one.
///
/// `Some(None)` for an ordinary statement; the outer `None` is the error path.
fn as_barrier(
    stmt: &PrecResInnerStmt,
    pipe_of: &dyn Fn(&str) -> Option<usize>,
    sink: &mut DiagSink,
) -> Option<Option<Barrier>> {
    let (pipe, bind, value) = if let Some((bind, pipe)) = as_blocking_recv(stmt) {
        (pipe, Some(bind), None)
    } else if let Some((pipe, value)) = as_blocking_send(stmt) {
        (pipe, None, Some(value))
    } else {
        return Some(None);
    };

    match pipe_of(&pipe) {
        Some(pipe_ix) => Some(Some(Barrier {
            pipe_ix,
            is_recv: bind.is_some(),
            bind,
            value,
        })),
        None => {
            sink.err_span(
                anchor_of(stmt, sink),
                format!("`{}` is not a pipe of this process", pipe),
            );
            None
        }
    }
}

/// Whether an expression holds a blocking operation.
fn contains_barrier_in_expr(e: &PrecResExpr) -> bool {
    contains_barrier(&PrecResInnerStmt::TailVal(e.clone()))
}

/// Schedules a whole loop body, renumbered so the entry state is 0.
///
/// Renumbering matters: the state register resets to 0, so the entry has to be
/// 0 for the machine to start where the program does. It also drops whatever
/// state the absorption step left unreachable.
pub fn schedule_body<'a>(
    body: &'a [PrecResInnerStmt],
    pipe_of: &dyn Fn(&str) -> Option<usize>,
    sync_mem_of: &dyn Fn(&str) -> Option<usize>,
    sink: &mut DiagSink,
) -> Option<Vec<State<'a>>> {
    let mut built: Vec<State<'a>> = Vec::new();
    let entry = schedule(body, Target::Exit, &mut built, pipe_of, sync_mem_of, sink)?;

    let entry = match entry {
        Target::State(ix) => ix,
        // No state at all: either nothing blocks, or the body is a bare
        // `break`, which is a process that stops before it starts.
        Target::Exit | Target::Halt => {
            sink.err_span(
                body.first().and_then(|s| anchor_span(s, sink)).unwrap_or_else(crate::driver::nowhere),
                "a `loop` with no blocking operation would never advance",
            );
            return None;
        }
    };

    // Depth-first from the entry, so reachable states come out in an order a
    // reader can follow and unreachable ones are dropped.
    let mut order: Vec<usize> = Vec::new();
    let mut seen = vec![false; built.len()];
    let mut stack = vec![entry];
    while let Some(ix) = stack.pop() {
        if seen[ix] {
            continue;
        }
        seen[ix] = true;
        order.push(ix);
        // Pushed in reverse so the `then` arm is visited before the `else`.
        match &built[ix].next {
            Next::Straight(Target::State(j)) => stack.push(*j),
            Next::Branch { then_t, else_t, .. } => {
                if let Target::State(j) = else_t {
                    stack.push(*j);
                }
                if let Target::State(j) = then_t {
                    stack.push(*j);
                }
            }
            _ => {}
        }
    }

    let mut new_ix = vec![usize::MAX; built.len()];
    for (new, old) in order.iter().enumerate() {
        new_ix[*old] = new;
    }
    let remap = |t: Target| match t {
        Target::State(ix) => Target::State(new_ix[ix]),
        other => other,
    };

    // Moved out rather than cloned: a state owns borrowed statement lists.
    let mut slots: Vec<Option<State<'a>>> = built.into_iter().map(Some).collect();
    let mut states: Vec<State<'a>> = Vec::with_capacity(order.len());
    for old in &order {
        let mut st = slots[*old].take().expect("each state is visited once");
        st.next = match st.next {
            Next::Straight(t) => Next::Straight(remap(t)),
            Next::Branch { cond, then_t, else_t } => {
                Next::Branch { cond, then_t: remap(then_t), else_t: remap(else_t) }
            }
        };
        states.push(st);
    }

    if states.iter().all(|s| s.barrier.is_none() && s.mem_read.is_none()) {
        sink.err_span(
            body.first().and_then(|s| anchor_span(s, sink)).unwrap_or_else(crate::driver::nowhere),
            "a `loop` with no blocking operation would never advance",
        );
        return None;
    }
    Some(states)
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

/// Names a state defines: its plain `let`s plus whatever its barrier binds.
pub fn defines_of(st: &State) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in st.stmts.iter().chain(st.post.iter()) {
        if let PrecResInnerStmt::VarDecl(d) = stmt {
            out.insert(anumspan_to_str(&d.name).to_string());
        }
    }
    if let Some(b) = st.barrier.as_ref().and_then(|b| b.bind.as_ref()) {
        out.insert(b.clone());
    }
    out
}

/// Everything a state reads, including the value its barrier sends and the
/// condition it branches on.
fn reads_in(st: &State) -> HashSet<String> {
    let mut r = HashSet::new();
    for stmt in st.stmts.iter().chain(st.post.iter()) {
        reads_of(stmt, &mut r);
    }
    if let Some(v) = st.barrier.as_ref().and_then(|b| b.value.as_ref()) {
        reads_of(&PrecResInnerStmt::TailVal(v.clone()), &mut r);
    }
    // The address of a synchronous read is presented in the fetch state, which
    // is a cycle after the state that computed it. Miss this and the address
    // reads a port that has already moved on -- a lookup that returns whatever
    // the NEXT request asked for.
    if let Some(read) = &st.mem_read {
        reads_of(&PrecResInnerStmt::TailVal(read.addr.clone()), &mut r);
    }
    if let Next::Branch { cond, .. } = &st.next {
        reads_of(&PrecResInnerStmt::TailVal(cond.clone()), &mut r);
    }
    r
}

/// Bindings defined in one state and read in another.
///
/// These cannot be wires: the value has to survive a clock edge, so each gets
/// a register written in the state that defines it.
///
/// "Another state" rather than "a later state", because with branches there is
/// no total order to be later in. Registering a value no reachable state reads
/// costs a register the synthesizer then removes; failing to register one that
/// is read across an edge is a wrong answer, so the approximation goes the
/// safe way.
pub fn crossing(states: &[State]) -> Vec<HashSet<String>> {
    let defines: Vec<HashSet<String>> = states.iter().map(defines_of).collect();
    let reads: Vec<HashSet<String>> = states.iter().map(reads_in).collect();

    (0..states.len())
        .map(|i| {
            defines[i]
                .iter()
                .filter(|name| {
                    reads
                        .iter()
                        .enumerate()
                        .any(|(j, r)| j != i && r.contains(*name))
                })
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

/// A memory's write port as the environment holds it: three bindings, and
/// what each was before any state ran.
///
/// A struct rather than the six-tuple this was, which needed a comment to say
/// which field was which and had `entry.3` at every use.
struct MemPortStart {
    we_key: String,
    addr_key: String,
    data_key: String,
    we: Option<ValueId>,
    addr: Option<ValueId>,
    data: Option<ValueId>,
}

impl MemPortStart {
    fn keys(&self) -> [(&String, &Option<ValueId>); 3] {
        [
            (&self.we_key, &self.we),
            (&self.addr_key, &self.addr),
            (&self.data_key, &self.data),
        ]
    }
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
    repeats: bool,
    sink: &mut DiagSink,
) -> Option<crate::ir::Module> {
    let pipe_names: Vec<String> = low.pipes.iter().map(|p| p.name.clone()).collect();
    let pipe_of = |n: &str| pipe_names.iter().position(|p| p == n);
    // A `bram` read is a state, and the value it fetches is a register: the
    // address is presented while the state is current, the array is read at
    // that state's clock edge, and the name is that register from the next
    // state on, which makes the cycle visible in the state count.
    //
    // What the ANNOTATION guarantees is that contract, not a primitive.
    // Measured on GowinSynthesis for the GW1NR-9C: examples/bram_lookup at
    // 256x32 infers one SDPB whether it is declared `bram` or `lutram`,
    // because its read is registered either way and the tool is free to
    // choose. k2g_xstage's 32x32 file infers RAM16SDP1/RAM16SDP4 because its
    // read feeds an adder in the same cycle and distributed RAM is the only
    // thing that can do that. The annotation decides which of those a program
    // is ALLOWED to be; the tool picks the cell.
    let sync_mems: Vec<usize> = (0..low.mems.len())
        .filter(|ix| low.mems[*ix].kind != crate::ty::MemKind::LutRam)
        .collect();
    let mem_names: Vec<String> = low.mems.iter().map(|m| m.name.clone()).collect();
    let sync_mem_of = |n: &str| {
        mem_names
            .iter()
            .position(|m| m == n)
            .filter(|ix| sync_mems.contains(ix))
    };
    let states_sched = schedule_body(body, &pipe_of, &sync_mem_of, sink)?;
    let n_states = states_sched.len();

    // Reject a pipe used in a direction it was not declared for.
    for st in &states_sched {
        let barrier = match &st.barrier {
            Some(b) => b,
            None => continue,
        };
        let pipe = &low.pipes[barrier.pipe_ix];
        if barrier.is_recv != pipe.is_input {
            let what = if pipe.is_input { "received from" } else { "sent to" };
            sink.err_span(
                low.here(),
                format!("`{}` can only be {}", pipe.name, what),
            );
            return None;
        }
    }

    // A `loop` wraps back to the first state. A linear body does not: it runs
    // once and stops, which is one more state than there are in the graph. The
    // terminal state has no barrier and no successor, so nothing drives a
    // `valid` or a `ready` in it -- the machine parks there and the registers
    // hold whatever the last pass left.
    // A `break` reaches the terminal state, so a repeating body that contains
    // one still needs it encoded.
    let halts = states_sched.iter().any(|st| match &st.next {
        Next::Straight(t) => *t == Target::Halt,
        Next::Branch { then_t, else_t, .. } => {
            *then_t == Target::Halt || *else_t == Target::Halt
        }
    });
    let done_state = n_states as u128;
    let needs_terminal = !repeats || halts;
    let encoded_states = if needs_terminal { n_states + 1 } else { n_states };
    let st_ty = Ty::UInt(state_width(encoded_states));
    let st_slot = reg_names.len();
    let state = low.emit(st_ty.clone(), Op::RegRead(st_slot as u32));
    low.name_value(state, "state".to_string());

    // PIPE REGISTERS, reserved between the state register and the crossing
    // registers. A consumer holds two bits; a producer holds two entries and
    // two bits -- which for a state machine is new. Its outputs used to be
    // combinational off the state (`dst_valid = in_s2`, `dst_data = a_r + b_r`),
    // and under salt they cannot be: the consumer reads an entry on a cycle
    // this side may already have left.
    let pipe_base = st_slot + 1;
    let mut pipe_regs: Vec<(String, Ty, usize)> = Vec::new();
    for ix in 0..low.pipes.len() {
        let name = low.pipes[ix].name.clone();
        let ty = low.pipes[ix].ty.clone();
        let slot = pipe_base + pipe_regs.len();
        if low.pipes[ix].is_input {
            pipe_regs.push((format!("{}_rsalt_q", name), SALT, slot));
            low.pipes[ix].salt_reg = Some(slot);
        } else {
            pipe_regs.push((format!("{}_e0", name), ty.clone(), slot));
            pipe_regs.push((format!("{}_e1", name), ty, slot + 1));
            pipe_regs.push((format!("{}_wsalt_q", name), SALT, slot + 2));
            low.pipes[ix].slot_reg = Some(slot);
        }
    }
    let pipe_slots = pipe_regs.len();

    // Emitted once per pipe and reused: `empty`/`full` and the entry index are
    // pure functions of two registers, so recomputing them per state only
    // multiplies identical wires in the output.
    for ix in 0..low.pipes.len() {
        let is_input = low.pipes[ix].is_input;
        let slot = if is_input {
            low.pipes[ix].salt_reg.expect("an input pipe has an rsalt register")
        } else {
            low.pipes[ix].slot_reg.expect("an output pipe has a slot") + 2
        };
        let name = low.pipes[ix].name.clone();
        let own = low.emit(SALT, Op::RegRead(slot as u32));
        let blocked =
            if is_input { low.pipe_empty(ix, own) } else { low.pipe_full(ix, own) };
        let movable = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: blocked });
        low.pipes[ix].movable = Some(movable);
        if is_input {
            // `pipe_item_at` emits the index on its way to the entry and
            // records it, so asking for one separately would name a second
            // wire for the same XOR.
            let item = low.pipe_item_at(ix, own);
            low.pipes[ix].item = Some(item);
        } else {
            let idx = low.salt_idx(own, format!("{}_widx", name));
            low.pipes[ix].idx = Some(idx);
        }
    }

    let mut in_st: Vec<ValueId> = Vec::with_capacity(n_states);
    for k in 0..n_states {
        let c = in_state(&mut low, state, &st_ty, k as u128);
        low.name_value(c, format!("in_s{}", k));
        in_st.push(c);
    }

    // A receive state drives that pipe's ready; a send state drives its valid.
    // `valid` is `state == i` and state is a register, so channel rule 3 holds
    // here exactly as it does for the non-blocking form. A state with no
    // barrier has nothing to wait for and fires the cycle it is entered.
    let mut fires: Vec<ValueId> = Vec::with_capacity(n_states);
    for (k, st) in states_sched.iter().enumerate() {
        let barrier = match &st.barrier {
            Some(b) => b,
            None => {
                // Both a read state and a plain always-fire state advance the
                // cycle they are entered; the read state also spends that
                // cycle fetching.
                fires.push(in_st[k]);
                continue;
            }
        };
        let ix = barrier.pipe_ix;
        // What says the transfer can happen: something to take, or somewhere
        // to put one. Both are two registers compared -- ours and theirs.
        let handshake = low.pipes[ix].movable.expect("computed once above");
        let fire = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: in_st[k], rhs: handshake });
        low.name_value(fire, format!("fire_s{}", k));
        fires.push(fire);
    }

    // `ready` and `valid` are driven further down, once the states have been
    // lowered: a non-blocking operation contributes to them too, and which
    // states did one is not known until their statements have run.
    let mut drivers: Vec<(crate::ir::PortId, ValueId)> = Vec::new();
    // Filled as the drivers are built; installed into `generated` in slot
    // order once every state has been lowered.
    let mut pipe_next: Vec<(usize, ValueId)> = Vec::new();
    let mut pushes: Vec<(usize, ValueId, ValueId)> = Vec::new();

    // Allocate a register for every binding that crosses a state, before any
    // state runs, so another one can read the registered copy.
    let cross = crossing(&states_sched);
    let mut slot_of: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut cross_order: Vec<String> = Vec::new();
    let mut next_slot = pipe_base + pipe_slots;
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

    // The read port's register belongs to the MEMORY, not to the state that
    // reads it, because that is what the hardware has: one output register per
    // port. Collected here and settled onto the memory once every state has
    // been lowered, so several read states share one port with a muxed address
    // -- which is also what the hardware has.
    let mut mem_reads: Vec<(usize, usize, ValueId)> = Vec::new();

    let mut send_values: Vec<(usize, usize, ValueId)> = Vec::new();
    // Per pipe: the states that touched it non-blockingly, and the branch the
    // operation sat on when it was not at the top of the state.
    let mut nonblocking: Vec<Vec<(usize, Option<ValueId>)>> =
        vec![Vec::new(); low.pipes.len()];
    // Several states may define the same name, one per arm of a branch, so
    // this is a list rather than a map: the register takes whichever of them
    // fired.
    let mut cross_writes: Vec<(String, usize, ValueId, Ty)> = Vec::new();
    // Where each state's branch condition ended up.
    let mut branch_conds: Vec<Option<ValueId>> = vec![None; n_states];
    // A `var` reads its register at the start of every state, and what a state
    // leaves in it is committed only when that state fires. Without the
    // per-state reset, an assignment written in one state would be the
    // register's next value in every state -- an increment meant to happen
    // once per loop would happen once per cycle.
    let var_start: Vec<Option<ValueId>> = reg_names
        .iter()
        .map(|n| env.get(n).and_then(|b| b.value))
        .collect();
    let mut var_writes: Vec<Vec<(usize, ValueId)>> = vec![Vec::new(); reg_names.len()];

    // A memory's write port is per-state exactly as a `var` is: the address
    // and data a state computed take effect only when that state fires.
    // Without the reset between states, a write in one state would be the
    // write port's value in every state, and a scratchpad updated once per
    // item would be rewritten every cycle.
    let mem_port_start: Vec<MemPortStart> = low
        .mems
        .iter()
        .map(|m| {
            let (we_key, addr_key, data_key) = Lowerer::mem_port_keys(&m.name);
            let we = env.get(&we_key).and_then(|b| b.value);
            let addr = env.get(&addr_key).and_then(|b| b.value);
            let data = env.get(&data_key).and_then(|b| b.value);
            MemPortStart { we_key, addr_key, data_key, we, addr, data }
        })
        .collect();
    // Per memory: which states wrote, and with what.
    let mut mem_writes: Vec<Vec<(usize, ValueId, ValueId, ValueId)>> =
        vec![Vec::new(); low.mems.len()];

    for (k, st) in states_sched.iter().enumerate() {
        // NON-BLOCKING OPERATIONS ARE PER-STATE, which is the whole difference
        // between them here and in a process with no states. `fired` there is
        // "did this pipe transfer this cycle", computed once before the body.
        // Here a pipe can be sampled in one state and offered to in another,
        // so it is "did it transfer while we were in state k" -- and `used`,
        // which stops two operations on one pipe colliding, resets per state
        // rather than per cycle for the same reason.
        for ix in 0..low.pipes.len() {
            let pipe = low.pipes[ix].clone();
            let handshake = pipe.movable.expect("computed once above");
            let f = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: in_st[k], rhs: handshake });
            low.name_value_safe(f, format!("{}_xfer_s{}", pipe.name, k));
            low.pipes[ix].fired = Some(f);
            // A barrier already spends this state's one transfer on that pipe,
            // so anything else consuming from it here would be a second
            // transfer in a cycle that has one. Marking it used is what makes
            // `@drop(p)` beside `@rcv(p)` an error instead of a no-op.
            low.pipes[ix].used = st.barrier.as_ref().is_some_and(|b| b.pipe_ix == ix);
            low.pipes[ix].sent = None;
            low.pipes[ix].send_guard = None;
        }

        for (ix, name) in reg_names.iter().enumerate() {
            if let (Some(v), Some(b)) = (var_start[ix], env.get_mut(name)) {
                b.value = Some(v);
            }
        }
        for port in &mem_port_start {
            for (key, start) in port.keys() {
                if let (Some(v), Some(b)) = (start, env.get_mut(key)) {
                    b.value = Some(*v);
                }
            }
        }

        for stmt in &st.stmts {
            crate::ir::lower_stmt_pub(&mut low, stmt, &mut env, sink)?;
        }
        // A synchronous read: present the address now, and bind the name to
        // the register the value lands in. From this state on the name means
        // that register -- which is a state later, exactly as the hardware
        // does it.
        if let Some(read) = &st.mem_read {
            let addr_width = low.mems[read.mem_ix].addr_width;
            let elem = low.mems[read.mem_ix].elem.clone();
            let raw = crate::ir::lower_expr(&mut low, read.addr, &env, sink)?;
            let addr = low.fit_address(raw, addr_width, &decl.name, sink)?;
            mem_reads.push((read.mem_ix, k, addr));
            // The name means the memory's output register from here on. Not a
            // value computed from the array -- the array is read inside the
            // memory's own clocked block, and this is the only way to see it.
            let q = low.emit(elem.clone(), Op::MemReadReg { mem: read.mem_ix as u32 });
            env.insert(read.bind.clone(), Binding::constant(q, elem));
        }
        if let Some(bind) = st.barrier.as_ref().and_then(|b| b.bind.as_ref()) {
            let pipe = low.pipes[st.barrier.as_ref().expect("a bind implies a barrier").pipe_ix]
                .clone();
            let data = pipe.item.expect("an input pipe has an item");
            env.insert(bind.clone(), Binding::constant(data, pipe.ty.clone()));
        }
        // Statements between the barrier and a branch. They see what the
        // barrier bound, which is what lets the branch decide in the same
        // cycle the transfer completes.
        for stmt in &st.post {
            crate::ir::lower_stmt_pub(&mut low, stmt, &mut env, sink)?;
        }
        if let Next::Branch { cond, .. } = &st.next {
            let v = crate::ir::lower_expr(&mut low, cond, &env, sink)?;
            if low.ty_of(v) != Ty::BOOL {
                sink.err_span(
                    low.here(),
                    format!(
                        "an `if` condition must be `i1`, found `{}`",
                        low.ty_of(v).display()
                    ),
                );
                return None;
            }
            low.name_value(v, format!("branch_s{}", k));
            branch_conds[k] = Some(v);
        }

        // A non-blocking operation asks for the handshake in this state
        // without making the state wait for it, so the state contributes to
        // the pipe's `ready`/`valid` exactly as a barrier state does -- and
        // does NOT contribute to `fires`, which is what "does not wait" means.
        for (ix, uses) in nonblocking.iter_mut().enumerate() {
            let used_here = low.pipes[ix].used;
            let barriered_here = st.barrier.as_ref().is_some_and(|b| b.pipe_ix == ix);
            if !used_here || barriered_here {
                continue;
            }
            uses.push((k, low.pipes[ix].send_guard));
            if !low.pipes[ix].is_input
                && let Some(v) = low.pipes[ix].sent
            {
                send_values.push((ix, k, v));
            }
        }

        // What this state leaves behind for the others.
        for name in &cross[k] {
            if let Some(b) = env.get(name).cloned()
                && let Some(v) = b.value {
                    cross_writes.push((name.clone(), k, v, b.ty.clone()));
                }
        }
        for (ix, name) in reg_names.iter().enumerate() {
            let now = env.get(name).and_then(|b| b.value);
            if now != var_start[ix]
                && let Some(v) = now {
                    var_writes[ix].push((k, v));
                }
        }
        for (ix, port) in mem_port_start.iter().enumerate() {
            let we = env.get(&port.we_key).and_then(|b| b.value);
            let wrote = we.is_some() && we != port.we;
            if !wrote {
                continue;
            }
            let (we, addr, data) = (
                we.expect("checked"),
                env.get(&port.addr_key).and_then(|b| b.value),
                env.get(&port.data_key).and_then(|b| b.value),
            );
            if let (Some(addr), Some(data)) = (addr, data) {
                mem_writes[ix].push((k, we, addr, data));
            }
        }

        if let Some(expr) = st.barrier.as_ref().and_then(|b| b.value.as_ref()) {
            let pipe =
                low.pipes[st.barrier.as_ref().expect("a value implies a barrier").pipe_ix].clone();
            // The send is not a statement as far as lowering is concerned --
            // the scheduler took it apart -- so its anchor has to be pushed
            // here or the type error lands at line 1.
            let depth = crate::ir::expr_anchor(expr).map(|at| {
                let span = low.span_of(&at);
                low.push_anchor(span)
            });
            let v = crate::ir::lower_expr(&mut low, expr, &env, sink)?;
            let have = low.ty_of(v);
            if have != pipe.ty {
                sink.err_span(
                    low.here(),
                    format!(
                        "`{}` carries `{}` but `{}` was sent",
                        pipe.name,
                        pipe.ty.display(),
                        have.display()
                    ),
                );
                return None;
            }
            send_values.push((
                st.barrier.as_ref().expect("a value implies a barrier").pipe_ix,
                k,
                v,
            ));
            if let Some(depth) = depth {
                low.pop_anchor(depth);
            }
        }
        // From here on the name means its registered copy.
        for name in &cross[k] {
            if let Some(slot) = slot_of.get(name).copied() {
                let ty = cross_writes
                    .iter()
                    .rev()
                    .find(|(n, _, _, _)| n == name)
                    .map(|(_, _, _, t)| t.clone());
                if let Some(ty) = ty {
                    let r = low.emit(ty.clone(), Op::RegRead(slot as u32));
                    low.name_value(r, format!("{}_r", name));
                    let stays_mutable = env.get(name).is_some_and(|b| b.is_mutable);
                    env.insert(
                        name.clone(),
                        Binding { value: Some(r), ty, is_output: false, is_mutable: stays_mutable },
                    );
                }
            }
        }
    }

    // A state asks for a pipe's handshake if it waits on it (a barrier) or if
    // it touched it without waiting. The two differ in `fires`, not here.
    for (ix, uses) in nonblocking.iter().enumerate() {
        // `fires[k]`, not `in_st[k]`. Under valid/ready a barrier state
        // ASSERTED and waited: `valid = in_st[k]`, and the transfer was
        // `valid & ready` decided at the other end. There is no asserting
        // here -- the toggle IS the transfer -- so a state that is merely
        // sitting in a stalled send must not push, and a state waiting on an
        // empty pipe must not walk its read index forward.
        let mut asks: Vec<ValueId> = states_sched
            .iter()
            .enumerate()
            .filter(|(_, s)| s.barrier.as_ref().is_some_and(|b| b.pipe_ix == ix))
            .map(|(k, _)| fires[k])
            .collect();
        for (k, guard) in uses {
            // FIRING, not merely being in the state. A state with a barrier
            // does its work in the cycle that barrier completes, so a pipe it
            // touches without waiting is touched then and not before -- which
            // for an offer means not publishing a value computed from an item
            // that has not arrived, and for a drop means not throwing one away.
            //
            // And an operation written inside an `if` only happens on that
            // branch, so the state alone is not the condition either.
            let ask = match guard {
                None => fires[*k],
                Some(g) => {
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: fires[*k], rhs: *g })
                }
            };
            asks.push(ask);
        }
        let active = any_of(&mut low, &asks);
        let pipe = low.pipes[ix].clone();
        low.name_value_fresh(active, format!("{}_take", pipe.name));
        if pipe.is_input {
            // Taking IS toggling. The consumer publishes its own bits and
            // nothing else.
            let slot = pipe.salt_reg.expect("an input pipe has an rsalt register");
            let rsalt_q = low.emit(SALT, Op::RegRead(slot as u32));
            let ridx = pipe.idx.expect("computed once above");
            let next = low.salt_next(rsalt_q, ridx, active);
            pipe_next.push((slot, next));
            drivers.push((pipe.rsalt_port, rsalt_q));
        } else {
            // The push is the transfer; the salt is what says so.
            let base = pipe.slot_reg.expect("an output pipe has a slot");
            let wsalt_q = low.emit(SALT, Op::RegRead((base + 2) as u32));
            let widx = pipe.idx.expect("computed once above");
            let next = low.salt_next(wsalt_q, widx, active);
            pipe_next.push((base + 2, next));
            drivers.push((pipe.wsalt_port, wsalt_q));
            pushes.push((ix, active, widx));
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

        // `acc` used to BE the data port -- a mux over states of combinational
        // values, with no register anywhere. Under salt it is what gets pushed
        // into an entry, because the consumer reads that entry on a cycle this
        // side may already have left.
        let (push, widx) = pushes
            .iter()
            .find(|(p, _, _)| *p == ix)
            .map(|(_, a, w)| (*a, *w))
            .expect("an output pipe was driven above");
        let base = pipe.slot_reg.expect("an output pipe has a slot");
        let e0 = low.emit(pipe.ty.clone(), Op::RegRead(base as u32));
        let e1 = low.emit(pipe.ty.clone(), Op::RegRead((base + 1) as u32));
        let not_widx = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: widx });
        let to_e0 = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: push, rhs: not_widx });
        let to_e1 = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: push, rhs: widx });
        let e0_next =
            low.emit(pipe.ty.clone(), Op::Mux { cond: to_e0, then_val: acc, else_val: e0 });
        let e1_next =
            low.emit(pipe.ty.clone(), Op::Mux { cond: to_e1, then_val: acc, else_val: e1 });
        pipe_next.push((base, e0_next));
        pipe_next.push((base + 1, e1_next));

        let pair = low.pack_entries(e0, e1, &pipe.ty);
        drivers.push((pipe.data_port, pair));
    }

    // One write port, whichever state drove it. The enable is that state's
    // own enable ANDed with its firing, so a state that computed a write it
    // never completed does not perform one; the address and data are muxed by
    // the same firing signal.
    //
    // A memory still has exactly one write port here, as it does everywhere
    // else (k2g_regfile.sv:18-24 records what a second one cost when it was
    // tried). Several states writing it is not several ports: only one state
    // is active in any cycle.
    for ix in 0..low.mems.len() {
        let writes = mem_writes[ix].clone();
        if writes.is_empty() {
            continue;
        }
        let port = &mem_port_start[ix];
        let mut we: Option<ValueId> = None;
        let mut addr = port.addr.expect("a declared memory has an address port");
        let mut data = port.data.expect("a declared memory has a data port");
        for (k, we_k, addr_k, data_k) in &writes {
            let gated = low.emit(
                Ty::BOOL,
                Op::Bin { op: BinOp::And, lhs: fires[*k], rhs: *we_k },
            );
            we = Some(match we {
                None => gated,
                Some(prev) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: prev, rhs: gated }),
            });
            let addr_ty = low.ty_of(addr);
            addr = low.emit(
                addr_ty,
                Op::Mux { cond: gated, then_val: *addr_k, else_val: addr },
            );
            let data_ty = low.ty_of(data);
            data = low.emit(
                data_ty,
                Op::Mux { cond: gated, then_val: *data_k, else_val: data },
            );
        }
        let we = we.expect("a non-empty write list");
        // With one writer the mux selector IS the write enable, so the mux is
        // redundant: the address is a don't-care wherever the enable is low.
        low.mems[ix].addr = low.drop_gated_mux(addr, we);
        low.mems[ix].data = low.drop_gated_mux(data, we);
        low.mems[ix].we = we;
    }

    let mut generated: Vec<Reg> = Vec::new();

    // The state advances only when the current state's barrier fires. Where it
    // advances TO is the state's own successor, which for a branching state is
    // a choice made from the condition it evaluated.
    let mut next_state = low.emit(st_ty.clone(), Op::Const(0));
    for k in (0..n_states).rev() {
        let target_of = |low: &mut Lowerer, t: Target| {
            let value = match t {
                Target::State(j) => j as u128,
                Target::Exit => {
                    if repeats {
                        0u128
                    } else {
                        done_state
                    }
                }
                Target::Halt => done_state,
            };
            low.emit(st_ty.clone(), Op::Const(value))
        };
        let target = match &states_sched[k].next {
            Next::Straight(t) => target_of(&mut low, *t),
            Next::Branch { then_t, else_t, .. } => {
                let then_val = target_of(&mut low, *then_t);
                let else_val = target_of(&mut low, *else_t);
                let cond = branch_conds[k].expect("a branching state lowered its condition");
                low.emit(st_ty.clone(), Op::Mux { cond, then_val, else_val })
            }
        };
        next_state = low.emit(
            st_ty.clone(),
            Op::Mux { cond: fires[k], then_val: target, else_val: next_state },
        );
    }
    let any_fire = any_of(&mut low, &fires);
    let held = low.emit(
        st_ty.clone(),
        Op::Mux { cond: any_fire, then_val: next_state, else_val: state },
    );
    generated.push(Reg { name: "state".to_string(), ty: st_ty, reset: 0, next: held });

    // In slot order, immediately after the state register -- which is where
    // they were reserved.
    pipe_next.sort_by_key(|(slot, _)| *slot);
    debug_assert_eq!(pipe_next.len(), pipe_slots, "every pipe register has a next value");
    for ((name, ty, slot), (nslot, next)) in pipe_regs.iter().zip(pipe_next.iter()) {
        debug_assert_eq!(slot, nslot, "pipe register slots line up with their reservations");
        generated.push(Reg { name: name.clone(), ty: ty.clone(), reset: 0, next: *next });
    }

    for name in &cross_order {
        let writes: Vec<(usize, ValueId, Ty)> = cross_writes
            .iter()
            .filter(|(n, _, _, _)| n == name)
            .map(|(_, k, v, t)| (*k, *v, t.clone()))
            .collect();
        let ty = match writes.first() {
            Some((_, _, t)) => t.clone(),
            None => continue,
        };
        let slot = slot_of[name];
        let mut next = low.emit(ty.clone(), Op::RegRead(slot as u32));
        // Later writers are muxed outermost, but only one state is ever
        // active, so the order between them does not matter.
        for (k, v, _) in &writes {
            next = low.emit(
                ty.clone(),
                Op::Mux { cond: fires[*k], then_val: *v, else_val: next },
            );
        }
        generated.push(Reg { name: format!("{}_r", name), ty, reset: 0, next });
    }

    // Settle each memory's read port: enabled in any state that reads it, with
    // the address muxed by which one. One port however many states use it,
    // because that is what a block RAM has.
    for ix in 0..low.mems.len() {
        let mine: Vec<(usize, ValueId)> = mem_reads
            .iter()
            .filter(|(m, _, _)| *m == ix)
            .map(|(_, k, addr)| (*k, *addr))
            .collect();
        let Some((first_state, first_addr)) = mine.first().copied() else {
            continue;
        };
        let mut en = in_st[first_state];
        let mut addr = first_addr;
        for (k, a) in mine.iter().skip(1) {
            en = low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: en, rhs: in_st[*k] });
            let addr_ty = low.ty_of(addr);
            addr = low.emit(addr_ty, Op::Mux { cond: in_st[*k], then_val: *a, else_val: addr });
        }
        low.mems[ix].read = Some(crate::ir::ReadPort { addr, en });
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
        // A `var` keeps its value unless the state that assigns it fires.
        let mut next = var_start[ix].expect("a register is bound");
        for (k, v) in &var_writes[ix] {
            next = low.emit(
                reg_tys[ix].clone(),
                Op::Mux { cond: fires[*k], then_val: *v, else_val: next },
            );
        }
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
