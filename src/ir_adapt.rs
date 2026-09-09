// Boundary adapters: the salt protocol on one side, an ordinary FIFO on the
// other.
//
// The salt protocol is how two things DDL compiled talk to each other, and it
// earns its keep there -- a `wsalt` is a register, so no transfer depends
// combinationally on the answer coming back, and the two-entry skid is what
// removes the consumer's back-pressure from the producer's timing path.
//
// It is a bad thing to hand a person. Writing the far side by hand means
// knowing that the read index is the xor of the two salt bits, that the gray
// step toggles one of them, that full is `wsalt == ~rsalt`, and that the data
// port carries both entries at once. None of that is in the emitted file.
//
// So it does not cross a boundary. Every module a person writes Verilog
// against -- an `extern`, or an export target -- gets a plain FIFO instead:
//
//   a consumer's side   can_receive / receive_en / data_write_in
//   a producer's side   has_data    / drop_item  / data_read_out
//
// which is `!full`/`wr_en`/`wr_data` and `!empty`/`rd_en`/`rd_data` with
// first-word-fall-through, and nothing to look up.
//
// These are modules the compiler writes, in the same sense `@merge` and
// `@split` are, keyed by shape so one serves every use of a width. Two logic
// cores: the READ core drains a salt pipe and offers what it drained, the
// WRITE core accepts an item and pushes it onto one. Each core appears twice,
// once as a slave -- the far side says when -- and once as a master, where
// this side computes the enable from the far side's flag. That difference is
// one AND gate.
//
// Both keep the rules `ir_comb.rs` states. A `wsalt` is a register, so
// `can_receive` is a function of `wsalt_q` and the incoming `rsalt` and never
// of anything combinational on the far side. And nothing is dropped: an item
// moves only when the side receiving it has said it has room.

use crate::diag::{DiagSink, SourceMap};
use crate::ir::{BinOp, Lowerer, Module, Op, PortDir, Reg, SALT, UnOp, ValueId};
use crate::ir_comb::{out_side, push_out};
use crate::symbols::Symbols;
use crate::ty::Ty;

/// Which way an adapter faces, and who computes the enable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adapt {
    /// Drains a salt pipe and offers what it drained as a read port. The far
    /// side raises `drop_item` when it takes one. Sits at an export target's
    /// `buffer out`.
    SaltToRport,
    /// Drains a salt pipe and writes it into a foreign module's write port,
    /// raising `receive_en` whenever that module says it can receive. Sits at
    /// an `extern`'s `buffer in`.
    SaltToWport,
    /// Accepts writes on a write port and pushes them onto a salt pipe. Sits
    /// at an export target's `buffer in`.
    WportToSalt,
    /// Reads a foreign module's read port and pushes what it reads onto a salt
    /// pipe, raising `drop_item` whenever it has room. Sits at an `extern`'s
    /// `buffer out`.
    RportToSalt,
}

/// A consumer's face: someone outside writes items in.
pub const WRITE_FACE: [&str; 3] = ["can_receive", "receive_en", "data_write_in"];
/// A producer's face: someone outside reads items out.
pub const READ_FACE: [&str; 3] = ["has_data", "drop_item", "data_read_out"];

impl Adapt {
    /// Which adapter belongs at one pipe of a module a person will wire up.
    ///
    /// `is_input` is the pipe's direction ON THAT MODULE, and `foreign` says
    /// whether the module is one DDL did not compile. An `extern` is driven BY
    /// the graph, so the adapter is the master there; an export target
    /// presents its face outward and lets whoever instantiates it decide.
    pub fn at(is_input: bool, foreign: bool) -> Adapt {
        match (is_input, foreign) {
            (true, true) => Adapt::SaltToWport,
            (false, true) => Adapt::RportToSalt,
            (true, false) => Adapt::WportToSalt,
            (false, false) => Adapt::SaltToRport,
        }
    }

    /// The three signals of the FIFO face, in declaration order.
    pub fn face(&self) -> [&'static str; 3] {
        match self {
            Adapt::SaltToWport | Adapt::WportToSalt => WRITE_FACE,
            Adapt::SaltToRport | Adapt::RportToSalt => READ_FACE,
        }
    }

    /// The name of the adapter's own salt pipe.
    pub fn salt_pipe(&self) -> &'static str {
        match self {
            Adapt::SaltToRport | Adapt::SaltToWport => "i",
            Adapt::WportToSalt | Adapt::RportToSalt => "o",
        }
    }

    fn what(&self) -> &'static str {
        match self {
            Adapt::SaltToRport => "salt_to_rport",
            Adapt::SaltToWport => "salt_to_wport",
            Adapt::WportToSalt => "wport_to_salt",
            Adapt::RportToSalt => "rport_to_salt",
        }
    }
}

/// One adapter a boundary asked for: which way, and carrying what.
///
/// Keyed by shape rather than by use, exactly as `CombUse` is: two boundaries
/// of the same width and direction are one module instantiated twice.
#[derive(Debug, Clone, PartialEq)]
pub struct AdaptUse {
    pub kind: Adapt,
    pub ty: Ty,
}

/// The module name for one shape.
///
/// The width is the payload's and not the pair's: a FIFO face carries one item
/// at a time, which is the whole point of it.
pub fn module_name(kind: Adapt, ty: &Ty) -> String {
    format!("ddl_{}_{}", kind.what(), ty.bit_width())
}

/// Builds one adapter module.
pub fn build(
    map: &SourceMap,
    syms: &Symbols,
    kind: Adapt,
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

    let reads_salt = matches!(kind, Adapt::SaltToRport | Adapt::SaltToWport);
    low.declare_pipe(kind.salt_pipe().to_string(), ty.clone(), reads_salt);

    // The FIFO face. The flag always points away from the side holding the
    // storage, and the `go` always points into it.
    let [flag, go, data] = kind.face();
    let (flag_dir, go_dir, data_dir) = match kind {
        // Slave: this module holds the storage, so it answers the flag and is
        // told when.
        Adapt::SaltToRport => (PortDir::Out, PortDir::In, PortDir::Out),
        Adapt::WportToSalt => (PortDir::Out, PortDir::In, PortDir::In),
        // Master: the foreign module holds the storage, so it answers the flag
        // and this side decides when.
        Adapt::SaltToWport => (PortDir::In, PortDir::Out, PortDir::Out),
        Adapt::RportToSalt => (PortDir::In, PortDir::Out, PortDir::In),
    };
    let flag_port = low.declare_raw_port(flag.to_string(), flag_dir, Ty::BOOL);
    let go_port = low.declare_raw_port(go.to_string(), go_dir, Ty::BOOL);
    let data_port = low.declare_raw_port(data.to_string(), data_dir, ty.clone());

    let mut regs: Vec<(usize, String, Ty, ValueId)> = Vec::new();
    let mut drivers: Vec<(crate::ir::PortId, ValueId)> = Vec::new();
    let slots;

    if reads_salt {
        // ---- the READ core ------------------------------------------------
        // One `rsalt` register. What it offers is what the producer committed
        // on an earlier cycle, so the item is stable for as long as it is
        // offered and the far side may take as long as it likes over it.
        slots = 1;
        let rsalt_q = low.emit(SALT, Op::RegRead(0));
        low.name_value_safe(rsalt_q, "rsalt_q".to_string());
        let empty = low.pipe_empty(0, rsalt_q);
        let has_item = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: empty });
        low.name_value_safe(has_item, "has_item".to_string());
        let item = low.pipe_item_at(0, rsalt_q);
        let ridx = low.pipes[0].idx.expect("pipe_item_at records the index");

        let take = match kind {
            // Slave. The AND with `has_item` is kept rather than trusted away:
            // a far side that raised `drop_item` on an empty cycle would
            // otherwise walk the read pointer past the write pointer, and the
            // pipe would report full for the rest of time.
            Adapt::SaltToRport => {
                let dropped = low.emit(Ty::BOOL, Op::Port(go_port));
                low.name_value_safe(dropped, "drop_item_in".to_string());
                low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: has_item, rhs: dropped })
            }
            // Master: write whenever there is something to write and the far
            // side says it can receive.
            _ => {
                let can = low.emit(Ty::BOOL, Op::Port(flag_port));
                low.name_value_safe(can, "can_receive_in".to_string());
                low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: has_item, rhs: can })
            }
        };
        low.name_value_safe(take, "take".to_string());

        drivers.push((data_port, item));
        match kind {
            Adapt::SaltToRport => drivers.push((flag_port, has_item)),
            _ => drivers.push((go_port, take)),
        }
        let next = low.salt_next(rsalt_q, ridx, take);
        regs.push((0, "rsalt_q".to_string(), SALT, next));
        drivers.push((low.pipes[0].rsalt_port, rsalt_q));
    } else {
        // ---- the WRITE core -----------------------------------------------
        // Two entries and a `wsalt`, the same shape a process's output has:
        // the consumer reads an entry on a cycle this side may already have
        // left, so what it reads has to be a register rather than a wire off
        // this side's current state.
        slots = 3;
        let side = out_side(&mut low, 0, 0);
        let room = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: side.full });
        low.name_value_safe(room, "room".to_string());
        let item = low.emit(ty.clone(), Op::Port(data_port));
        low.name_value_safe(item, "item".to_string());

        let push = match kind {
            // Slave: the far side raises `receive_en`, gated by the `room`
            // this module already published as `can_receive`. Gated again for
            // the same reason the read core gates `drop_item`.
            Adapt::WportToSalt => {
                let en = low.emit(Ty::BOOL, Op::Port(go_port));
                low.name_value_safe(en, "receive_en_in".to_string());
                low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: en, rhs: room })
            }
            // Master: take one whenever the far side has one and there is room.
            _ => {
                let has = low.emit(Ty::BOOL, Op::Port(flag_port));
                low.name_value_safe(has, "has_data_in".to_string());
                low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: has, rhs: room })
            }
        };
        low.name_value_safe(push, "push".to_string());

        match kind {
            Adapt::WportToSalt => drivers.push((flag_port, room)),
            _ => drivers.push((go_port, push)),
        }
        push_out(&mut low, 0, &side, item, push, &mut regs, &mut drivers);
    }

    if sink.errored_since(errors_before) {
        return None;
    }

    regs.sort_by_key(|(slot, _, _, _)| *slot);
    debug_assert_eq!(regs.len(), slots, "every reserved slot has a next value");
    let regs: Vec<Reg> = regs
        .into_iter()
        .map(|(_, name, ty, next)| Reg { name, ty, reset: 0, next })
        .collect();

    let (values, ports) = low.take_values();
    Some(Module {
        calls: Vec::new(),
        name: module_name(kind, ty),
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
