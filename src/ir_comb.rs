// Pipe combinators: modules the compiler writes rather than the program.
//
// `@merge` and `@split` are structural. Writing either as a `process` works
// and costs a cycle per hop, because a process is a state machine and a state
// is a cycle -- which for something whose whole job is to pass an item along
// is the wrong price. These build the datapath directly instead: one transfer
// per cycle through either, and no state beyond the entries the protocol
// already requires.
//
// Both obey the two rules everything else does. A `wsalt` is a register, so it
// never depends combinationally on the `rsalt` coming back -- the property
// k2g_chan.sv:26 rule 3 exists for. And nothing is ever dropped: a `@split`
// takes from its input only when EVERY sink has room, and a `@merge` grants
// one input at a time and leaves the others where they are.
//
// The alternative to a `@split` is ANDing the sinks' readys together, which
// rebuilds exactly the combinational coupling the salt protocol removed. It
// costs a slot per sink instead, which is what `port-k2g.md` predicted it
// would.

use crate::diag::{DiagSink, SourceMap};
use crate::ir::{BinOp, Lowerer, Module, Op, Reg, SALT, UnOp, ValueId};
use crate::symbols::Symbols;
use crate::ty::Ty;

/// Which combinator a graph asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comb {
    /// `@merge(a, b, .., out)` -- several producers onto one pipe, one at a
    /// time, in rotation.
    Merge,
    /// `@split(in, a, b, ..)` -- one producer to several consumers, each
    /// getting its own copy.
    Split,
}

impl Comb {
    pub fn of(name: &str) -> Option<Comb> {
        match name {
            "@merge" => Some(Comb::Merge),
            "@split" => Some(Comb::Split),
            _ => None,
        }
    }

    pub fn spelling(&self) -> &'static str {
        match self {
            Comb::Merge => "@merge",
            Comb::Split => "@split",
        }
    }

    /// How many of the arguments are on the many-side.
    pub fn fan(&self, args: usize) -> usize {
        args - 1
    }
}

/// The module name for one instantiation.
///
/// Keyed by shape rather than by use, so two merges of the same arity and
/// payload are one module instantiated twice.
pub fn module_name(kind: Comb, fan: usize, ty: &Ty) -> String {
    let what = match kind {
        Comb::Merge => "merge",
        Comb::Split => "split",
    };
    format!("ddl_{}_{}x{}", what, fan, ty.bit_width())
}

/// The port names of a combinator, in the order a graph passes them.
///
/// `@merge(a, b, out)` is the many side first and the one side last; `@split`
/// is the other way round. Both read as "from, to".
pub fn pipe_names(kind: Comb, fan: usize) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    match kind {
        Comb::Merge => {
            for k in 0..fan {
                out.push((format!("i{}", k), true));
            }
            out.push(("o".to_string(), false));
        }
        Comb::Split => {
            out.push(("i".to_string(), true));
            for k in 0..fan {
                out.push((format!("o{}", k), false));
            }
        }
    }
    out
}

/// ORs a list; false when empty.
pub(crate) fn any_of(low: &mut Lowerer, conds: &[ValueId]) -> ValueId {
    let mut acc: Option<ValueId> = None;
    for c in conds {
        acc = Some(match acc {
            None => *c,
            Some(prev) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: prev, rhs: *c }),
        });
    }
    acc.unwrap_or_else(|| low.emit(Ty::BOOL, Op::Const(0)))
}

/// ANDs a list; true when empty.
pub(crate) fn all_of(low: &mut Lowerer, conds: &[ValueId]) -> ValueId {
    let mut acc: Option<ValueId> = None;
    for c in conds {
        acc = Some(match acc {
            None => *c,
            Some(prev) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: prev, rhs: *c }),
        });
    }
    acc.unwrap_or_else(|| low.emit(Ty::BOOL, Op::Const(1)))
}

/// The producer half of one output pipe: two entries and a `wsalt`.
///
/// The same shape `ir_fsm` gives a process's output, and for the same reason:
/// the consumer reads an entry on a cycle this side may already have left, so
/// what it reads has to be a register rather than a wire off this side's
/// current state.
pub(crate) struct OutSide {
    pub(crate) e0: ValueId,
    pub(crate) e1: ValueId,
    pub(crate) wsalt_q: ValueId,
    pub(crate) widx: ValueId,
    pub(crate) full: ValueId,
    pub(crate) base: usize,
}

pub(crate) fn out_side(low: &mut Lowerer, ix: usize, base: usize) -> OutSide {
    let ty = low.pipes[ix].ty.clone();
    let name = low.pipes[ix].name.clone();
    let e0 = low.emit(ty.clone(), Op::RegRead(base as u32));
    low.name_value_safe(e0, format!("{}_e0", name));
    let e1 = low.emit(ty.clone(), Op::RegRead((base + 1) as u32));
    low.name_value_safe(e1, format!("{}_e1", name));
    let wsalt_q = low.emit(SALT, Op::RegRead((base + 2) as u32));
    low.name_value_safe(wsalt_q, format!("{}_wsalt_q", name));
    let widx = low.salt_idx(wsalt_q, format!("{}_widx", name));
    let full = low.pipe_full(ix, wsalt_q);
    OutSide { e0, e1, wsalt_q, widx, full, base }
}

/// Pushes `item` into an output when `push` holds, and advances its salt.
pub(crate) fn push_out(
    low: &mut Lowerer,
    ix: usize,
    side: &OutSide,
    item: ValueId,
    push: ValueId,
    regs: &mut Vec<(usize, String, Ty, ValueId)>,
    drivers: &mut Vec<(crate::ir::PortId, ValueId)>,
) {
    let ty = low.pipes[ix].ty.clone();
    let name = low.pipes[ix].name.clone();
    let not_widx = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: side.widx });
    let to_e0 = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: push, rhs: not_widx });
    let to_e1 = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: push, rhs: side.widx });
    let e0_next =
        low.emit(ty.clone(), Op::Mux { cond: to_e0, then_val: item, else_val: side.e0 });
    let e1_next =
        low.emit(ty.clone(), Op::Mux { cond: to_e1, then_val: item, else_val: side.e1 });
    let salt_next = low.salt_next(side.wsalt_q, side.widx, push);

    regs.push((side.base, format!("{}_e0", name), ty.clone(), e0_next));
    regs.push((side.base + 1, format!("{}_e1", name), ty, e1_next));
    regs.push((side.base + 2, format!("{}_wsalt_q", name), SALT, salt_next));

    let pair = low.pack_entries(side.e0, side.e1, &low.pipes[ix].ty.clone());
    drivers.push((low.pipes[ix].data_port, pair));
    drivers.push((low.pipes[ix].wsalt_port, side.wsalt_q));
}

/// Builds one combinator module.
pub fn build(
    map: &SourceMap,
    syms: &Symbols,
    kind: Comb,
    fan: usize,
    ty: &Ty,
    sink: &mut DiagSink,
) -> Option<Module> {
    // Errors from EARLIER declarations are not this one's failure: the sink
    // is shared by the whole compilation, so `has_errors` would make every
    // declaration after the first bad one return `None` without a reason.
    let errors_before = sink.error_mark();
    let bodies = std::collections::HashMap::new();
    let mut low = Lowerer::new(map, syms, &bodies);

    let mut env = crate::ir::Env::new();
    low.declare_clock(&mut env);

    for (name, is_input) in pipe_names(kind, fan) {
        low.declare_pipe(name, ty.clone(), is_input);
    }

    // Slot layout, fixed before anything reads a register: an `rsalt` for each
    // input, then three slots for each output.
    let mut slot = 0usize;
    let mut in_slots: Vec<usize> = Vec::new();
    let mut out_slots: Vec<usize> = Vec::new();
    for ix in 0..low.pipes.len() {
        if low.pipes[ix].is_input {
            in_slots.push(slot);
            slot += 1;
        } else {
            out_slots.push(slot);
            slot += 3;
        }
    }

    let mut regs: Vec<(usize, String, Ty, ValueId)> = Vec::new();
    let mut drivers: Vec<(crate::ir::PortId, ValueId)> = Vec::new();

    match kind {
        Comb::Split => {
            let in_ix = 0;
            let rsalt_slot = in_slots[0];
            let rsalt_q = low.emit(SALT, Op::RegRead(rsalt_slot as u32));
            low.name_value_safe(rsalt_q, "i_rsalt_q".to_string());
            let empty = low.pipe_empty(in_ix, rsalt_q);
            let has_item = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: empty });
            let item = low.pipe_item_at(in_ix, rsalt_q);
            let ridx = low.pipes[in_ix].idx.expect("pipe_item_at records the index");

            let sides: Vec<OutSide> = (0..fan)
                .map(|k| out_side(&mut low, 1 + k, out_slots[k]))
                .collect();
            // Every sink, or none. ANDing the sinks' READYS is what this
            // avoids -- that would put each consumer's logic in every other
            // one's timing path. Each has its own slot, and the input waits
            // for the slowest of them.
            let rooms: Vec<ValueId> = sides
                .iter()
                .map(|s| low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: s.full }))
                .collect();
            let all_room = all_of(&mut low, &rooms);
            low.name_value_safe(all_room, "all_room".to_string());
            let take = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: has_item, rhs: all_room });
            low.name_value_safe(take, "take".to_string());

            for (k, side) in sides.iter().enumerate() {
                push_out(&mut low, 1 + k, side, item, take, &mut regs, &mut drivers);
            }

            let next = low.salt_next(rsalt_q, ridx, take);
            regs.push((rsalt_slot, "i_rsalt_q".to_string(), SALT, next));
            drivers.push((low.pipes[in_ix].rsalt_port, rsalt_q));
        }

        Comb::Merge => {
            let out_ix = fan;
            let side = out_side(&mut low, out_ix, out_slots[0]);
            let room = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: side.full });
            low.name_value_safe(room, "room".to_string());

            let mut rsalts: Vec<ValueId> = Vec::new();
            let mut items: Vec<ValueId> = Vec::new();
            let mut ridxs: Vec<ValueId> = Vec::new();
            let mut offered: Vec<ValueId> = Vec::new();
            for (k, in_slot) in in_slots.iter().enumerate().take(fan) {
                let rsalt_q = low.emit(SALT, Op::RegRead(*in_slot as u32));
                low.name_value_safe(rsalt_q, format!("i{}_rsalt_q", k));
                let empty = low.pipe_empty(k, rsalt_q);
                let has = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: empty });
                low.name_value_safe(has, format!("i{}_offered", k));
                let item = low.pipe_item_at(k, rsalt_q);
                let ridx = low.pipes[k].idx.expect("pipe_item_at records the index");
                rsalts.push(rsalt_q);
                items.push(item);
                ridxs.push(ridx);
                offered.push(has);
            }

            // ROTATING PRIORITY. `turn` names the input that goes first this
            // cycle; the rest follow in order after it. A fixed priority would
            // be smaller and would starve input 1 whenever input 0 is busy,
            // which for two requesters sharing a bus is the bug the arbiter
            // exists to not have.
            //
            // Unrolled over `turn` because `fan` is known here: n candidate
            // orders, each a plain priority chain, selected by a comparison.
            // O(n^2) for an n that is 2 or 3.
            let turn_w = crate::ty::bits_for(fan as u128 - 1).max(1);
            let turn_ty = Ty::UInt(turn_w);
            let turn_slot = slot;
            slot += 1;
            let turn = low.emit(turn_ty.clone(), Op::RegRead(turn_slot as u32));
            low.name_value_safe(turn, "turn".to_string());

            let mut grants: Vec<ValueId> = Vec::with_capacity(fan);
            for k in 0..fan {
                // Granted under start `s` when nothing ahead of it is offering.
                let mut per_start: Vec<ValueId> = Vec::new();
                for s in 0..fan {
                    let position = (k + fan - s) % fan;
                    let ahead: Vec<ValueId> = (0..fan)
                        .filter(|j| (*j + fan - s) % fan < position)
                        .map(|j| offered[j])
                        .collect();
                    // The input that goes FIRST under this start has nothing
                    // ahead of it, so there is nothing to test. Emitting the
                    // negation of a constant instead would put `(!1'b0)` in
                    // the middle of the arbiter for a reader to decode.
                    let clear = if ahead.is_empty() {
                        low.emit(Ty::BOOL, Op::Const(1))
                    } else {
                        let blocked = any_of(&mut low, &ahead);
                        low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: blocked })
                    };
                    let is_start = low.emit(turn_ty.clone(), Op::Const(s as u128));
                    let matches = low.emit_eq(turn, is_start);
                    per_start.push(low.emit(
                        Ty::BOOL,
                        Op::Bin { op: BinOp::And, lhs: matches, rhs: clear },
                    ));
                }
                let first = any_of(&mut low, &per_start);
                let g = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: offered[k], rhs: first });
                low.name_value_safe(g, format!("grant{}", k));
                grants.push(g);
            }

            let granted = any_of(&mut low, &grants);
            let push = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: granted, rhs: room });
            low.name_value_safe(push, "push".to_string());

            // The item of whichever input was granted. Only one grant is ever
            // high, so the order of the mux chain does not matter.
            let mut item = items[0];
            for k in 1..fan {
                item = low.emit(
                    ty.clone(),
                    Op::Mux { cond: grants[k], then_val: items[k], else_val: item },
                );
            }
            low.name_value_safe(item, "item".to_string());
            push_out(&mut low, out_ix, &side, item, push, &mut regs, &mut drivers);

            // Each input advances only on its own transfer.
            for k in 0..fan {
                let took = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: grants[k], rhs: push });
                low.name_value_safe(took, format!("take{}", k));
                let next = low.salt_next(rsalts[k], ridxs[k], took);
                regs.push((in_slots[k], format!("i{}_rsalt_q", k), SALT, next));
                drivers.push((low.pipes[k].rsalt_port, rsalts[k]));
            }

            // Whoever went this time goes last next time.
            let mut turn_next = turn;
            for (k, grant) in grants.iter().enumerate() {
                let after = low.emit(turn_ty.clone(), Op::Const(((k + 1) % fan) as u128));
                let took = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: *grant, rhs: room });
                turn_next = low.emit(
                    turn_ty.clone(),
                    Op::Mux { cond: took, then_val: after, else_val: turn_next },
                );
            }
            regs.push((turn_slot, "turn".to_string(), turn_ty, turn_next));
        }
    }

    if sink.errored_since(errors_before) {
        return None;
    }

    regs.sort_by_key(|(slot, _, _, _)| *slot);
    debug_assert_eq!(regs.len(), slot, "every reserved slot has a next value");
    let regs: Vec<Reg> = regs
        .into_iter()
        .map(|(_, name, ty, next)| Reg { name, ty, reset: 0, next })
        .collect();

    let (values, ports) = low.take_values();
    Some(Module {
        calls: Vec::new(),
        name: module_name(kind, fan, ty),
        ports,
        values,
        drivers,
        regs,
        mems: Vec::new(),
        asserts: Vec::new(),
        params: Vec::new(),
        nets: Vec::new(),
        instances: Vec::new(),
    })
}
