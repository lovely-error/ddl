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
use crate::lex::{AlphanumSpan, BindingPattern};
use crate::parse::{BuiltinOp, PrecResExpr, PrecResInnerStmt, anumspan_to_str};
use crate::ty::Ty;

/// What a switching state resolved its patterns to: the tag it selects on,
/// and the labels of each case in source order.
///
/// `None` for a case that selects nothing -- an `@unreachable` arm.
type SwitchSel = (ValueId, Vec<Option<Vec<u128>>>);

struct LocalReg {
    name: String,
    ty: Ty,
    held: ValueId,
    writes: Vec<(usize, ValueId)>,
}

/// A blocking operation: what the state it ends waits on.
pub struct Barrier {
    /// Which channel, and of which kind.
    ///
    /// A port is a pipe without back-pressure, so `@rcv` and `@send` mean on
    /// one what they mean on the other and cost the same cycle. What differs
    /// is underneath: a receive waits on the enable instead of on two salts
    /// disagreeing, and a send never waits at all, because there is no `ready`
    /// coming back to wait for.
    pub on: BarrierOn,
    pub is_recv: bool,
    /// The name a receive binds.
    pub bind: Option<String>,
    /// The value a send offers.
    pub value: Option<PrecResExpr>,
    /// Preserve the receive declaration's mutability, type and source anchor.
    pub declaration: Option<crate::parse::VarDeclStmt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierOn {
    Pipe(usize),
    Port(usize),
}

impl Barrier {
    /// The pipe this waits on, or `None` when it waits on a port.
    pub fn pipe(&self) -> Option<usize> {
        match self.on {
            BarrierOn::Pipe(ix) => Some(ix),
            BarrierOn::Port(_) => None,
        }
    }

    pub fn port(&self) -> Option<usize> {
        match self.on {
            BarrierOn::Port(ix) => Some(ix),
            BarrierOn::Pipe(_) => None,
        }
    }
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
    /// A `match` whose arms need states of their own.
    ///
    /// The patterns are kept rather than resolved here: turning a variant name
    /// into a discriminant needs the symbol table, and the scheduler runs
    /// before lowering. `targets` is one per case of `stmt`, in order.
    Switch { stmt: crate::parse::MatchStmt, targets: Vec<Target> },
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
    /// Something already jumps here, so this state cannot be folded into the
    /// one before it. A loop's entry is the case: the body's last statement
    /// comes back to it, and absorbing it would leave that edge pointing at a
    /// state that had been emptied out.
    pub pinned: bool,
}

impl<'a> State<'a> {
    /// A state that exists only to hold a branch: no barrier of its own, so a
    /// barrier state immediately before it can absorb it and save the cycle.
    fn is_bare_branch(&self) -> bool {
        let forks = matches!(self.next, Next::Branch { .. } | Next::Switch { .. });
        !self.pinned && self.barrier.is_none() && forks
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
            pinned: false,
        }
    }
}

/// Recognises `let x = @rcv(p)`.
fn as_blocking_recv(stmt: &PrecResInnerStmt) -> Option<(String, String)> {
    let decl = match stmt {
        PrecResInnerStmt::VarDecl(d) if d.names().len() == 1 => d,
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
    Some((anumspan_to_str(&decl.head_name()).to_string(), pipe))
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
        PrecResInnerStmt::Loop(l) => in_expr(&l.repeat_expr),
        _ => false,
    }
}

/// `let x = mem[i]` where `mem` reads synchronously.
///
/// Only in statement position, and only as the whole initialiser: a read whose
/// value arrives a cycle later cannot be a subexpression of anything, because
/// there is no way to say that the rest of the expression waits.
pub(crate) fn as_sync_read<'a>(
    stmt: &'a PrecResInnerStmt,
    sync_mem_of: &dyn Fn(&str) -> Option<usize>,
) -> Option<MemRead<'a>> {
    let decl = match stmt {
        PrecResInnerStmt::VarDecl(d) if d.names().len() == 1 && !d.is_mutable => d,
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
        bind: anumspan_to_str(&decl.head_name()).to_string(),
    })
}

/// Every synchronous memory read, lifted out of the expression it sits in.
///
/// A `bram` read costs a cycle, and a cycle has to be a state. `let x = m[i]`
/// on its own line always could be one; `m[i] + m[j]` could not, and said so
/// -- "a read of `m` takes a cycle, so it cannot sit inside an expression".
/// That was a scheduling limit dressed as a language rule. The reads in an
/// expression are ordinary reads in a fixed order, so the compiler can put
/// each in its own state and leave the expression reading the names.
///
/// Rewriting rather than scheduling in place, because the scheduler walks
/// borrowed statements and a lifted read is a statement that was not written.
/// The generated names are kept beside the statements that use them: an
/// `AlphanumSpan` is a pointer into text, so the text has to outlive the AST.
///
/// A read is lifted to just before the statement that used it and no further.
/// Inside an `if` arm it stays in that arm, because the cycle it costs is only
/// spent on that path; out of an `if` CONDITION it lifts to before the branch,
/// which is where a value the branch depends on has to be.
pub struct Hoisted {
    pub stmts: Vec<PrecResInnerStmt>,
    /// Backing text for the generated names. Dropping this while `stmts` is
    /// alive would leave every generated `Ref` dangling.
    _names: Vec<Box<str>>,
}

struct Hoister<'m> {
    sync_mem_of: &'m dyn Fn(&str) -> Option<usize>,
    names: Vec<Box<str>>,
    next: u32,
}

impl Hoister<'_> {
    /// A name no source can collide with: `@` cannot start a DDL identifier,
    /// and a builtin is resolved before a reference ever reaches here.
    fn fresh(&mut self) -> AlphanumSpan {
        let text: Box<str> = format!("@rd{}", self.next).into_boxed_str();
        self.next += 1;
        // The Box's contents do not move when the Box itself is pushed, so
        // this pointer stays good for as long as `names` is held.
        let span = AlphanumSpan { byte_ptr: text.as_ptr(), len: text.len() as u32 };
        self.names.push(text);
        span
    }

    /// `m[i]` where `m` reads synchronously.
    fn is_sync_read(&self, e: &PrecResExpr) -> bool {
        let PrecResExpr::SubscriptAccess(sub) = e else {
            return false;
        };
        let PrecResExpr::Ref(name) = &sub.base else {
            return false;
        };
        (self.sync_mem_of)(anumspan_to_str(name)).is_some()
    }

    fn expr(&mut self, e: &PrecResExpr, pre: &mut Vec<PrecResInnerStmt>) -> PrecResExpr {
        if self.is_sync_read(e) {
            let PrecResExpr::SubscriptAccess(sub) = e else {
                unreachable!("checked by is_sync_read")
            };
            // The index first: a read whose address is itself a read needs the
            // inner one to have happened, and the order the states come out in
            // is the order they are pushed.
            let index = self.expr(&sub.index, pre);
            let name = self.fresh();
            pre.push(PrecResInnerStmt::VarDecl(crate::parse::VarDeclStmt {
                is_mutable: false,
                binding: crate::lex::VarBindingKind::PlainName(name),
                ty_expr: None,
                assign_val: Some(PrecResExpr::SubscriptAccess(Box::new(
                    crate::parse::SubscriptAccess { base: sub.base.clone(), index },
                ))),
            }));
            return PrecResExpr::Ref(name);
        }
        match e {
            PrecResExpr::FieldAccess { base, field_name } => PrecResExpr::FieldAccess {
                base: Box::new(self.expr(base, pre)),
                field_name: *field_name,
            },
            PrecResExpr::Call { base, args } => PrecResExpr::Call {
                base: Box::new(self.expr(base, pre)),
                args: args.iter().map(|a| self.expr(a, pre)).collect(),
            },
            PrecResExpr::SubscriptAccess(sub) => {
                PrecResExpr::SubscriptAccess(Box::new(crate::parse::SubscriptAccess {
                    base: self.expr(&sub.base, pre),
                    index: self.expr(&sub.index, pre),
                }))
            }
            PrecResExpr::Splice(parts) => {
                PrecResExpr::Splice(parts.iter().map(|p| self.expr(p, pre)).collect())
            }
            PrecResExpr::Span(sp) => PrecResExpr::Span(Box::new(crate::parse::Span {
                left: self.expr(&sp.left, pre),
                right: self.expr(&sp.right, pre),
            })),
            // A block is statements, and statements are the other half of this
            // walk: a read inside one belongs to that block, not out here.
            PrecResExpr::StmtBlock(b) => PrecResExpr::StmtBlock(crate::parse::StmtBlock {
                components: self.block(&b.components),
            }),
            other => other.clone(),
        }
    }

    fn block(&mut self, stmts: &[PrecResInnerStmt]) -> Vec<PrecResInnerStmt> {
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            self.stmt(stmt, &mut out);
        }
        out
    }

    fn stmt(&mut self, stmt: &PrecResInnerStmt, out: &mut Vec<PrecResInnerStmt>) {
        // `let x = m[i]` on its own line is already a read state, spelled the
        // way the language documents it. Lifting it would only rename it.
        if let PrecResInnerStmt::VarDecl(d) = stmt
            && d.names().len() == 1
            && !d.is_mutable
            && d.assign_val.as_ref().is_some_and(|e| self.is_sync_read(e))
        {
            out.push(stmt.clone());
            return;
        }

        let mut pre: Vec<PrecResInnerStmt> = Vec::new();
        let rewritten = match stmt {
            PrecResInnerStmt::VarDecl(d) => {
                let assign_val = d.assign_val.as_ref().map(|e| self.expr(e, &mut pre));
                PrecResInnerStmt::VarDecl(crate::parse::VarDeclStmt {
                    is_mutable: d.is_mutable,
                    binding: d.binding.clone(),
                    ty_expr: d.ty_expr.clone(),
                    assign_val,
                })
            }
            PrecResInnerStmt::CallStmt(c) => PrecResInnerStmt::CallStmt(crate::parse::CallStmt {
                base: c.base.clone(),
                args: c.args.iter().map(|a| self.expr(a, &mut pre)).collect(),
            }),
            PrecResInnerStmt::AssignStmt(a) => {
                PrecResInnerStmt::AssignStmt(crate::parse::AssignStmt {
                    lvalue: self.lvalue(&a.lvalue, &mut pre),
                    rvalue: self.expr(&a.rvalue, &mut pre),
                    kind: a.kind,
                })
            }
            PrecResInnerStmt::IfThenElse(i) => {
                let condition = self.expr(&i.condition, &mut pre);
                PrecResInnerStmt::IfThenElse(crate::parse::ITEStmt {
                    condition,
                    then_case: self.arm(&i.then_case),
                    else_case: i.else_case.as_ref().map(|e| self.arm(e)),
                })
            }
            PrecResInnerStmt::MatchStmt(m) => {
                let scrutinees = m.scrutinees.iter().map(|e| self.expr(e, &mut pre)).collect();
                PrecResInnerStmt::MatchStmt(crate::parse::MatchStmt {
                    scrutinees,
                    cases: m
                        .cases
                        .iter()
                        .map(|c| crate::parse::MatchArm {
                            binding_patterns: c.binding_patterns.clone(),
                            rhs: self.arm(&c.rhs),
                        })
                        .collect(),
                })
            }
            PrecResInnerStmt::Loop(l) => PrecResInnerStmt::Loop(crate::parse::LoopStmt {
                repeat_expr: self.arm(&l.repeat_expr),
            }),
            PrecResInnerStmt::ForLoop(f) => {
                PrecResInnerStmt::ForLoop(Box::new(crate::parse::ForLoopStmt {
                    binding: f.binding,
                    target: f.target.clone(),
                    body: self.arm(&f.body),
                }))
            }
            PrecResInnerStmt::TailVal(e) => PrecResInnerStmt::TailVal(self.expr(e, &mut pre)),
            other => other.clone(),
        };
        out.append(&mut pre);
        out.push(rewritten);
    }

    /// An assignment target. `m[a] = d` is the memory's WRITE port, not a
    /// read of it, so the subscript stays where it is -- lifting it would turn
    /// every store into a load and then assign to the loaded value. Anything
    /// inside the address is still an ordinary expression.
    fn lvalue(&mut self, e: &PrecResExpr, pre: &mut Vec<PrecResInnerStmt>) -> PrecResExpr {
        if self.is_sync_read(e) {
            let PrecResExpr::SubscriptAccess(sub) = e else {
                unreachable!("checked by is_sync_read")
            };
            return PrecResExpr::SubscriptAccess(Box::new(crate::parse::SubscriptAccess {
                base: sub.base.clone(),
                index: self.expr(&sub.index, pre),
            }));
        }
        self.expr(e, pre)
    }

    /// A branch arm: its reads stay inside it.
    fn arm(&mut self, e: &PrecResExpr) -> PrecResExpr {
        match e {
            PrecResExpr::StmtBlock(b) => PrecResExpr::StmtBlock(crate::parse::StmtBlock {
                components: self.block(&b.components),
            }),
            other => {
                let mut pre = Vec::new();
                let rewritten = self.expr(other, &mut pre);
                if pre.is_empty() {
                    return rewritten;
                }
                pre.push(PrecResInnerStmt::TailVal(rewritten));
                PrecResExpr::StmtBlock(crate::parse::StmtBlock { components: pre })
            }
        }
    }
}

/// Lifts every synchronous read in `body` onto a line of its own.
pub fn hoist_sync_reads(
    body: &[PrecResInnerStmt],
    sync_mem_of: &dyn Fn(&str) -> Option<usize>,
) -> Hoisted {
    let mut h = Hoister { sync_mem_of, names: Vec::new(), next: 0 };
    let stmts = h.block(body);
    Hoisted { stmts, _names: h.names }
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
    crate::ir::stmt_anchor(stmt).map(|at| sink.span_of(&at))
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
        // A `loop` is a back edge, and a back edge is a state graph. Even a
        // body of nothing but combinational work has to be able to come round
        // again, which a mux cannot express.
        if matches!(stmt, PrecResInnerStmt::Loop(_)) {
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

/// Places statements that have no state of their own yet.
///
/// They run before whatever `target` is, so they join that state -- combinational
/// work costs no cycle wherever it sits. Unless `target` is `cont`, which a
/// sibling arm shares: putting them there would run them on its path too, so
/// those get a state of their own and the cycle that costs.
fn place_pending<'a>(
    pending: &mut Vec<&'a PrecResInnerStmt>,
    target: Target,
    cont: Target,
    states: &mut Vec<State<'a>>,
) -> Target {
    if pending.is_empty() {
        return target;
    }
    let leading: Vec<&'a PrecResInnerStmt> = pending.drain(..).rev().collect();
    match target {
        Target::State(ix) if target != cont && !states[ix].pinned => {
            let mut merged = leading;
            merged.extend(std::mem::take(&mut states[ix].stmts));
            states[ix].stmts = merged;
            Target::State(ix)
        }
        other => {
            states.push(State {
                stmts: leading,
                barrier: None,
                mem_read: None,
                post: Vec::new(),
                next: Next::Straight(other),
                pinned: false,
            });
            Target::State(states.len() - 1)
        }
    }
}

/// Repoints every edge that goes to `from` at `to`.
///
/// A loop's back edge cannot be written when the body is scheduled: the walk
/// is backwards, so the body's last statement needs the loop's FIRST state and
/// that is the last thing decided. The body is scheduled against a placeholder
/// instead, and this closes the loop once the entry is known. The placeholder
/// is then unreachable, and the renumbering in `schedule_body` drops it.
fn retarget(states: &mut [State], from: usize, to: Target) {
    fn swap(t: &mut Target, from: usize, to: Target) {
        if *t == Target::State(from) {
            *t = to;
        }
    }
    for st in states.iter_mut() {
        match &mut st.next {
            Next::Straight(t) => swap(t, from, to),
            Next::Branch { then_t, else_t, .. } => {
                swap(then_t, from, to);
                swap(else_t, from, to);
            }
            Next::Switch { targets, .. } => {
                for t in targets.iter_mut() {
                    swap(t, from, to);
                }
            }
        }
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
///
/// `brk` is where `break` goes: the terminal state at the top of a process, and
/// the statement after the loop inside a nested one. Carried rather than
/// global, because "the innermost loop" is exactly what a parameter threaded
/// through the recursion says and a flag would not.
fn schedule<'a>(
    stmts: &'a [PrecResInnerStmt],
    cont: Target,
    brk: Target,
    states: &mut Vec<State<'a>>,
    pipe_of: &dyn Fn(&str) -> Option<usize>,
    port_of: &dyn Fn(&str) -> Option<(usize, bool)>,
    sync_mem_of: &dyn Fn(&str) -> Option<usize>,
    sink: &mut DiagSink,
) -> Option<Target> {
    let mut target = cont;
    // Collected in reverse order, flipped when placed.
    let mut pending: Vec<&'a PrecResInnerStmt> = Vec::new();

    for stmt in stmts.iter().rev() {
        if let Some(barrier) = as_barrier(stmt, pipe_of, port_of, sink)? {
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
                pinned: false,
            });
            target = Target::State(states.len() - 1);
            continue;
        }

        if let Some(read) = as_sync_read(stmt, sync_mem_of) {
            // A read state has NO `post`, and absorbs no branch.
            //
            // This is where it differs from a barrier, and the difference is
            // the clock edge. What `@rcv` binds is a wire off the pipe, there
            // in the cycle the transfer completes, so statements after it --
            // and a branch on what it bound -- run in that same cycle. What a
            // `bram` read binds is the memory's output register, and the array
            // is read at the END of the state that presents the address: for
            // the whole of that state the register still holds the PREVIOUS
            // read.
            //
            // So everything after the read goes to the next state, where the
            // value exists. Merged into it rather than given one of its own,
            // because combinational work costs no cycle wherever it sits --
            // only the branch, which has to have a state to be decided in,
            // costs the cycle the absorption used to save wrongly.
            target = place_pending(&mut pending, target, cont, states);
            states.push(State {
                stmts: Vec::new(),
                barrier: None,
                mem_read: Some(read),
                post: Vec::new(),
                next: Next::Straight(target),
                pinned: false,
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
            // These statements FOLLOW the branch. Break must bypass them,
            // and its condition must not observe their assignments.
            target = place_pending(&mut pending, target, cont, states);
            let then_stmts = match arm_stmts(&ite.then_case) {
                Some(s) => s,
                None => {
                    sink.err_span(anchor_of(stmt, sink), "expected statements in this branch");
                    return None;
                }
            };
            let then_t = schedule(then_stmts, target, brk, states, pipe_of, port_of, sync_mem_of, sink)?;
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
                    schedule(else_stmts, target, brk, states, pipe_of, port_of, sync_mem_of, sink)?
                }
            };
            states.push(State {
                stmts: Vec::new(),
                barrier: None,
                mem_read: None,
                post: Vec::new(),
                next: Next::Branch { cond: ite.condition.clone(), then_t, else_t },
                pinned: false,
            });
            target = Target::State(states.len() - 1);
            continue;
        }

        if let PrecResInnerStmt::MatchStmt(m) = stmt
            && needs_states(stmt, sync_mem_of)
        {
            if m.scrutinees.iter().any(contains_barrier_in_expr) {
                sink.err_span(
                    anchor_of(stmt, sink),
                    "a `match` scrutinee cannot contain a blocking `@rcv` or `@send`",
                );
                return None;
            }
            // Every arm rejoins at whatever followed the `match`, exactly as
            // the two arms of an `if` do. An arm with no statements -- a bare
            // expression, or `@unreachable` -- falls straight through, which
            // for `@unreachable` is a target nothing ever selects.
            target = place_pending(&mut pending, target, cont, states);
            let mut targets = Vec::with_capacity(m.cases.len());
            for case in &m.cases {
                let t = match arm_stmts(&case.rhs) {
                    Some(arm) => {
                        schedule(arm, target, brk, states, pipe_of, port_of, sync_mem_of, sink)?
                    }
                    None => target,
                };
                targets.push(t);
            }
            states.push(State {
                stmts: Vec::new(),
                barrier: None,
                mem_read: None,
                post: Vec::new(),
                next: Next::Switch { stmt: m.clone(), targets },
                pinned: false,
            });
            target = Target::State(states.len() - 1);
            continue;
        }

        if matches!(stmt, PrecResInnerStmt::Break) {
            // Everything after a `break` on this path is unreachable, and
            // `pending` is exactly the statements after it -- the walk is
            // backwards. Dropping them is what makes `break` mean leave.
            pending.clear();
            target = brk;
            continue;
        }

        if let PrecResInnerStmt::Loop(l) = stmt {
            let body = match arm_stmts(&l.repeat_expr) {
                Some(b) => b,
                None => {
                    sink.err_span(anchor_of(stmt, sink), "a `loop` needs an indented body");
                    return None;
                }
            };
            // Statements after the loop are reachable only by `break`, so they
            // are placed first: the loop's exit is where they start.
            let after = place_pending(&mut pending, target, cont, states);
            // A placeholder for the top of this loop. Its index is known now;
            // where it points is known once the body has been scheduled.
            states.push(State::dead());
            let head = states.len() - 1;
            let entry =
                schedule(body, Target::State(head), after, states, pipe_of, port_of, sync_mem_of, sink)?;
            let body_has_no_states = entry == Target::State(head);
            if body_has_no_states {
                sink.err_span(
                    anchor_of(stmt, sink),
                    "a `loop` with no blocking operation would never advance",
                );
                return None;
            }
            retarget(states, head, entry);
            // The body now jumps back here, so the state before the loop must
            // not absorb it. Without the pin, `let x = @rcv(p)` above a loop
            // folds the loop's own branch into the receive state and the back
            // edge lands on what is left.
            if let Target::State(ix) = entry {
                states[ix].pinned = true;
            }
            target = entry;
            continue;
        }

        if needs_states(stmt, sync_mem_of) {
            sink.err_span(
                anchor_of(stmt, sink),
                "a blocking `@rcv`, a `@send`, a `break` or a `bram` read needs a statement that can hold states: an `if`, a `match` or a `loop`",
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
    Some(place_pending(&mut pending, target, cont, states))
}

/// The barrier a statement is, if it is one.
///
/// `Some(None)` for an ordinary statement; the outer `None` is the error path.
fn as_barrier(
    stmt: &PrecResInnerStmt,
    pipe_of: &dyn Fn(&str) -> Option<usize>,
    port_of: &dyn Fn(&str) -> Option<(usize, bool)>,
    sink: &mut DiagSink,
) -> Option<Option<Barrier>> {
    let (pipe, bind, value) = if let Some((bind, pipe)) = as_blocking_recv(stmt) {
        (pipe, Some(bind), None)
    } else if let Some((pipe, value)) = as_blocking_send(stmt) {
        (pipe, None, Some(value))
    } else {
        return Some(None);
    };
    let is_recv = bind.is_some();
    let declaration = match stmt {
        PrecResInnerStmt::VarDecl(d) => Some(d.clone()),
        _ => None,
    };

    if let Some(pipe_ix) = pipe_of(&pipe) {
        return Some(Some(Barrier { on: BarrierOn::Pipe(pipe_ix), is_recv, bind, value, declaration }));
    }
    if let Some((port_ix, is_input)) = port_of(&pipe) {
        if is_recv != is_input {
            let what = if is_input { "received from" } else { "sent to" };
            sink.err_span(
                anchor_of(stmt, sink),
                format!("`{}` can only be {}", pipe, what),
            );
            return None;
        }
        return Some(Some(Barrier { on: BarrierOn::Port(port_ix), is_recv, bind, value, declaration }));
    }
    sink.err_span(
        anchor_of(stmt, sink),
        format!("`{}` is not a pipe or `port` of this process", pipe),
    );
    None
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
    port_of: &dyn Fn(&str) -> Option<(usize, bool)>,
    sync_mem_of: &dyn Fn(&str) -> Option<usize>,
    sink: &mut DiagSink,
) -> Option<Vec<State<'a>>> {
    let mut built: Vec<State<'a>> = Vec::new();
    // At the top of a process, `break` is what makes it stop: desc.md:37, "may
    // stop (reach terminal state)". Inside a nested loop it means leave that
    // loop, which is what the `brk` parameter carries down.
    let entry = schedule(body, Target::Exit, Target::Halt, &mut built, pipe_of, port_of, sync_mem_of, sink)?;

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
            Next::Switch { targets, .. } => {
                for t in targets.iter().rev() {
                    if let Target::State(j) = t {
                        stack.push(*j);
                    }
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
            Next::Switch { stmt, targets } => Next::Switch {
                stmt,
                targets: targets.into_iter().map(remap).collect(),
            },
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
        PrecResInnerStmt::Loop(l) => in_expr(&l.repeat_expr, out),
        _ => {}
    }
}

/// Memory names a statement WRITES: the base of every `m[i] = v` in it.
///
/// Separate from `reads_of`, which reports the base of a subscript assignment
/// too -- it has to, because the address is read either way. Telling the two
/// apart is what decides whether a memory is a table (readable from anywhere,
/// nothing to order) or storage (one stage owns it).
pub(crate) fn writes_of(stmt: &PrecResInnerStmt, out: &mut HashSet<String>) {
    fn in_expr(e: &PrecResExpr, out: &mut HashSet<String>) {
        // Only a block can hold statements; every other expression form is a
        // value, and a value writes nothing.
        if let PrecResExpr::StmtBlock(b) = e {
            for c in &b.components {
                writes_of(c, out);
            }
        }
    }
    match stmt {
        PrecResInnerStmt::AssignStmt(a) => {
            if let PrecResExpr::SubscriptAccess(sub) = &a.lvalue
                && let PrecResExpr::Ref(n) = &sub.base
            {
                out.insert(anumspan_to_str(n).to_string());
            }
        }
        PrecResInnerStmt::IfThenElse(i) => {
            in_expr(&i.then_case, out);
            if let Some(e) = &i.else_case {
                in_expr(e, out);
            }
        }
        PrecResInnerStmt::MatchStmt(m) => {
            for c in &m.cases {
                in_expr(&c.rhs, out);
            }
        }
        PrecResInnerStmt::Loop(l) => in_expr(&l.repeat_expr, out),
        PrecResInnerStmt::ForLoop(f) => in_expr(&f.body, out),
        _ => {}
    }
}

/// Names a state defines: its plain `let`s, whatever its barrier binds, and
/// whatever its synchronous read fetches.
///
/// The read binding matters as much as the others and was missing. A `bram`
/// read binds the MEMORY's output register, and there is one of those per
/// port however many states read it -- so a name still holding it two reads
/// later is not holding its own value any more, it is holding the later
/// read's. One read hid this: nothing overwrote the register, so the name
/// stayed accidentally correct. Two reads and `x + y` became `y + y`.
pub fn defines_of(st: &State) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in st.stmts.iter().chain(st.post.iter()) {
        if let PrecResInnerStmt::VarDecl(d) = stmt {
            // Every name, not just the first: a `let (x, got) = @try_rcv(p)`
            // defines both, and a `got` read in a later state needs the same
            // register `x` does.
            out.extend(d.names().iter().map(|n| anumspan_to_str(n).to_string()));
        }
    }
    if let Some(b) = st.barrier.as_ref().and_then(|b| b.bind.as_ref()) {
        out.insert(b.clone());
    }
    if let Some(read) = &st.mem_read {
        out.insert(read.bind.clone());
    }
    // A `match` arm's pattern binds in the state that decides the arm, and is
    // read in the states that arm scheduled -- so it crosses, like anything
    // else defined in one state and read in another.
    if let Next::Switch { stmt, .. } = &st.next {
        for case in &stmt.cases {
            for pattern in &case.binding_patterns {
                pattern_binds(pattern, &mut out);
            }
        }
    }
    out
}

fn mutable_definitions(st: &State) -> Vec<String> {
    st.stmts.iter().chain(&st.post)
        .filter_map(|s| match s { PrecResInnerStmt::VarDecl(d) => Some(d), _ => None })
        .chain(st.barrier.as_ref().and_then(|b| b.declaration.as_ref()))
        .filter(|d| d.is_mutable)
        .flat_map(|d| d.names().iter().map(|n| anumspan_to_str(n).to_string()))
        .collect()
}

/// Names a pattern brings into scope.
pub fn pattern_binds(pattern: &BindingPattern, out: &mut HashSet<String>) {
    match pattern {
        BindingPattern::Alphanum(n) => {
            out.insert(anumspan_to_str(n).to_string());
        }
        BindingPattern::EnumCase { subbinding, .. } => {
            if let Some(n) = subbinding {
                out.insert(anumspan_to_str(n).to_string());
            }
        }
        BindingPattern::AnyOf(alts) => {
            for alt in alts {
                pattern_binds(alt, out);
            }
        }
    }
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
    if let Next::Switch { stmt, .. } = &st.next {
        for scrutinee in &stmt.scrutinees {
            reads_of(&PrecResInnerStmt::TailVal(scrutinee.clone()), &mut r);
        }
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
    let globals = env.keys().cloned()
        .chain(low.pipes.iter().map(|p| p.name.clone()))
        .chain(low.port_ins.iter().map(|p| p.name.clone()))
        .chain(low.port_outs.iter().map(|p| p.name.clone()))
        .chain(low.syms.funcs.keys().cloned())
        .chain(low.syms.structs.keys().cloned())
        .chain(low.syms.enums.keys().cloned())
        .chain(low.syms.enums.values().flat_map(|e| e.variants.iter().map(|(n, _)| n.clone())))
        .collect();
    let scoped = crate::ir_scope::resolve(body, globals, sink)?;
    low.synthetic_spans = scoped.origins.clone();
    sink.set_synthetic_spans(scoped.origins.clone());
    let body = &scoped.stmts;
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
    // Lifted before scheduling, so the scheduler sees one read per statement
    // however the source spelled them. `hoisted` is borrowed by every state
    // below and must outlive them, which is why it is bound here.
    let hoisted = hoist_sync_reads(body, &sync_mem_of);
    let body = &hoisted.stmts[..];
    // `@rcv`/`@send` reach a `port` by the same names they reach a pipe by, so
    // the scheduler has to be able to tell which it is looking at.
    let port_names: Vec<(String, bool)> = low
        .port_ins
        .iter()
        .map(|p| (p.name.clone(), true))
        .chain(low.port_outs.iter().map(|p| (p.name.clone(), false)))
        .collect();
    let port_of = |n: &str| {
        port_names.iter().position(|(name, _)| name == n).map(|ix| {
            let (_, is_input) = port_names[ix];
            // Re-indexed into whichever list it came from: the two are separate
            // vectors and a `BarrierOn::Port` names a position in one of them.
            let own = if is_input {
                ix
            } else {
                ix - port_names.iter().filter(|(_, i)| *i).count()
            };
            (own, is_input)
        })
    };
    let states_sched = schedule_body(body, &pipe_of, &port_of, &sync_mem_of, sink)?;
    let n_states = states_sched.len();

    // Reject a pipe used in a direction it was not declared for.
    for st in &states_sched {
        let barrier = match &st.barrier {
            Some(b) => b,
            None => continue,
        };
        let Some(pipe_ix) = barrier.pipe() else {
            // A port barrier had its direction settled when the name was
            // resolved, because a port's direction is which list it is in.
            continue;
        };
        let pipe = &low.pipes[pipe_ix];
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
        Next::Switch { targets, .. } => targets.contains(&Target::Halt),
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
        // What says the transfer can happen. On a pipe it is two registers
        // compared -- ours and theirs. On a `port in` it is the enable, which
        // is the only thing a port has to say. On a `port out` it is nothing:
        // there is no `ready` coming back, so a send never waits and the state
        // costs its cycle and no more.
        let handshake = match barrier.on {
            BarrierOn::Pipe(ix) => Some(low.pipes[ix].movable.expect("computed once above")),
            BarrierOn::Port(pix) if barrier.is_recv => Some(low.port_ins[pix].en),
            BarrierOn::Port(_) => None,
        };
        let fire = match handshake {
            None => in_st[k],
            Some(h) => {
                let f = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: in_st[k], rhs: h });
                low.name_value(f, format!("fire_s{}", k));
                f
            }
        };
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
    let mut cross = crossing(&states_sched);
    // Mutable locals have loop-carried storage, not a one-time capture of
    // their initializer. Their lexical identities were resolved above.
    let mutable_locals: HashSet<String> = states_sched.iter().flat_map(mutable_definitions).collect();
    for set in &mut cross { set.retain(|n| !mutable_locals.contains(n)); }
    // A read binding needs saving only where another read can overwrite the
    // port it names. With ONE read state per memory the output register is
    // written only while that state is current, so the name stays good until
    // the machine comes back round to the read that defined it -- and coming
    // back round redefines it. Registering it anyway would cost a real flop
    // per read, and the mux beside it, in every process that reads a `bram`
    // once: the synthesizer cannot remove either, because both are used.
    let mut reads_per_mem = vec![0usize; low.mems.len()];
    for st in &states_sched {
        if let Some(read) = &st.mem_read {
            reads_per_mem[read.mem_ix] += 1;
        }
    }
    for (k, st) in states_sched.iter().enumerate() {
        if let Some(read) = &st.mem_read
            && reads_per_mem[read.mem_ix] == 1
        {
            cross[k].remove(&read.bind);
        }
    }
    let mut slot_of: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut cross_order: Vec<String> = Vec::new();
    // A READ-VALID FLOP per synchronous read whose binding crosses a state.
    //
    // A `bram` read presents its address while its state is current and the
    // array is read at that state's clock edge, so the memory's output
    // register holds the value from the NEXT state on -- not during the state
    // that asked. A crossing register capturing on `fires[k]` therefore
    // captures the previous read.
    //
    // One flop says "the read fired last cycle", which is exactly when the
    // output register is fresh. It is also what lets the name be forwarded:
    // in that one cycle it means the memory's register, and from then on the
    // saved copy. Reserved only where the binding actually crosses, because
    // where it does not the output register is the whole of the answer.
    let mut read_delay_slot: Vec<Option<usize>> = vec![None; n_states];
    let mut read_delay_order: Vec<usize> = Vec::new();
    let mut next_slot = pipe_base + pipe_slots;
    for (k, st) in states_sched.iter().enumerate() {
        let crosses = st
            .mem_read
            .as_ref()
            .is_some_and(|r| cross[k].contains(&r.bind));
        if crosses {
            read_delay_slot[k] = Some(next_slot);
            read_delay_order.push(k);
            next_slot += 1;
        }
    }
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
    // `(name, capture condition, value, type)`. The condition is the state's
    // own firing for everything except a synchronous read, which is a cycle
    // later -- so it is carried rather than re-derived from the state index.
    let mut cross_writes: Vec<(String, ValueId, ValueId, Ty)> = Vec::new();
    // Where each state's branch condition ended up.
    let mut branch_conds: Vec<Option<ValueId>> = vec![None; n_states];
    let mut switch_sel: Vec<Option<SwitchSel>> = vec![None; n_states];
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
    // Allocated after fixed crossing slots once each declaration's type has
    // been inferred. Storage is reset deterministically, but its initializer
    // is still an ordinary statement executed on every dynamic scope entry.
    let mut locals: Vec<LocalReg> = Vec::new();

    // A `port out` is per-state exactly as a pipe's offer is: cleared at the
    // top of every state, and whatever a state left on it belongs to that
    // state. What comes out at the end is a driver rather than a register,
    // because a port has no entries to push into -- the wire IS the offer.
    //
    // Per port: which state offered, what it offered, and on which branch.
    let mut port_sends: Vec<Vec<(usize, ValueId, Option<ValueId>)>> =
        vec![Vec::new(); low.port_outs.len()];

    // A memory's write port is per-state exactly as a `var` is: the address
    // and data a state computed take effect only when that state fires.
    // Without the reset between states, a write in one state would be the
    // write port's value in every state, and a scratchpad updated once per
    // item would be rewritten every cycle.
    // Per memory: which states wrote which of its ports, and with what.
    let mut mem_writes: Vec<Vec<(usize, usize, crate::ir::WritePort)>> =
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
            let f = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: fires[k], rhs: handshake });
            low.name_value_safe(f, format!("{}_xfer_s{}", pipe.name, k));
            low.pipes[ix].fired = Some(f);
            // A barrier already spends this state's one transfer on that pipe,
            // so anything else consuming from it here would be a second
            // transfer in a cycle that has one. Marking it used is what makes
            // `@drop(p)` beside `@rcv(p)` an error instead of a no-op.
            low.pipes[ix].used = st.barrier.as_ref().is_some_and(|b| b.pipe() == Some(ix));
            low.pipes[ix].sent = None;
            low.pipes[ix].send_guard = None;
        }

        for (ix, name) in reg_names.iter().enumerate() {
            if let (Some(v), Some(b)) = (var_start[ix], env.get_mut(name)) {
                b.value = Some(v);
            }
        }
        for local in &locals {
            env.insert(local.name.clone(), Binding::variable(local.held, local.ty.clone()));
        }
        for port in low.port_outs.iter_mut() {
            port.sent = None;
            port.send_guard = None;
        }
        // Every state starts its write ports over. A port a state took would
        // otherwise still be in the environment for the next one, and a
        // scratchpad written once per item would be rewritten every cycle.
        low.clear_write_slots(&mut env);

        let assertion_start = low.asserts.len();
        for stmt in &st.stmts {
            crate::ir::lower_stmt_pub(&mut low, stmt, &mut env, sink)?;
        }
        // Capture a send's operand before post-barrier assignments mutate its
        // environment. The ValueId, unlike a source expression, is immutable.
        let barrier_value = match st.barrier.as_ref().and_then(|b| b.value.as_ref()) {
            Some(expr) => Some(crate::ir::lower_expr(&mut low, expr, &env, sink)?),
            None => None,
        };
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
            let q = low.emit(elem.clone(), Op::MemReadReg { mem: read.mem_ix as u32, port: 0 });
            env.insert(read.bind.clone(), Binding::constant(q, elem));
        }
        if let Some(bind) = st.barrier.as_ref().and_then(|b| b.bind.as_ref()) {
            let barrier = st.barrier.as_ref().expect("a bind implies a barrier");
            let (data, ty) = match barrier.on {
                BarrierOn::Pipe(ix) => {
                    let pipe = low.pipes[ix].clone();
                    (pipe.item.expect("an input pipe has an item"), pipe.ty)
                }
                // One entry, read straight off the wire: a `port` has no slot
                // to skid into, so there is no pair to select from.
                BarrierOn::Port(pix) => {
                    let port = low.port_ins[pix].clone();
                    (port.data, port.ty)
                }
            };
            let declaration = barrier.declaration.as_ref().expect("a receive has a declaration");
            if let Some(t) = &declaration.ty_expr {
                let want = match crate::ty::resolve_type_expr(t, low.syms) {
                    Ok(ty) => ty,
                    Err(e) => {
                        sink.err_at(&declaration.head_name(), e.message());
                        return None;
                    }
                };
                if want != ty {
                    sink.err_at(&declaration.head_name(), format!(
                        "`{}` is declared `{}` but the pipe carries `{}`", bind, want.display(), ty.display(),
                    ));
                    return None;
                }
            }
            env.insert(bind.clone(), if declaration.is_mutable {
                Binding::variable(data, ty)
            } else { Binding::constant(data, ty) });
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
                        "an `if` condition must be `u1`, found `{}`",
                        low.ty_of(v).display()
                    ),
                );
                return None;
            }
            low.name_value(v, format!("branch_s{}", k));
            branch_conds[k] = Some(v);
        }
        if let Next::Switch { stmt, .. } = &st.next {
            let scrutinee = crate::ir::lower_expr(&mut low, &stmt.scrutinees[0], &env, sink)?;
            let scrutinee_ty = low.ty_of(scrutinee);
            let shape = crate::ir_match::plan_match(&mut low, stmt, &scrutinee_ty, sink)?;
            let tag = crate::ir_match::match_tag(&mut low, &shape, scrutinee);
            low.name_value_safe(tag, format!("sel_s{}", k));

            // What each arm's pattern binds.
            //
            // A name can be bound by more than one arm -- `.Read a` and
            // `.Write a`. Each arm of a COMBINATIONAL `match` keeps its own
            // environment, so there one name can be a different type on each.
            // Here the arms are states and the name is one wire that crosses
            // them, so it is one type; and once the types agree the value is
            // the same slice of the same scrutinee, so there is nothing to
            // select between.
            let mut bound: Vec<(String, Vec<Option<AlphanumSpan>>)> = Vec::new();
            for plan in shape.cases.iter().filter(|c| !c.is_unreachable) {
                let mut note = |name: String, from: Option<AlphanumSpan>| {
                    match bound.iter_mut().find(|(n, _)| *n == name) {
                        Some((_, froms)) => froms.push(from),
                        None => bound.push((name, vec![from])),
                    }
                };
                if let Some(name) = plan.catch_all {
                    note(anumspan_to_str(&name).to_string(), None);
                }
                if let Some((variant, bind)) = plan.payload {
                    note(anumspan_to_str(&bind).to_string(), Some(variant));
                }
            }
            for (name, froms) in bound {
                let ty_of = |low: &Lowerer, from: &Option<AlphanumSpan>| match from {
                    None => scrutinee_ty.clone(),
                    Some(v) => low
                        .syms
                        .enums
                        .get(&shape.enum_name)
                        .and_then(|d| d.payload_of(anumspan_to_str(v)))
                        .cloned()
                        .expect("plan_match checked this variant carries a payload"),
                };
                let first = &froms[0];
                let want = ty_of(&low, first);
                for other in &froms[1..] {
                    let have = ty_of(&low, other);
                    if have != want {
                        sink.push(
                            crate::diag::Diag::error(
                                shape.span,
                                format!(
                                    "`{}` binds `{}` on one arm and `{}` on another",
                                    name,
                                    want.display(),
                                    have.display()
                                ),
                            )
                            .with_note(
                                "an arm that waits is a state, and a name that crosses one is a single wire; give the two payloads different names",
                            ),
                        );
                        return None;
                    }
                }
                let value = match first {
                    None => scrutinee,
                    Some(variant) => {
                        let (v, _) =
                            crate::ir_match::payload_value(&mut low, &shape, scrutinee, variant)?;
                        v
                    }
                };
                low.name_value_safe(value, name.clone());
                env.insert(name, crate::ir::Binding::constant(value, want));
            }

            let labels = shape
                .cases
                .iter()
                .map(|c| if c.is_unreachable { None } else { Some(c.labels.clone()) })
                .collect();
            switch_sel[k] = Some((tag, labels));
        }

        // A non-blocking operation asks for the handshake in this state
        // without making the state wait for it, so the state contributes to
        // the pipe's `ready`/`valid` exactly as a barrier state does -- and
        // does NOT contribute to `fires`, which is what "does not wait" means.
        for (ix, uses) in nonblocking.iter_mut().enumerate() {
            let used_here = low.pipes[ix].used;
            let barriered_here = st.barrier.as_ref().is_some_and(|b| b.pipe() == Some(ix));
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

        // The cycle after a synchronous read is when its value is there to
        // capture; everything else is captured in the state that computed it.
        let read_ready = read_delay_slot[k].map(|slot| {
            let d = low.emit(Ty::BOOL, Op::RegRead(slot as u32));
            low.name_value_safe(d, format!("rd_valid_s{}", k));
            d
        });
        let read_bind = st.mem_read.as_ref().map(|r| r.bind.clone());

        // A declaration's value is its dynamic initializer, possibly followed
        // by assignments in this state. Allocate a distinct storage identity
        // even when an outer scope uses the same source spelling.
        for name in mutable_definitions(st) {
            if !locals.iter().any(|l| l.name == name) {
                let binding = env.get(&name)?;
                let ty = binding.ty.clone();
                let held = low.emit(ty.clone(), Op::RegRead(next_slot as u32));
                next_slot += 1;
                locals.push(LocalReg { name, ty, held, writes: Vec::new() });
            }
        }
        for local in &mut locals {
            if let Some(v) = env.get(&local.name).and_then(|b| b.value)
                && v != local.held {
                local.writes.push((k, v));
            }
        }

        // What this state leaves behind for the others.
        for name in &cross[k] {
            let capture = match (&read_bind, read_ready) {
                (Some(bind), Some(d)) if bind == name => d,
                _ => fires[k],
            };
            if let Some(b) = env.get(name).cloned()
                && let Some(v) = b.value {
                    cross_writes.push((name.clone(), capture, v, b.ty.clone()));
                }
        }
        for (ix, name) in reg_names.iter().enumerate() {
            let now = env.get(name).and_then(|b| b.value);
            if now != var_start[ix]
                && let Some(v) = now {
                    var_writes[ix].push((k, v));
                }
        }
        for (port, sends) in low.port_outs.iter().zip(port_sends.iter_mut()) {
            if let Some(v) = port.sent {
                sends.push((k, v, port.send_guard));
            }
        }
        for (ix, writes) in mem_writes.iter_mut().enumerate() {
            for slot in 0..low.mem_slots(ix) {
                if let Some(p) = low.write_slot(ix, slot, &env) {
                    writes.push((k, slot, p));
                }
            }
        }

        if let Some(expr) = st.barrier.as_ref().and_then(|b| b.value.as_ref()) {
            let barrier = st.barrier.as_ref().expect("a value implies a barrier");
            let (name, want) = match barrier.on {
                BarrierOn::Pipe(ix) => (low.pipes[ix].name.clone(), low.pipes[ix].ty.clone()),
                BarrierOn::Port(pix) => {
                    (low.port_outs[pix].name.clone(), low.port_outs[pix].ty.clone())
                }
            };
            // The send is not a statement as far as lowering is concerned --
            // the scheduler took it apart -- so its anchor has to be pushed
            // here or the type error lands at line 1.
            let depth = crate::ir::expr_anchor(expr).map(|at| {
                let span = low.span_of(&at);
                low.push_anchor(span)
            });
            let v = barrier_value.expect("a send payload was captured before its post statements");
            let have = low.ty_of(v);
            if have != want {
                sink.err_span(
                    low.here(),
                    format!(
                        "`{}` carries `{}` but `{}` was sent",
                        name,
                        want.display(),
                        have.display()
                    ),
                );
                return None;
            }
            match barrier.on {
                BarrierOn::Pipe(ix) => send_values.push((ix, k, v)),
                // No guard: a scheduled send is the whole of what its state
                // does, so the state firing is the condition and there is no
                // branch inside it to narrow to.
                BarrierOn::Port(pix) => port_sends[pix].push((k, v, None)),
            }
            if let Some(depth) = depth {
                low.pop_anchor(depth);
            }
        }
        // Include checks introduced by inlining in a branch condition or
        // match scrutinee, as well as the state's ordinary statements.
        for ix in assertion_start..low.asserts.len() {
            let cond = low.asserts[ix].cond;
            let inactive = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: fires[k] });
            low.asserts[ix].cond =
                low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: inactive, rhs: cond });
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
                    // A read's copy lands at the end of the cycle the value
                    // appears in, so for that one cycle the name still has to
                    // mean the memory's output register. Forwarding it is what
                    // makes a read usable in the state right after the fetch
                    // AND in every state after that, with one register rather
                    // than a rule about which states may use it.
                    let live = match (&read_bind, read_ready) {
                        (Some(bind), Some(d)) if bind == name => {
                            let fresh = env
                                .get(name)
                                .and_then(|b| b.value)
                                .expect("a read binding has a value");
                            let m = low.emit(
                                ty.clone(),
                                Op::Mux { cond: d, then_val: fresh, else_val: r },
                            );
                            low.name_value_safe(m, format!("{}_live", name));
                            m
                        }
                        _ => r,
                    };
                    let stays_mutable = env.get(name).is_some_and(|b| b.is_mutable);
                    env.insert(
                        name.clone(),
                        Binding { value: Some(live), ty, is_output: false, is_mutable: stays_mutable },
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
            .filter(|(_, s)| s.barrier.as_ref().is_some_and(|b| b.pipe() == Some(ix)))
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
            let movable = low.pipes[ix].movable.expect("computed before lowering states");
            let transfer = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: ask, rhs: movable });
            asks.push(transfer);
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
    // SEVERAL STATES WRITING ONE PORT IS STILL ONE PORT: only one state is
    // active in any cycle, so they mux onto it. What decides how many ports a
    // memory has is how many writes one state performs -- two writes in a row
    // both happen in that cycle and there is nothing to mux them onto.
    for (ix, writes) in mem_writes.iter().enumerate() {
        let writes = writes.clone();
        let ports = writes.iter().map(|(_, slot, _)| slot + 1).max().unwrap_or(0);
        let mut settled: Vec<crate::ir::WritePort> = Vec::with_capacity(ports);
        for slot in 0..ports {
            let idle = low.idle_write(ix);
            let mut we: Option<ValueId> = None;
            let mut addr = idle.addr;
            let mut data = idle.data;
            for (k, _, p) in writes.iter().filter(|(_, s, _)| *s == slot) {
                let gated =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: fires[*k], rhs: p.we });
                we = Some(match we {
                    None => gated,
                    Some(prev) => {
                        low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: prev, rhs: gated })
                    }
                });
                let addr_ty = low.ty_of(addr);
                addr = low.emit(
                    addr_ty,
                    Op::Mux { cond: gated, then_val: p.addr, else_val: addr },
                );
                let data_ty = low.ty_of(data);
                data = low.emit(
                    data_ty,
                    Op::Mux { cond: gated, then_val: p.data, else_val: data },
                );
            }
            let we = we.expect("a slot in range has at least one writer");
            // With one writer the mux selector IS the write enable, so the mux
            // is redundant: address and data are don't-cares where it is low.
            settled.push(crate::ir::WritePort {
                we,
                addr: low.drop_gated_mux(addr, we),
                data: low.drop_gated_mux(data, we),
            });
        }
        low.mems[ix].write = settled;
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
            // A `case` over the tag, not a chain of comparisons. The same
            // measurement that made a combinational `match` a `case` applies
            // to the next-state logic: a ternary chain is a PRIORITY structure
            // and synthesis has to honour the priority, where a `case` says
            // the arms are parallel.
            Next::Switch { targets, .. } => {
                let (tag, labels) =
                    switch_sel[k].clone().expect("a switching state resolved its patterns");
                // Selectable arms, in source order. The last one is the
                // default -- either it is the catch-all, or coverage is
                // complete and it is reached by elimination. Exactly the rule
                // the combinational `match` follows.
                let selectable: Vec<(Vec<u128>, Target)> = labels
                    .iter()
                    .zip(targets.iter())
                    .filter_map(|(l, t)| l.clone().map(|l| (l, *t)))
                    .collect();
                let (_, default_t) =
                    selectable.last().expect("plan_match rejects a match with no arms");
                let default = target_of(&mut low, *default_t);
                let mut arms: Vec<(Vec<u128>, ValueId)> = Vec::new();
                for (l, t) in &selectable[..selectable.len() - 1] {
                    if l.is_empty() {
                        continue;
                    }
                    let v = target_of(&mut low, *t);
                    arms.push((l.clone(), v));
                }
                if arms.is_empty() {
                    default
                } else {
                    low.emit(st_ty.clone(), Op::Case { scrutinee: tag, arms, default })
                }
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

    // In slot order, immediately after the pipe registers, which is where they
    // were reserved.
    for k in &read_delay_order {
        generated.push(Reg {
            name: format!("rd_valid_s{}", k),
            ty: Ty::BOOL,
            reset: 0,
            next: fires[*k],
        });
    }

    for name in &cross_order {
        let writes: Vec<(ValueId, ValueId, Ty)> = cross_writes
            .iter()
            .filter(|(n, _, _, _)| n == name)
            .map(|(_, cond, v, t)| (*cond, *v, t.clone()))
            .collect();
        let ty = match writes.first() {
            Some((_, _, t)) => t.clone(),
            None => continue,
        };
        let slot = slot_of[name];
        let mut next = low.emit(ty.clone(), Op::RegRead(slot as u32));
        // Later writers are muxed outermost, but only one state is ever
        // active, so the order between them does not matter.
        for (cond, v, _) in &writes {
            next = low.emit(
                ty.clone(),
                Op::Mux { cond: *cond, then_val: *v, else_val: next },
            );
        }
        generated.push(Reg { name: format!("{}_r", name), ty, reset: 0, next });
    }

    // Locals occupy the slots reserved after the crossing registers. Their
    // values are committed on execution, and held on stalls and other paths.
    for local in locals {
        let mut next = local.held;
        for (k, v) in local.writes {
            next = low.emit(local.ty.clone(), Op::Mux {
                cond: fires[k], then_val: v, else_val: next,
            });
        }
        generated.push(Reg { name: local.name, ty: local.ty, reset: 0, next });
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
        low.mems[ix].read = vec![crate::ir::ReadPort { addr, en }];
    }

    for (port, sends) in low.port_outs.clone().into_iter().zip(port_sends.clone()) {
        // A port nothing sent to still drives: zero, with the enable low. An
        // output left undriven would be a floating wire.
        let mut value = low.emit(port.ty.clone(), Op::Const(0));
        let mut enable = low.emit(Ty::BOOL, Op::Const(0));
        for (k, v, guard) in &sends {
            // FIRING, not merely being in the state. A state with a barrier
            // does its work in the cycle that barrier completes, so a port it
            // sends to is sent to then and not while it is still waiting --
            // and a send written inside an `if` happens on that branch only.
            let asks = match guard {
                None => fires[*k],
                Some(g) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: fires[*k], rhs: *g }),
            };
            value = low.emit(
                port.ty.clone(),
                Op::Mux { cond: asks, then_val: *v, else_val: value },
            );
            enable = low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: enable, rhs: asks });
        }
        low.name_value_safe(value, port.name.clone());
        low.name_value_safe(enable, format!("{}_en", port.name));
        drivers.push((port.data_port, value));
        drivers.push((port.en_port, enable));
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
