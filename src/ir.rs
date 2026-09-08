// The combinational intermediate representation, and lowering into it.
//
// Every value carries a `Ty`, so widths are decided here once and the backend
// never has to guess. That is the property that lets the Verilog emitter fold
// every width to a literal and never emit `$clog2` or a width cast, both of
// which make GowinSynthesis exit 1 with an empty log.
//
// Scope: purely combinational functions. Values are in definition order, which
// is a valid topological order because nothing here can refer forwards. There
// are no registers, no clocks and no control-flow graph -- `process` and
// `sequence` need a real CFG with basic blocks and phi nodes, and that lands
// with the FSM scheduler rather than being bolted onto this.
//
// `if`/`else` IS supported, because nothing real can be written without it. It
// lowers by evaluating both arms into separate environments and emitting a
// `Mux` for every binding whose value differs -- an SSA join without needing
// blocks, which works precisely because there are no loops.

use std::collections::{BTreeMap, HashMap};

use crate::diag::{Diag, DiagSink, SourceMap, Span};
use crate::lex::{AlphanumSpan, ArgTypeQualifier};
use crate::parse::{
    BuiltinOp, FunctionDecl, Literal, PrecResExpr, PrecResInnerStmt, ProcessDecl,
    anumspan_to_str,
};
use crate::symbols::Symbols;
use crate::ty::{
    self, MemKind, OpTyError, Ty, binop_result, bits_for, comparison_operand_ty, const_eval,
    literal_fits, resolve_type_expr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDir {
    In,
    Out,
}

#[derive(Debug, Clone)]
pub struct Port {
    pub name: String,
    pub dir: PortDir,
    pub ty: Ty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    /// Logical or arithmetic according to the left operand's signedness.
    Shr,
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `~x`
    BitNot,
    /// `-x`, wrapping at the operand's width.
    Neg,
    /// `!x`, u1 in and out.
    LogNot,
}

#[derive(Debug, Clone)]
pub enum Op {
    /// Reads an input port.
    Port(PortId),
    /// The value a register holds at the start of the cycle. The index is
    /// into `Module::regs`.
    RegRead(u32),
    Const(u128),
    Bin { op: BinOp, lhs: ValueId, rhs: ValueId },
    /// Operands are already extended to a common type by the lowering, so the
    /// backend can compare them directly.
    Cmp { op: CmpOp, lhs: ValueId, rhs: ValueId },
    Un { op: UnOp, arg: ValueId },
    /// Constant bit range, inclusive, `hi >= lo`.
    Slice { arg: ValueId, hi: u32, lo: u32 },
    /// `arg[base +: width]` -- the computed-base part-select that
    /// k2g_decode.sv:89 and k2g_mon.sv:304 depend on.
    DynSlice { arg: ValueId, base: ValueId, width: u32 },
    /// High-to-low, matching Verilog's `{a, b, c}`.
    Concat(Vec<ValueId>),
    Repeat { arg: ValueId, times: u32 },
    ZExt { arg: ValueId, to: u32 },
    SExt { arg: ValueId, to: u32 },
    Trunc { arg: ValueId, to: u32 },
    /// Reinterprets the bits; width is unchanged.
    Cast { arg: ValueId },
    Mux { cond: ValueId, then_val: ValueId, else_val: ValueId },
    /// A `match`, kept whole rather than folded into a chain of `Mux`.
    ///
    /// This exists for one measured reason: on the GW1NR-9C the same 20-way
    /// selection costs 536 cells as nested ternaries and 210 as a `case`.
    /// A ternary chain is a PRIORITY structure, and synthesis has to honour
    /// the priority; a case statement says the arms are parallel.
    ///
    /// Each arm carries every discriminant that selects it, so an or-pattern
    /// becomes several labels on one arm -- exactly `LB_ADD, LB_SUB, LB_MUL,
    /// LB_DIV:` in the SystemVerilog this replaces.
    Case {
        scrutinee: ValueId,
        arms: Vec<(Vec<u128>, ValueId)>,
        default: ValueId,
    },
    /// An asynchronous read of `Module::mems[mem]`.
    ///
    /// Reads see the memory as of the start of the cycle: the write lands at
    /// the clock edge, so a read and a write of the same address in one cycle
    /// give the OLD value. That is read-before-write, it is what the SSRAM
    /// primitive does anyway, and k2g_regfile.sv:118 records why it must not
    /// be bypassed -- a write-first bypass there closes a combinational loop
    /// through the register file and hangs simulation.
    MemRead { mem: u32, addr: ValueId },
    /// The output register of a memory's synchronous read port.
    ///
    /// A leaf: it is driven by the memory's own clocked block, not by anything
    /// in the value graph, so nothing here computes it.
    MemReadReg { mem: u32, port: u32 },
}

#[derive(Debug, Clone)]
pub struct ValueDef {
    pub id: ValueId,
    pub ty: Ty,
    pub op: Op,
    /// Set when the value came from a named `let`, so the backend can emit a
    /// readable wire name instead of a numbered temporary.
    pub name: Option<String>,
}

/// A register: state that survives the clock edge.
///
/// `reset` is the value on synchronous reset, `next` the value the body left
/// behind at the end of the cycle. Both are ordinary values in the same graph,
/// so nothing downstream needs a second evaluation order.
#[derive(Debug, Clone)]
pub struct Reg {
    pub name: String,
    pub ty: Ty,
    pub reset: u128,
    pub next: ValueId,
}

/// An array with a backing store, and as many ports as the source asked for.
///
/// ONE WRITE PORT IS WHAT AN FPGA CAN INFER, and K2G paid for learning it: a
/// second write port on the 32x32 value array inferred no RAM at all and the
/// array collapsed to 1120 flip-flops and ~3700 LUTs of read muxing, against
/// 32 SSRAM primitives and ~100 LUTs for the single-port version
/// (k2g_regfile.sv:18-24).
///
/// That is a cost on one target, though, not a rule about what a design may
/// say. A memory compiler on an ASIC flow emits a real multi-write cell, and
/// capping the language at one port would put an FPGA's limit in the way of
/// every other target. So the count follows the source -- one port per write
/// that can happen in a cycle -- and the two answers to what that costs are
/// both available: emit the ports and let the target's compiler build the
/// cell, or `--lvt-bram` to build one out of one-write blocks and a live
/// value table, which is what gets a block RAM back on an FPGA.
///
/// Writes that CANNOT happen together still share a port, which is the case
/// the measurement above is really about: the arms of an `if` both write slot
/// 0 and the SSA join muxes them, so the ordinary conditional write is one
/// port as it always was.
#[derive(Debug, Clone)]
pub struct Memory {
    pub name: String,
    pub elem: Ty,
    pub len: u32,
    pub kind: MemKind,
    /// Folded to a literal here so the backend never emits `$clog2`, which
    /// makes GowinSynthesis exit 1 with an empty log.
    pub addr_width: u32,
    /// What every element takes on reset, or `None` for a memory that powers
    /// up undefined. The reset loop costs area -- roughly 85 LUTs on the K2G
    /// value array -- so it is a choice the source makes, not a default.
    pub reset: Option<u128>,
    /// The write ports, as ordinary values in the same graph, in source
    /// order: a later one overrides an earlier one at the same address.
    ///
    /// A LIST because two writes that can happen in the same cycle are two
    /// ports and there is nothing to mux them onto. Writes that CANNOT --
    /// the two arms of an `if` -- share one, and the SSA join is what proves
    /// it: both arms write slot 0, and the join muxes them together.
    pub write: Vec<WritePort>,
    /// A port that never fires, emitted once with the memory.
    ///
    /// Every join needs one for the arm that wrote fewer times, and every
    /// mux chain needs one to start from. Kept here rather than emitted where
    /// it is wanted, so a memory contributes three constants to the value
    /// graph and not three per question asked about it.
    pub idle: WritePort,
    /// The synchronous read ports, in the order the source asked for them.
    ///
    /// Empty for a `lutram`, whose reads are combinational and need no port --
    /// that is the whole difference between the two kinds, and it is the
    /// difference the backend has to emit for a synthesizer to infer the right
    /// primitive.
    ///
    /// A LIST because a pipeline's stages are concurrent: two reads in one
    /// stage are two addresses in the same cycle and there is nothing to mux
    /// them onto. A `process` still settles its reads onto one port, because
    /// one state is current and the mux is free -- so the length here is what
    /// the lowering decided it needed, not a property of the kind.
    pub read: Vec<ReadPort>,
}

/// One write port: when, where, and what.
#[derive(Debug, Clone, Copy)]
pub struct WritePort {
    pub we: ValueId,
    pub addr: ValueId,
    pub data: ValueId,
}

/// A synchronous read: an address, an enable, and the register the value
/// lands in.
///
/// The register belongs to the MEMORY and is driven from inside the memory's
/// own clocked block, because that is the shape block RAM is inferred from. A
/// read written as `wire q = mem[addr];` with the flop somewhere else is a
/// combinational array read plus a register, and infers distributed RAM --
/// which is what `lutram` already gives you, with a wasted flop on top.
#[derive(Debug, Clone)]
pub struct ReadPort {
    pub addr: ValueId,
    /// When to capture. Held rather than free-running: the value has to
    /// survive however long the state that consumes it waits.
    pub en: ValueId,
}

/// An immediate assertion: a condition that must hold, checked in simulation.
///
/// `cond` is already guarded by the path it was written on, so an assertion
/// inside an `if` reads as an implication and is vacuously true elsewhere.
/// Nothing about it reaches synthesis -- the emitted block sits inside
/// `ifdef SIMULATION`, which is the guard the target toolchain needs because
/// GowinSynthesis does not define `SYNTHESIS`.
#[derive(Debug, Clone)]
pub struct Assertion {
    pub cond: ValueId,
    pub message: String,
    /// `$fatal` rather than `$error`: stop, do not carry on producing output
    /// that is already known to be wrong.
    pub is_fatal: bool,
}

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub ports: Vec<Port>,
    pub values: Vec<ValueDef>,
    /// Final driver of each output port, in port order.
    pub drivers: Vec<(PortId, ValueId)>,
    /// Empty for a combinational `fun`; a `process` has clk/rst_n and these.
    pub regs: Vec<Reg>,
    pub mems: Vec<Memory>,
    pub asserts: Vec<Assertion>,
    /// Constant parameters, folded away before the backend runs. Kept only so
    /// the emitted file can say what they were: a module whose shape depends
    /// on a number that appears nowhere in it is hard to read.
    pub params: Vec<(String, Ty, u128)>,
    /// Wires between instances. A `graph` has these and nothing else -- no
    /// values, no registers -- because a graph computes nothing.
    pub nets: Vec<Net>,
    /// Submodules, in source order. Empty for everything but a `graph`: a
    /// `fun` call is inlined, so this is the only place hierarchy comes from.
    pub instances: Vec<Instance>,
    /// Every `fun` this module's body called. The other half of the use
    /// graph: `instances` records the hierarchy a `graph` builds, and this
    /// records the calls a body inlined, which leave no instance behind.
    /// Together they say which modules nothing uses, and so what an
    /// invocation is for.
    pub calls: Vec<String>,
}

/// One wire in a `graph`, carrying one leg of an internal pipe.
#[derive(Debug, Clone)]
pub struct Net {
    pub name: String,
    pub ty: Ty,
}

/// One instantiated `process` or `sequence`.
#[derive(Debug, Clone)]
pub struct Instance {
    /// The module being instantiated.
    pub module: String,
    /// The instance's own name, unique within the graph.
    pub name: String,
    /// `(formal, actual)`, connected by name. By name rather than by position
    /// because the port order of a process is an implementation detail of the
    /// lowering -- three ports per pipe, in an order this file chose.
    pub conns: Vec<(String, String)>,
    /// The pipes this instance drives, by the graph's name for them.
    ///
    /// Kept rather than re-derived from the connection list, because which end
    /// an instance is on is decided by the CALLEE's parameter direction, and
    /// that is known here and nowhere downstream. It is also the same fact
    /// `check_endpoints` counts, so a picture drawn from it cannot disagree
    /// with the design that was checked.
    pub produces: Vec<String>,
}

impl Module {
    /// Whether the module needs `clk` and `rst_n`.
    ///
    /// A graph has no registers of its own and still needs both: every
    /// instance in it does.
    pub fn is_clocked(&self) -> bool {
        !self.regs.is_empty() || !self.mems.is_empty() || !self.instances.is_empty()
    }
}

impl Module {
    pub fn value(&self, id: ValueId) -> &ValueDef {
        &self.values[id.0 as usize]
    }

    pub fn port(&self, id: PortId) -> &Port {
        &self.ports[id.0 as usize]
    }
}

/// The IR of one module, as text.
///
/// For `--emit=ir`. The point is to be able to see what the compiler decided
/// BEFORE the backend folds expressions together: which values became
/// registers, where the pipeline cut, what the write port of each memory ended
/// up carrying. Reading that out of the emitted Verilog means reading it
/// through one more transformation.
pub fn render_module(m: &Module) -> String {
    let mut out = String::new();
    out.push_str(&format!("module {}\n", m.name));
    for (ix, p) in m.ports.iter().enumerate() {
        let dir = match p.dir {
            PortDir::In => "in ",
            PortDir::Out => "out",
        };
        out.push_str(&format!("  port  #{} {} {} : {}\n", ix, dir, p.name, p.ty.display()));
    }
    for net in &m.nets {
        out.push_str(&format!("  net   {} : {}\n", net.name, net.ty.display()));
    }
    for inst in &m.instances {
        out.push_str(&format!("  inst  {} : {}\n", inst.name, inst.module));
        for (formal, actual) in &inst.conns {
            out.push_str(&format!("          .{} = {}\n", formal, actual));
        }
    }
    for mem in &m.mems {
        let reset = match mem.reset {
            Some(k) => format!("reset={}", k),
            None => "no reset".to_string(),
        };
        out.push_str(&format!(
            "  mem   {} : [{}; {}] {} addr:{}b {} we=%{} addr=%{} data=%{}\n",
            mem.name,
            mem.elem.display(),
            mem.len,
            mem.kind.display(),
            mem.addr_width,
            reset,
            mem.write.iter().map(|w| w.we.0.to_string()).collect::<Vec<_>>().join(","),
            mem.write.iter().map(|w| w.addr.0.to_string()).collect::<Vec<_>>().join(","),
            mem.write.iter().map(|w| w.data.0.to_string()).collect::<Vec<_>>().join(",")
        ));
    }
    for (ix, r) in m.regs.iter().enumerate() {
        out.push_str(&format!(
            "  reg   #{} {} : {} reset={} next=%{}\n",
            ix, r.name, r.ty.display(), r.reset, r.next.0
        ));
    }
    out.push('\n');
    for def in &m.values {
        let named = match &def.name {
            Some(n) => format!("  ; {}", n),
            None => String::new(),
        };
        out.push_str(&format!(
            "  %{} : {} = {}{}\n",
            def.id.0,
            def.ty.display(),
            render_op_ir(m, &def.op),
            named
        ));
    }
    if !m.drivers.is_empty() {
        out.push('\n');
    }
    for (port, value) in &m.drivers {
        out.push_str(&format!("  drive {} = %{}\n", m.port(*port).name, value.0));
    }
    for a in &m.asserts {
        let kind = if a.is_fatal { "fatal " } else { "assert" };
        out.push_str(&format!("  {} %{} {:?}\n", kind, a.cond.0, a.message));
    }
    out
}

fn render_op_ir(m: &Module, op: &Op) -> String {
    match op {
        Op::Port(p) => format!("port {}", m.port(*p).name),
        Op::RegRead(ix) => format!("reg {}", m.regs.get(*ix as usize).map_or("?", |r| &r.name)),
        Op::Const(k) => format!("const {}", k),
        Op::Bin { op, lhs, rhs } => format!("{:?} %{}, %{}", op, lhs.0, rhs.0),
        Op::Cmp { op, lhs, rhs } => format!("{:?} %{}, %{}", op, lhs.0, rhs.0),
        Op::Un { op, arg } => format!("{:?} %{}", op, arg.0),
        Op::Slice { arg, hi, lo } => format!("slice %{}[{}:{}]", arg.0, hi, lo),
        Op::DynSlice { arg, base, width } => {
            format!("dynslice %{}[%{} +: {}]", arg.0, base.0, width)
        }
        Op::Concat(parts) => {
            let items: Vec<String> = parts.iter().map(|p| format!("%{}", p.0)).collect();
            format!("concat {}", items.join(", "))
        }
        Op::Repeat { arg, times } => format!("repeat %{} x{}", arg.0, times),
        Op::ZExt { arg, to } => format!("zext %{} to {}", arg.0, to),
        Op::SExt { arg, to } => format!("sext %{} to {}", arg.0, to),
        Op::Trunc { arg, to } => format!("trunc %{} to {}", arg.0, to),
        Op::Cast { arg } => format!("cast %{}", arg.0),
        Op::Mux { cond, then_val, else_val } => {
            format!("mux %{} ? %{} : %{}", cond.0, then_val.0, else_val.0)
        }
        Op::Case { scrutinee, arms, default } => {
            let mut text = format!("case %{}", scrutinee.0);
            for (labels, v) in arms {
                let ls: Vec<String> = labels.iter().map(|l| l.to_string()).collect();
                text.push_str(&format!(" [{} -> %{}]", ls.join("|"), v.0));
            }
            text.push_str(&format!(" [_ -> %{}]", default.0));
            text
        }
        Op::MemReadReg { mem, port } => format!("memq #{}.{}", mem, port),
        Op::MemRead { mem, addr } => {
            format!("memread {}[%{}]", m.mems[*mem as usize].name, addr.0)
        }
    }
}

// ---- lowering ------------------------------------------------------------

/// What a name is bound to while lowering.
#[derive(Clone)]
pub struct Binding {
    pub value: Option<ValueId>,
    pub ty: Ty,
    /// Output parameters are assigned rather than read; reading one before it
    /// has been written is an error rather than an undefined wire.
    pub is_output: bool,
    /// Declared `var`. `let` is a constant, and assigning to one is an error
    /// rather than a redefinition -- the distinction existed only in the
    /// reader's head until this field, because nothing consulted `is_mutable`
    /// outside the scan that picks registers out of a process body.
    pub is_mutable: bool,
}

impl Binding {
    /// A `let`: bound once, never assigned.
    pub fn constant(value: ValueId, ty: Ty) -> Binding {
        Binding { value: Some(value), ty, is_output: false, is_mutable: false }
    }

    /// A `var`: state, or a mutable local.
    pub fn variable(value: ValueId, ty: Ty) -> Binding {
        Binding { value: Some(value), ty, is_output: false, is_mutable: true }
    }
}

/// Ordered, and that is load-bearing rather than tidy.
///
/// The SSA joins walk `env.keys()` to decide which bindings a branch disagreed
/// about, so the iteration order decides the order values are emitted in. With
/// a `HashMap` that order is randomised per process, and the same source
/// compiled three times produced three different files -- which makes the
/// `--check` staleness gate impossible to pass and every regeneration a diff.
pub type Env = BTreeMap<String, Binding>;

/// A pipe parameter, and what the body did with it.
///
/// The compiler writes the protocol, not the user, and the protocol is two
/// entries with a salt bit pair on each side -- `docs/attic/pipe.sv`'s
/// `PipeCDC` in KAMASUTRA2G, which that repo kept for exactly this idea.
///
/// Each side reads only the other's REGISTERED bits, so there is no
/// combinational path across a module boundary in either direction. Channel
/// rule 3 -- `valid` must not depend combinationally on `ready` -- stops being
/// a rule to keep and becomes a shape that cannot be written. k2g_chan.sv
/// records the bug it existed to prevent: routing a stall back into `cp_valid`
/// closed a loop through stall -> decode -> CSP request -> stall.
///
/// The salt pair is a gray-coded pointer, which is why the vector compares
/// below are exact rather than approximate: `wsalt` runs 00, 01, 11, 10, the
/// 2-bit gray code of 0..3, so it is `k2g_cdc_fifo`'s `wgray` at DEPTH 2.
#[derive(Debug, Clone)]
pub struct PipeInfo {
    pub name: String,
    /// The PAYLOAD type. `data_port` is two of these; everything that
    /// type-checks a send or a receive reads this one.
    pub ty: Ty,
    pub is_input: bool,
    /// `<name>_wsalt` (2 bits, producer's), `<name>_rsalt` (2 bits,
    /// consumer's), `<name>_data` (two entries, entry 0 in the low half).
    pub wsalt_port: PortId,
    pub rsalt_port: PortId,
    pub data_port: PortId,
    /// The raw two-entry read of the data port; inputs only.
    pub data_value: Option<ValueId>,
    /// `data[ridx]` -- the entry this side is owed. Inputs only, and set by
    /// each lowering once its `rsalt` register exists, which is why it cannot
    /// be `data_value`.
    pub item: Option<ValueId>,
    /// Inputs: the `rsalt` register. Outputs: unused (see `slot_reg`).
    pub salt_reg: Option<usize>,
    /// The entry index this side is at, emitted once and reused.
    pub idx: Option<ValueId>,
    /// Can an item move: `!empty` at a consumer, `!full` at a producer.
    /// Emitted once, from registers on both sides.
    pub movable: Option<ValueId>,
    /// Set once the body has done a `@try_rcv` / `@try_send` on this pipe.
    pub used: bool,
    /// What `@try_send` offered; outputs only.
    pub sent: Option<ValueId>,
    /// The branch this pipe's operation sat on, if not at the top level.
    ///
    /// A decoder does not produce a micro-op every cycle. Without this an
    /// offer written inside an `if` would leak out of it -- `sent` lives on
    /// the lowerer rather than in the environment, so the SSA join that muxes
    /// everything else never sees it.
    pub send_guard: Option<ValueId>,
    /// Inputs: this pipe transferred this cycle. Outputs: the slot can take a
    /// new item. Both are computed before the body, from this side's registers
    /// and the other side's salt -- which is also a register.
    pub fired: Option<ValueId>,
    /// Outputs only: the first of the three registers backing the slot --
    /// entry 0, entry 1, then the `wsalt` register.
    pub slot_reg: Option<usize>,
}

/// A salt: two bits, gray-coded, counting 0..3 over a buffer that holds two.
pub(crate) const SALT: Ty = Ty::UInt(2);

/// One narrowing of the path an assertion sits on.
#[derive(Debug, Clone)]
pub(crate) enum PathTerm {
    Cond { value: ValueId, taken: bool },
    Labels { scrutinee: ValueId, labels: Vec<u128>, taken: bool },
}

/// What a parameter of a `process` or a `sequence` is.
///
/// Data crosses the boundary through pipes and through nothing else. A plain
/// parameter is a CONSTANT: folded at elaboration, never a port. That is what
/// separates DDL from a nicer Verilog -- a process cannot present a raw wire
/// and hand-roll a protocol over it, because the protocol is the compiler's
/// job and hand-rolling it is the thing this language exists to stop.
pub enum ParamKind {
    Pipe { is_input: bool },
    Constant,
}

/// `got`, narrowed to the branch the operation sat on.
///
/// The pipe is only CLAIMED on that branch, so on any other one no transfer
/// happened and the answer has to say so. Without this a `@try_rcv` inside an
/// `if` reports a transfer the handshake never performed.
fn narrow_to_path(low: &mut Lowerer, fired: ValueId, path: Option<ValueId>) -> ValueId {
    match path {
        None => fired,
        Some(p) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: fired, rhs: p }),
    }
}

/// Classifies one parameter, or reports why it cannot be one.
pub fn classify_param(
    low: &Lowerer,
    arg: &crate::parse::PrecArgTupleEntry,
    sink: &mut DiagSink,
) -> Option<ParamKind> {
    match arg.qualifier {
        ArgTypeQualifier::BufferIn => Some(ParamKind::Pipe { is_input: true }),
        ArgTypeQualifier::BufferOut => Some(ParamKind::Pipe { is_input: false }),
        // desc.md:29 draws the line this sits on -- "compute only logic in
        // ddl, io in verilog". A `wire` is the far side of it: a pin, a PLL,
        // a bus whose master does not take `ready` for an answer. A body has
        // nothing to wait on there, so a `wire` reaches the program only
        // through the two declarations that do no computing.
        ArgTypeQualifier::WireIn | ArgTypeQualifier::WireOut => {
            sink.push(
                Diag::error(
                    low.span_of(&arg.arg_name),
                    format!(
                        "`{}` is a `wire`, which this declaration cannot take",
                        anumspan_to_str(&arg.arg_name)
                    ),
                )
                .with_note("only an `extern` or a `graph` takes a `wire`; a body reads a `buffer`"),
            );
            None
        }
        ArgTypeQualifier::Inout => {
            sink.err_at(&arg.arg_name, "`inout` parameters are not supported");
            None
        }
        ArgTypeQualifier::Out => {
            sink.push(
                Diag::error(
                    low.span_of(&arg.arg_name),
                    format!(
                        "`{}` is an `out` parameter, and a process has no plain outputs",
                        anumspan_to_str(&arg.arg_name)
                    ),
                )
                .with_note("results leave through a `buffer out` pipe"),
            );
            None
        }
        ArgTypeQualifier::In => Some(ParamKind::Constant),
    }
}

pub struct Lowerer<'a> {
    map: &'a SourceMap,
    pub syms: &'a Symbols,
    /// Resolved bodies of every function in the file, so a call can be inlined.
    pub bodies: &'a HashMap<String, &'a FunctionDecl>,
    /// Names currently being inlined. A repeat is a combinational loop, which
    /// is not a stack overflow in hardware -- it is a circuit that does not
    /// settle -- so it is rejected rather than expanded.
    pub call_stack: Vec<String>,
    /// Every `fun` this body called, at any depth. A call is inlined and
    /// leaves no `Instance`, so this is the only record that the callee is
    /// used -- which is what decides whether it is an export root.
    pub calls: std::collections::BTreeSet<String>,
    /// Pipe parameters in declaration order.
    pub pipes: Vec<PipeInfo>,
    /// `port in` parameters in declaration order.
    /// `port out` parameters in declaration order.
    /// Memories in declaration order. Reads name one by index.
    pub mems: Vec<Memory>,
    /// How many write ports each memory has taken on the path being lowered.
    ///
    /// Beside the environment rather than in it, because it is a count the
    /// compiler keeps and not a value the hardware holds -- a branch join
    /// takes the LARGER of its two arms' counts, which is not something a mux
    /// could express.
    mem_slots: Vec<usize>,
    /// Immediate assertions, in source order.
    pub asserts: Vec<Assertion>,
    /// Source locations of compiler-owned lexical identifiers.
    pub(crate) synthetic_spans: HashMap<usize, Span>,
    /// Constant parameters, in declaration order.
    pub params: Vec<(String, Ty, u128)>,
    /// `!done` for a process that stops, so its memories stop being written
    /// when it does. `None` for one that repeats.
    pub stop_writes: Option<ValueId>,
    /// Set while lowering a `sequence`.
    ///
    /// Read only by diagnostics, which have to name the place a cycle can be
    /// spent, and the two kinds of declaration spell it differently: a state
    /// in a process, a `|||` cut in a pipeline.
    pub in_pipeline: bool,
    /// The conditions under which the statements being lowered right now run,
    /// outermost first. Empty means unconditionally.
    ///
    /// Kept as a DESCRIPTION rather than as emitted values, because only
    /// assertions ever consult it. Materialising each branch guard eagerly
    /// would put a comparison and an `and` into the graph for every `if` and
    /// every match arm in the program -- dead, stripped by the backend, and
    /// still enough to renumber every generated wire in every module.
    path: Vec<PathTerm>,
    pub(crate) transfer_paths: HashMap<String, Vec<Vec<PathTerm>>>,
    /// Where the statement being lowered right now is, innermost last.
    ///
    /// Lowering has no other idea where it is. The AST carries a span on every
    /// identifier and on nothing else -- there is no statement node to hang
    /// one on -- so a diagnostic raised from the middle of a lowering pass had
    /// no location at all and rendered at line 1 of the file, whichever line
    /// it was actually about.
    anchors: Vec<Span>,
    values: Vec<ValueDef>,
    ports: Vec<Port>,
}

impl<'a> Lowerer<'a> {
    fn claim_transfer(&mut self, name: &str, occupied: bool, receive: bool, sink: &mut DiagSink) -> Option<()> {
        fn disjoint(a: &[PathTerm], b: &[PathTerm]) -> bool {
            a.iter().any(|x| b.iter().any(|y| match (x, y) {
                (PathTerm::Cond { value: x, taken: a }, PathTerm::Cond { value: y, taken: b }) => x == y && a != b,
                (PathTerm::Labels { scrutinee: x, labels: a, taken: at }, PathTerm::Labels { scrutinee: y, labels: b, taken: bt }) if x == y => {
                    match (*at, *bt) {
                        (true, true) => a.iter().all(|v| !b.contains(v)),
                        (true, false) => a.iter().all(|v| b.contains(v)),
                        (false, true) => b.iter().all(|v| a.contains(v)),
                        _ => false,
                    }
                }
                _ => false,
            }))
        }
        let conflict = match self.transfer_paths.get(name) {
            Some(paths) => paths.iter().any(|p| !disjoint(p, &self.path)),
            None => occupied,
        };
        if conflict {
            let verb = if receive { "received from" } else { "sent to" };
            sink.err_span(self.here(), format!("`{}` is {} more than once in one cycle", name, verb));
            return None;
        }
        self.transfer_paths.entry(name.to_string()).or_default().push(self.path.clone());
        Some(())
    }

    fn union_transfer_guard(&mut self, occupied: bool, old: Option<ValueId>, new: Option<ValueId>) -> Option<ValueId> {
        if !occupied { return new; }
        match (old, new) {
            (Some(lhs), Some(rhs)) => Some(self.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs, rhs })),
            _ => None,
        }
    }

    fn request_pipe(&mut self, ix: usize, data: Option<ValueId>) -> ValueId {
        let p = self.pipes[ix].clone();
        let path = self.materialise_path();
        let result = narrow_to_path(self, p.fired.expect("pipe eligibility"), path);
        if let Some(value) = data {
            self.pipes[ix].sent = Some(match (p.sent, path) {
                (Some(old), Some(cond)) => self.emit(p.ty, Op::Mux { cond, then_val: value, else_val: old }),
                _ => value,
            });
        }
        self.pipes[ix].send_guard = self.union_transfer_guard(p.used, p.send_guard, path);
        self.pipes[ix].used = true;
        result
    }

    /// One definition of a nonblocking transfer, shared by its success result
    /// and the emitted pointer/data updates. `fired` contains execution and
    /// availability; the operation additionally requires an actual request
    /// on its source path. Merely observing a pipe never requests a transfer.
    pub(crate) fn pipe_transfer(&mut self, ix: usize) -> ValueId {
        if !self.pipes[ix].used {
            return self.emit(Ty::BOOL, Op::Const(0));
        }
        let eligible = self.pipes[ix]
            .fired
            .expect("eligibility established before lowering");
        narrow_to_path(self, eligible, self.pipes[ix].send_guard)
    }

    pub fn new(
        map: &'a SourceMap,
        syms: &'a Symbols,
        bodies: &'a HashMap<String, &'a FunctionDecl>,
    ) -> Self {
        Lowerer {
            map,
            syms,
            bodies,
            call_stack: Vec::new(),
            calls: std::collections::BTreeSet::new(),
            pipes: Vec::new(),
            mems: Vec::new(),
            mem_slots: Vec::new(),
            asserts: Vec::new(),
            synthetic_spans: HashMap::new(),
            params: Vec::new(),
            stop_writes: None,
            in_pipeline: false,
            path: Vec::new(),
            transfer_paths: HashMap::new(),
            anchors: Vec::new(),
            values: Vec::new(),
            ports: Vec::new(),
        }
    }


    /// A pipe becomes three flat ports, which is the flattening k3g_chan.sv:60
    /// already pre-commits to for the yosys-slang risk.
    pub fn declare_pipes(
        &mut self,
        args: &crate::parse::PrecArgDefTuple,
        env: &mut Env,
        sink: &mut DiagSink,
    ) -> Option<()> {
        for arg in &args.entries {
            let name = anumspan_to_str(&arg.arg_name).to_string();
            let kind = classify_param(self, arg, sink)?;
            let is_input = match kind {
                ParamKind::Constant => {
                    self.declare_constant(arg, env, sink)?;
                    continue;
                }
                ParamKind::Pipe { is_input } => is_input,
            };
            let ty = match resolve_type_expr(&arg.type_expr, self.syms) {
                Ok(t) => t,
                Err(e) => {
                    sink.err_at(&arg.arg_name, e.message());
                    return None;
                }
            };
            self.declare_pipe(name, ty, is_input);
        }
        Some(())
    }

    /// `clk` and `rst_n`, which every clocked module has and none declares.
    ///
    /// "A process has channel ports and clock/reset. Nothing else."
    /// -- k3g_chan.sv:31.
    pub fn declare_clock(&mut self, env: &mut Env) {
        for implicit in ["clk", "rst_n"] {
            let port_id = PortId(self.ports.len() as u32);
            self.ports.push(Port {
                name: implicit.to_string(),
                dir: PortDir::In,
                ty: Ty::BOOL,
            });
            let v = self.emit(Ty::BOOL, Op::Port(port_id));
            self.values[v.0 as usize].name = Some(implicit.to_string());
            env.insert(implicit.to_string(), Binding::constant(v, Ty::BOOL));
        }
    }



    /// One port of the declared width, with nothing attached to it.
    ///
    /// For the modules the compiler writes at a boundary: an adapter's FIFO
    /// face is a handshake in plain signals, not a pipe, so it cannot come
    /// from `declare_pipe`.
    pub(crate) fn declare_raw_port(&mut self, name: String, dir: PortDir, ty: Ty) -> PortId {
        let id = PortId(self.ports.len() as u32);
        self.ports.push(Port { name, dir, ty });
        id
    }

    /// One pipe: three flat ports, and the bookkeeping that goes with them.
    ///
    /// Factored out of the parameter walk so a module the compiler writes --
    /// a `@merge` or a `@split` -- gets the same interface as one a program
    /// declares. Two spellings of the port list is how the two would drift.
    pub fn declare_pipe(&mut self, name: String, ty: Ty, is_input: bool) -> usize {
        let (vd, rd, dd) = if is_input {
            (PortDir::In, PortDir::Out, PortDir::In)
        } else {
            (PortDir::Out, PortDir::In, PortDir::Out)
        };
        let mk = |low: &mut Self, suffix: &str, dir: PortDir, t: Ty| {
            let id = PortId(low.ports.len() as u32);
            low.ports.push(Port { name: format!("{}_{}", name, suffix), dir, ty: t });
            id
        };
        let wsalt_port = mk(self, "wsalt", vd, SALT);
        let rsalt_port = mk(self, "rsalt", rd, SALT);
        let pair = Ty::Array(Box::new(ty.clone()), 2);
        let data_port = mk(self, "data", dd, pair.clone());
        let data_value = if is_input {
            let v = self.emit(pair, Op::Port(data_port));
            self.values[v.0 as usize].name = Some(format!("{}_data", name));
            Some(v)
        } else {
            None
        };
        self.pipes.push(PipeInfo {
            name,
            ty,
            is_input,
            wsalt_port,
            rsalt_port,
            data_port,
            data_value,
            item: None,
            salt_reg: None,
            idx: None,
            movable: None,
            used: false,
            sent: None,
            send_guard: None,
            fired: None,
            slot_reg: None,
        });
        self.pipes.len() - 1
    }

    /// Binds a constant parameter, folding its value at elaboration.
    ///
    /// No port is emitted. A parameter with no value is the error the whole
    /// rule exists to produce: it means the source expected per-cycle data
    /// there, and per-cycle data arrives through a pipe.
    pub fn declare_constant(
        &mut self,
        arg: &crate::parse::PrecArgTupleEntry,
        env: &mut Env,
        sink: &mut DiagSink,
    ) -> Option<()> {
        let name = anumspan_to_str(&arg.arg_name).to_string();
        let ty = match resolve_type_expr(&arg.type_expr, self.syms) {
            Ok(t) => t,
            Err(e) => {
                sink.err_at(&arg.arg_name, e.message());
                return None;
            }
        };
        if ty.is_memory() {
            sink.push(
                Diag::error(
                    self.span_of(&arg.arg_name),
                    format!("`{}` is a memory, which cannot be a parameter", name),
                )
                .with_note("declare it inside the process with `var`"),
            );
            return None;
        }
        let init = match &arg.default {
            Some(e) => e,
            None => {
                sink.push(
                    Diag::error(
                        self.span_of(&arg.arg_name),
                        format!("`{}` has no value", name),
                    )
                    .with_note(
                        "a plain parameter is a compile-time constant and needs `= <value>`; data arrives through a `buffer in` pipe",
                    ),
                );
                return None;
            }
        };
        let empty: Env = Env::new();
        let value = lower_expr_expecting(self, init, Some(&ty), &empty, sink)?;
        let konst = match &self.values[value.0 as usize].op {
            Op::Const(k) => *k,
            _ => {
                sink.err_at(&arg.arg_name, format!("`{}` must be a compile-time constant", name));
                return None;
            }
        };
        let have = self.ty_of(value);
        if have != ty {
            match self.coerce_const(value, &ty) {
                Some(_) => {}
                None => {
                    sink.push(
                        Diag::error(
                            self.span_of(&arg.arg_name),
                            format!(
                                "`{}` is declared `{}` but its value is `{}`",
                                name,
                                ty.display(),
                                have.display()
                            ),
                        )
                        .with_note(cast_hint(&have, &ty)),
                    );
                    return None;
                }
            }
        }
        let folded = self.emit(ty.clone(), Op::Const(konst));
        env.insert(name.clone(), Binding::constant(folded, ty.clone()));
        self.params.push((name, ty, konst));
        Some(())
    }

    pub fn take_values(self) -> (Vec<ValueDef>, Vec<Port>) {
        (self.values, self.ports)
    }

    pub fn add_port(&mut self, name: String, dir: PortDir, ty: Ty) -> PortId {
        let id = PortId(self.ports.len() as u32);
        self.ports.push(Port { name, dir, ty });
        id
    }

    /// Env keys holding one of a memory's write ports while the body runs.
    ///
    /// `#` cannot appear in an identifier, so these cannot collide with a name
    /// the source chose. Keeping them in the ordinary environment is what
    /// makes a write inside an `if` work with no extra machinery: the SSA join
    /// muxes them exactly as it muxes any other binding, so
    /// `if we_value: values[w_addr] = w_value` becomes a write enable.
    ///
    /// The slot is which port. They are created ON DEMAND, one per write the
    /// path has performed, so a memory nothing writes has none at all and a
    /// branch that writes once more than the other is a slot only one arm
    /// filled -- which the join finishes with an idle port.
    pub fn mem_port_keys(name: &str, slot: usize) -> (String, String, String) {
        (
            format!("{}#{}#we", name, slot),
            format!("{}#{}#addr", name, slot),
            format!("{}#{}#data", name, slot),
        )
    }

    /// How many write ports `mems[ix]` has taken on the path being lowered.
    pub fn mem_slots(&self, ix: usize) -> usize {
        self.mem_slots[ix]
    }

    /// Sets that count, for a join that reconciles two arms or a state that
    /// starts over.
    pub fn set_mem_slots(&mut self, ix: usize, n: usize) {
        self.mem_slots[ix] = n;
    }

    /// A port that never fires: enabled never, addressing nothing, zero data.
    pub fn idle_write(&mut self, ix: usize) -> WritePort {
        self.mems[ix].idle
    }

    /// Reads slot `slot` of `mems[ix]` out of `env`, if it has one.
    ///
    /// Emits nothing, so probing a slot an arm never filled costs no values --
    /// the caller supplies `idle_write` only where it turns out to need one.
    pub fn write_slot(&self, ix: usize, slot: usize, env: &Env) -> Option<WritePort> {
        let (we_key, addr_key, data_key) = Self::mem_port_keys(&self.mems[ix].name, slot);
        Some(WritePort {
            we: env.get(&we_key).and_then(|b| b.value)?,
            addr: env.get(&addr_key).and_then(|b| b.value)?,
            data: env.get(&data_key).and_then(|b| b.value)?,
        })
    }

    /// Writes one slot back into `env`.
    pub fn put_write_slot(&mut self, ix: usize, slot: usize, port: WritePort, env: &mut Env) {
        let (we_key, addr_key, data_key) = Self::mem_port_keys(&self.mems[ix].name, slot);
        let (aw, elem) = (self.mems[ix].addr_width, self.mems[ix].elem.clone());
        env.insert(we_key, Binding::constant(port.we, Ty::BOOL));
        env.insert(addr_key, Binding::constant(port.addr, Ty::UInt(aw)));
        env.insert(data_key, Binding::constant(port.data, elem));
    }

    /// Joins the write ports two arms of a branch created.
    ///
    /// Slots below `base` existed before the branch and neither arm touched
    /// them -- a write appends, it never overwrites -- so only the ones the
    /// arms added need reconciling. An arm that added fewer gets an idle port
    /// for the difference, which is what makes `if c then m[a] = v` with no
    /// else come out as a write enable of `c` rather than as a second port.
    #[allow(clippy::too_many_arguments)]
    pub fn join_write_slots(
        &mut self,
        cond: ValueId,
        base: &[usize],
        then_env: &Env,
        then_n: &[usize],
        else_env: &Env,
        else_n: &[usize],
        env: &mut Env,
    ) {
        for ix in 0..self.mems.len() {
            let n = then_n[ix].max(else_n[ix]);
            for slot in base[ix]..n {
                let t = self.write_slot(ix, slot, then_env);
                let e = self.write_slot(ix, slot, else_env);
                let (t, e) = match (t, e) {
                    (None, None) => continue,
                    (Some(t), None) => {
                        let idle = self.idle_write(ix);
                        (t, idle)
                    }
                    (None, Some(e)) => {
                        let idle = self.idle_write(ix);
                        (idle, e)
                    }
                    (Some(t), Some(e)) => (t, e),
                };
                let we = self.mux_or_same(cond, t.we, e.we, Ty::BOOL);
                let addr_ty = Ty::UInt(self.mems[ix].addr_width);
                let addr = self.mux_or_same(cond, t.addr, e.addr, addr_ty);
                let elem = self.mems[ix].elem.clone();
                let data = self.mux_or_same(cond, t.data, e.data, elem);
                self.put_write_slot(ix, slot, WritePort { we, addr, data }, env);
            }
            self.mem_slots[ix] = n;
        }
    }

    /// Joins the write ports a `match`'s arms created.
    ///
    /// The `if` join with more than two ways to go: `arms` is every arm's
    /// environment and slot count in order, the LAST being the default, and
    /// each field becomes a `Case` on the tag rather than a mux.
    pub fn join_write_slots_case(
        &mut self,
        tag: ValueId,
        base: &[usize],
        arms: &[(Vec<u128>, Env, Vec<usize>)],
        env: &mut Env,
    ) {
        let Some((_, default_env, default_n)) = arms.last() else {
            return;
        };
        for ix in 0..self.mems.len() {
            let n = arms.iter().map(|(_, _, c)| c[ix]).max().unwrap_or(0);
            for slot in base[ix]..n {
                let idle = self.idle_write(ix);
                let default = if default_n[ix] > slot {
                    self.write_slot(ix, slot, default_env).unwrap_or(idle)
                } else {
                    idle
                };
                // Only arms that differ from the default earn a label, the
                // same rule the value join uses -- otherwise every arm would
                // contribute an entry for every port of every memory.
                let mut we_arms = Vec::new();
                let mut addr_arms = Vec::new();
                let mut data_arms = Vec::new();
                for (labels, arm_env, counts) in arms.iter().take(arms.len() - 1) {
                    let p = if counts[ix] > slot {
                        match self.write_slot(ix, slot, arm_env) {
                            Some(p) => p,
                            None => idle,
                        }
                    } else {
                        idle
                    };
                    if p.we != default.we {
                        we_arms.push((labels.clone(), p.we));
                    }
                    if p.addr != default.addr {
                        addr_arms.push((labels.clone(), p.addr));
                    }
                    if p.data != default.data {
                        data_arms.push((labels.clone(), p.data));
                    }
                }
                let addr_ty = Ty::UInt(self.mems[ix].addr_width);
                let elem = self.mems[ix].elem.clone();
                let we = self.case_or_same(tag, we_arms, default.we, Ty::BOOL);
                let addr = self.case_or_same(tag, addr_arms, default.addr, addr_ty);
                let data = self.case_or_same(tag, data_arms, default.data, elem);
                self.put_write_slot(ix, slot, WritePort { we, addr, data }, env);
            }
            self.mem_slots[ix] = n;
        }
    }

    /// A `Case`, or the default alone when no arm disagreed with it.
    fn case_or_same(
        &mut self,
        tag: ValueId,
        arms: Vec<(Vec<u128>, ValueId)>,
        default: ValueId,
        ty: Ty,
    ) -> ValueId {
        if arms.is_empty() {
            return default;
        }
        self.emit(ty, Op::Case { scrutinee: tag, arms, default })
    }

    /// `cond ? a : b`, or `a` when the two cannot differ.
    ///
    /// The constant case is not an optimisation for its own sake. Both arms of
    /// `if c then m[i] = x else m[j] = y` write with an enable of literal 1,
    /// emitted separately, so without this the shared port's enable comes out
    /// as `c ? 1'b1 : 1'b1`.
    fn mux_or_same(&mut self, cond: ValueId, a: ValueId, b: ValueId, ty: Ty) -> ValueId {
        if a == b {
            return a;
        }
        if let (Op::Const(x), Op::Const(y)) =
            (&self.values[a.0 as usize].op, &self.values[b.0 as usize].op)
            && x == y
        {
            return a;
        }
        self.emit(ty, Op::Mux { cond, then_val: a, else_val: b })
    }

    /// What each memory's write-port count is right now, for a branch to
    /// restore between its arms and reconcile after them.
    pub fn mem_slot_counts(&self) -> Vec<usize> {
        self.mem_slots.clone()
    }

    /// Restores those counts.
    pub fn set_mem_slot_counts(&mut self, counts: &[usize]) {
        self.mem_slots.copy_from_slice(counts);
    }

    /// Forgets every write port, for a state that starts its own.
    pub fn clear_write_slots(&mut self, env: &mut Env) {
        for ix in 0..self.mems.len() {
            for slot in 0..self.mem_slots[ix] {
                let (we, addr, data) = Self::mem_port_keys(&self.mems[ix].name, slot);
                env.remove(&we);
                env.remove(&addr);
                env.remove(&data);
            }
            self.mem_slots[ix] = 0;
        }
    }

    pub fn mem_index(&self, name: &str) -> Option<usize> {
        self.mems.iter().position(|m| m.name == name)
    }

    /// Declares a memory, its write port and the binding a subscript reaches.
    pub fn declare_memory(
        &mut self,
        name: String,
        elem: Ty,
        len: u32,
        kind: MemKind,
        reset: Option<u128>,
        env: &mut Env,
    ) -> usize {
        // `len - 1` rather than `len`: 32 entries are addressed by 5 bits, and
        // `bits_for(32)` would say 6.
        let addr_width = bits_for((len - 1) as u128);
        let idle = WritePort {
            we: self.emit(Ty::BOOL, Op::Const(0)),
            addr: self.emit(Ty::UInt(addr_width), Op::Const(0)),
            data: self.emit(elem.clone(), Op::Const(0)),
        };

        let mem_ty = Ty::Mem { elem: Box::new(elem.clone()), len, kind };
        // A memory is reached by subscript, never assigned as a whole.
        env.insert(
            name.clone(),
            Binding { value: None, ty: mem_ty, is_output: false, is_mutable: false },
        );

        let ix = self.mems.len();
        self.mems.push(Memory {
            name,
            elem,
            len,
            kind,
            addr_width,
            reset,
            write: Vec::new(),
            idle,
            read: Vec::new(),
        });
        self.mem_slots.push(0);
        ix
    }

    /// Reads back the write port each memory was left with at the end of the
    /// body, so what the source did decides what the write port carries.
    pub fn settle_memories(&mut self, env: &Env) {
        for ix in 0..self.mems.len() {
            let mut ports = Vec::with_capacity(self.mem_slots[ix]);
            for slot in 0..self.mem_slots[ix] {
                let Some(p) = self.write_slot(ix, slot, env) else {
                    continue;
                };
                let we = match self.stop_writes {
                    None => p.we,
                    Some(r) => self.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: p.we, rhs: r }),
                };
                ports.push(WritePort {
                    we,
                    addr: self.drop_gated_mux(p.addr, we),
                    data: self.drop_gated_mux(p.data, we),
                });
            }
            self.mems[ix].write = ports;
        }
    }

    /// Strips `gate ? x : <idle>` from a value the write enable already gates.
    ///
    /// The SSA join gives the address and the data the same mux it gives the
    /// write enable, because it does not know the three belong together. When
    /// the enable is false the write does not happen, so the address and the
    /// data are don't-cares -- but only when the mux is selected by exactly
    /// that enable, which is why this compares value ids rather than trying to
    /// prove an implication.
    pub fn drop_gated_mux(&self, mut value: ValueId, gate: ValueId) -> ValueId {
        loop {
            match &self.values[value.0 as usize].op {
                Op::Mux { cond, then_val, .. } if *cond == gate => value = *then_val,
                _ => return value,
            }
        }
    }

    /// Adapts an index expression to a memory's address width.
    ///
    /// A narrower index is zero-extended, because an address that cannot reach
    /// the whole array is not an error. A wider one is refused: dropping high
    /// address bits silently would turn an out-of-range access into a
    /// different in-range one.
    pub fn fit_address(
        &mut self,
        idx: ValueId,
        want: u32,
        at: &AlphanumSpan,
        sink: &mut DiagSink,
    ) -> Option<ValueId> {
        let have = self.ty_of(idx);
        if have.is_signed() {
            sink.err_at(at, format!("an address must be unsigned, found `{}`", have.display()));
            return None;
        }
        let w = have.bit_width();
        if w == want {
            return Some(idx);
        }
        if w < want {
            // A constant address is re-materialised at the address width, so
            // an unrolled `for` reads `mem[2'd1]` rather than
            // `mem[{{1{1'b0}}, 1'b1}]`.
            if let Op::Const(k) = self.values[idx.0 as usize].op {
                return Some(self.emit(Ty::UInt(want), Op::Const(k)));
            }
            return Some(self.emit(Ty::UInt(want), Op::ZExt { arg: idx, to: want }));
        }
        sink.push(
            Diag::error(
                self.span_of(at),
                format!(
                    "an index of `{}` is {} bits wide, but this memory is addressed by {}",
                    anumspan_to_str(at),
                    w,
                    want
                ),
            )
            .with_note(format!("narrow it with `@trunc(x, {})`", want)),
        );
        None
    }

    /// Enters the `then` (or `else`) side of an `if`. The answer is the depth
    /// to hand back to `pop_path`.
    pub fn push_cond(&mut self, value: ValueId, taken: bool) -> usize {
        self.path.push(PathTerm::Cond { value, taken });
        self.path.len() - 1
    }

    /// Enters a match arm: the scrutinee carries one of `labels`, or -- for
    /// the catch-all -- none of the labels the earlier arms claimed.
    pub fn push_labels(&mut self, scrutinee: ValueId, labels: Vec<u128>, taken: bool) -> usize {
        self.path.push(PathTerm::Labels { scrutinee, labels, taken });
        self.path.len() - 1
    }

    pub fn pop_path(&mut self, depth: usize) {
        self.path.truncate(depth);
    }

    fn logical_not(&mut self, v: ValueId) -> ValueId {
        self.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: v })
    }

    /// `v == labels[0] | v == labels[1] | ...`, and `0` for no labels.
    fn any_equal(&mut self, v: ValueId, labels: &[u128]) -> ValueId {
        let ty = self.ty_of(v);
        let mut acc: Option<ValueId> = None;
        for k in labels {
            let konst = self.emit(ty.clone(), Op::Const(*k));
            let eq = self.emit(Ty::BOOL, Op::Cmp { op: CmpOp::Eq, lhs: v, rhs: konst });
            acc = Some(match acc {
                None => eq,
                Some(prev) => self.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: prev, rhs: eq }),
            });
        }
        match acc {
            Some(v) => v,
            None => self.emit(Ty::BOOL, Op::Const(0)),
        }
    }

    /// Builds the value of the current path, emitting for the first time.
    pub fn materialise_path(&mut self) -> Option<ValueId> {
        let terms = self.path.clone();
        let mut acc: Option<ValueId> = None;
        for term in terms {
            let mut guard = match term {
                PathTerm::Cond { value, .. } => value,
                PathTerm::Labels { scrutinee, ref labels, .. } => {
                    self.any_equal(scrutinee, labels)
                }
            };
            let taken = match term {
                PathTerm::Cond { taken, .. } | PathTerm::Labels { taken, .. } => taken,
            };
            if !taken {
                guard = self.logical_not(guard);
            }
            acc = Some(match acc {
                None => guard,
                Some(outer) => {
                    self.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: outer, rhs: guard })
                }
            });
        }
        acc
    }

    /// Records an assertion, weakened by the path it sits on.
    ///
    /// `path -> cond` is `!path | cond`, so an assertion inside an `if` says
    /// nothing about the cycles the `if` did not take. Writing it as a plain
    /// `cond` would make every conditional assertion fire on the other branch.
    pub fn add_assert(&mut self, cond: ValueId, message: String, is_fatal: bool) {
        let path = self.materialise_path();
        let guarded = match path {
            None => cond,
            Some(path) => {
                let off_path = self.logical_not(path);
                self.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: off_path, rhs: cond })
            }
        };
        self.asserts.push(Assertion { cond: guarded, message, is_fatal });
    }

    /// Names a value unless it is already a signal in its own right.
    ///
    /// A port or a register read has a name the backend depends on, and a
    /// constant is folded rather than declared -- renaming any of them would
    /// produce a reference to a wire that is never emitted.
    /// Name a value only if it does not already have one.
    ///
    /// A single-barrier state's `fire_sN` and that pipe's `<p>_take` are the
    /// same wire, and `fire_sN` is the better name: it is what the state
    /// transition is written in terms of. Naming over it loses that vocabulary
    /// for a synonym.
    pub fn name_value_fresh(&mut self, v: ValueId, name: String) {
        if self.values[v.0 as usize].name.is_none() {
            self.name_value_safe(v, name);
        }
    }

    pub fn name_value_safe(&mut self, v: ValueId, name: String) {
        let is_already_a_signal = matches!(
            self.values[v.0 as usize].op,
            Op::Port(_) | Op::RegRead(_) | Op::Const(_)
        );
        if is_already_a_signal {
            return;
        }
        self.values[v.0 as usize].name = Some(name);
    }

    pub fn name_value(&mut self, v: ValueId, name: String) {
        self.values[v.0 as usize].name = Some(name);
    }

    pub fn emit(&mut self, ty: Ty, op: Op) -> ValueId {
        // `c ? 1'b1 : 1'b0` is `c`. Not cosmetic: an SSA join produces exactly
        // this for a flag set on one branch of an `if`, and a memory write
        // enable is that flag, so without this every conditional write carries
        // a redundant mux into synthesis.
        // `x & 1'b1` and `x | 1'b0` are `x`. The generated handshake produces
        // both -- an input's readiness is an AND over output slots, and with a
        // single output pipe that identity is all there is.
        if let Op::Bin { op: bin, lhs, rhs } = &op {
            let (bin, lhs, rhs) = (*bin, *lhs, *rhs);
            let identity = match bin {
                BinOp::And => Some(1u128),
                BinOp::Or => Some(0u128),
                _ => None,
            };
            if let Some(k) = identity {
                let one_bit = ty == Ty::BOOL;
                if one_bit && self.is_const(rhs, k) {
                    return lhs;
                }
                if one_bit && self.is_const(lhs, k) {
                    return rhs;
                }
            }
        }
        if let Op::Mux { cond, then_val, else_val } = &op {
            let (cond, then_val, else_val) = (*cond, *then_val, *else_val);
            let picks_the_condition = ty == Ty::BOOL
                && self.is_const(then_val, 1)
                && self.is_const(else_val, 0)
                && self.ty_of(cond) == Ty::BOOL;
            if picks_the_condition {
                return cond;
            }
        }
        let id = ValueId(self.values.len() as u32);
        self.values.push(ValueDef { id, ty, op, name: None });
        id
    }

    fn is_const(&self, v: ValueId, k: u128) -> bool {
        matches!(&self.values[v.0 as usize].op, Op::Const(c) if *c == k)
    }

    pub fn ty_of(&self, id: ValueId) -> Ty {
        self.values[id.0 as usize].ty.clone()
    }

    fn span(&self, at: &AlphanumSpan) -> Span {
        self.span_of(at)
    }

    pub fn span_of(&self, at: &AlphanumSpan) -> Span {
        self.synthetic_spans.get(&(at.byte_ptr as usize)).copied()
            .unwrap_or_else(|| self.map.span_of(at))
    }

    /// Where the diagnostic being raised right now belongs.
    ///
    /// The innermost statement being lowered, or nowhere if lowering has not
    /// entered one yet -- a parameter list, say, where the caller has a better
    /// span of its own and passes it.
    pub fn here(&self) -> Span {
        match self.anchors.last() {
            Some(s) => *s,
            None => crate::driver::nowhere(),
        }
    }

    /// Enters a statement. The answer is the depth to hand back to
    /// `pop_anchor`, matching how `push_cond` and `pop_path` pair up.
    pub fn push_anchor(&mut self, at: Span) -> usize {
        self.anchors.push(at);
        self.anchors.len() - 1
    }

    pub fn pop_anchor(&mut self, depth: usize) {
        self.anchors.truncate(depth);
    }


    /// Variant list of a named enum, or a diagnostic-free `None` if the table
    /// does not have it (which the caller has already reported).
    pub fn enum_variants(&self, name: &str) -> Option<Vec<(String, u128)>> {
        self.syms.enums.get(name).map(|d| d.variants.clone())
    }

    // ---- the salt protocol -------------------------------------------------
    //
    // Written once, here, and never inline. `pipe_full` and `pipe_empty` read
    // their own port rather than taking a salt argument, because passing the
    // wrong side's salt is the one mistake that compiles, synthesizes, and
    // deadlocks only under back-pressure.

    /// The salt the OTHER side publishes: `wsalt` at a consumer, `rsalt` at a
    /// producer.
    pub(crate) fn salt_from_other(&mut self, ix: usize) -> ValueId {
        let p = &self.pipes[ix];
        let port = if p.is_input { p.wsalt_port } else { p.rsalt_port };
        let name = format!("{}_{}", p.name, if p.is_input { "wsalt" } else { "rsalt" });
        let v = self.emit(SALT, Op::Port(port));
        self.name_value_safe(v, name);
        v
    }

    /// Which entry a salt points at: `salt[0] ^ salt[1]`.
    ///
    /// The gray sequence is 00, 01, 11, 10, so the parity of the two bits is
    /// the low bit of the binary position -- which for two entries is the
    /// index.
    pub(crate) fn salt_idx(&mut self, salt: ValueId, name: String) -> ValueId {
        let b0 = self.emit(Ty::BOOL, Op::Slice { arg: salt, hi: 0, lo: 0 });
        let b1 = self.emit(Ty::BOOL, Op::Slice { arg: salt, hi: 1, lo: 1 });
        let v = self.emit(Ty::BOOL, Op::Bin { op: BinOp::Xor, lhs: b0, rhs: b1 });
        self.name_value_safe(v, name);
        v
    }

    /// Nothing to take: the two salts agree.
    pub(crate) fn pipe_empty(&mut self, ix: usize, rsalt_q: ValueId) -> ValueId {
        let wsalt = self.salt_from_other(ix);
        let v = self.emit_eq(wsalt, rsalt_q);
        let name = format!("{}_empty", self.pipes[ix].name);
        self.name_value_safe(v, name);
        v
    }

    /// Nowhere to put one: the salts differ in BOTH bits, which in gray code
    /// is one lap ahead -- two entries, for a buffer that holds two.
    pub(crate) fn pipe_full(&mut self, ix: usize, wsalt_q: ValueId) -> ValueId {
        let rsalt = self.salt_from_other(ix);
        // BitNot, not LogNot: a one-bit answer here would widen in the compare
        // and `full` would be false almost always.
        let flipped = self.emit(SALT, Op::Un { op: UnOp::BitNot, arg: rsalt });
        let v = self.emit_eq(wsalt_q, flipped);
        let name = format!("{}_full", self.pipes[ix].name);
        self.name_value_safe(v, name);
        v
    }

    /// The salt after a transfer: toggle the bit the index names, or hold.
    pub(crate) fn salt_next(&mut self, salt_q: ValueId, idx: ValueId, enable: ValueId) -> ValueId {
        let one = self.emit(SALT, Op::Const(0b01));
        let two = self.emit(SALT, Op::Const(0b10));
        let bit = self.emit(SALT, Op::Mux { cond: idx, then_val: two, else_val: one });
        let toggled = self.emit(SALT, Op::Bin { op: BinOp::Xor, lhs: salt_q, rhs: bit });
        self.emit(SALT, Op::Mux { cond: enable, then_val: toggled, else_val: salt_q })
    }

    /// One entry out of the pair on the wire.
    ///
    /// A `Mux` over two constant-bounds slices rather than a `DynSlice`: the
    /// latter scales the index to a bit offset, and for a payload whose width
    /// is not a power of two that is a multiplier in front of a 2-way select.
    pub(crate) fn entry_of(&mut self, pair: ValueId, idx: ValueId, ty: &Ty) -> ValueId {
        let w = ty.bit_width();
        let e0 = self.emit(ty.clone(), Op::Slice { arg: pair, hi: w - 1, lo: 0 });
        let e1 = self.emit(ty.clone(), Op::Slice { arg: pair, hi: 2 * w - 1, lo: w });
        self.emit(ty.clone(), Op::Mux { cond: idx, then_val: e1, else_val: e0 })
    }

    /// The entry this side is owed, given its own `rsalt` register.
    pub(crate) fn pipe_item_at(&mut self, ix: usize, rsalt_q: ValueId) -> ValueId {
        let name = self.pipes[ix].name.clone();
        let ty = self.pipes[ix].ty.clone();
        let pair = self.pipes[ix].data_value.expect("an input pipe has a data value");
        let ridx = self.salt_idx(rsalt_q, format!("{}_ridx", name));
        let item = self.entry_of(pair, ridx, &ty);
        self.name_value_safe(item, format!("{}_item", name));
        self.pipes[ix].idx = Some(ridx);
        item
    }

    /// Both entries as one value, entry 0 in the low half.
    pub(crate) fn pack_entries(&mut self, e0: ValueId, e1: ValueId, ty: &Ty) -> ValueId {
        let pair = Ty::Array(Box::new(ty.clone()), 2);
        self.emit(pair, Op::Concat(vec![e1, e0]))
    }

    /// The literal a value is, if it is one.
    pub fn const_of(&self, v: ValueId) -> Option<u128> {
        match self.values[v.0 as usize].op {
            Op::Const(k) => Some(k),
            _ => None,
        }
    }

    /// An equality test between two already-matching operands.
    pub fn emit_eq(&mut self, lhs: ValueId, rhs: ValueId) -> ValueId {
        self.emit(Ty::BOOL, Op::Cmp { op: CmpOp::Eq, lhs, rhs })
    }

    /// Whether a read of `mem` at `addr` collides with the write the source has
    /// offered so far, and what that write would put there.
    ///
    /// `None` when nothing has written the memory yet on this path -- the
    /// common case, and worth answering separately rather than as a mux on a
    /// constant-false: a memory that is only read should emit exactly what it
    /// emitted before this existed.
    ///
    /// "So far" is the whole trick. `lower_mem_write` leaves the write port in
    /// the environment, and every branch join muxes it like any other binding,
    /// so at the moment a read is lowered these three values ARE the writes
    /// that precede it in source order -- already collapsed onto the one port
    /// the memory has, guards and all. Nothing has to walk the statements
    /// again to work out which writes came first.
    pub fn pending_write(
        &mut self,
        mem_ix: usize,
        addr: ValueId,
        env: &Env,
    ) -> Option<(ValueId, ValueId)> {
        let mut answer: Option<(ValueId, ValueId)> = None;
        // Earliest first, each later one layered over the top: two writes to
        // one address in a cycle are resolved by source order, and the port
        // logic resolves them the same way, so what a read sees and what the
        // array ends up holding cannot disagree.
        for slot in 0..self.mem_slots[mem_ix] {
            let (we_key, addr_key, data_key) = Self::mem_port_keys(&self.mems[mem_ix].name, slot);
            let Some(we) = env.get(&we_key).and_then(|b| b.value) else {
                continue;
            };
            if matches!(self.values[we.0 as usize].op, Op::Const(0)) {
                continue;
            }
            let Some(waddr) = env.get(&addr_key).and_then(|b| b.value) else {
                continue;
            };
            let Some(wdata) = env.get(&data_key).and_then(|b| b.value) else {
                continue;
            };
            // `m[i] = v` then `m[i]` is the shape this exists for, and both
            // lines lower `i` to the same value. Emitting `(i == i)` would put
            // a comparator in the netlist that is true by construction, so the
            // identity is worth spotting here rather than hoping a synthesizer
            // spots it later.
            let hit = if addr == waddr {
                we
            } else {
                let same = self.emit_eq(addr, waddr);
                self.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: we, rhs: same })
            };
            answer = Some(match answer {
                None => (hit, wdata),
                Some((prev_hit, prev_data)) => {
                    let ty = self.ty_of(prev_data);
                    let data = self.emit(
                        ty,
                        Op::Mux { cond: hit, then_val: wdata, else_val: prev_data },
                    );
                    let any = self.emit(
                        Ty::BOOL,
                        Op::Bin { op: BinOp::Or, lhs: prev_hit, rhs: hit },
                    );
                    (any, data)
                }
            });
        }
        answer
    }

    pub fn coerce_const_pub(&mut self, value: ValueId, want: &Ty) -> Option<ValueId> {
        self.coerce_const(value, want)
    }

    /// Coerces `value` to `want`, inserting an extension or truncation only
    /// when the value is a constant that provably fits. Anything else is the
    /// user's decision and must be written explicitly.
    fn coerce_const(&mut self, value: ValueId, want: &Ty) -> Option<ValueId> {
        let have = self.ty_of(value);
        if have == *want {
            return Some(value);
        }
        let konst = match &self.values[value.0 as usize].op {
            Op::Const(k) => *k,
            _ => return None,
        };
        if !literal_fits(konst, want) {
            return None;
        }
        Some(self.emit(want.clone(), Op::Const(konst)))
    }

    /// Extends both operands of a comparison to a common type wide enough that
    /// the comparison cannot misorder.
    fn extend_for_compare(&mut self, lhs: ValueId, rhs: ValueId) -> (ValueId, ValueId) {
        let lt = self.ty_of(lhs);
        let rt = self.ty_of(rhs);
        let common = comparison_operand_ty(&lt, &rt);
        let lhs = self.extend_to(lhs, &common);
        let rhs = self.extend_to(rhs, &common);
        (lhs, rhs)
    }

    fn extend_to(&mut self, value: ValueId, want: &Ty) -> ValueId {
        let have = self.ty_of(value);
        if have == *want {
            return value;
        }

        // A constant is re-materialised at the wanted width rather than
        // concatenated with zeros. Both are correct; only one is readable.
        // `mem[i]` inside an unrolled `for` is the case that made it worth
        // doing -- the index is a constant whose natural width is one bit, and
        // the address port wants two, so without this the output reads
        // `mem[{{1{1'b0}}, 1'b1}]` where it should read `mem[2'd1]`.
        if !have.is_signed() && !want.is_signed()
            && let Op::Const(k) = self.values[value.0 as usize].op
                && literal_fits(k, want) {
                    return self.emit(want.clone(), Op::Const(k));
                }

        let to = want.bit_width();
        let width_already_matches = have.bit_width() == to;
        let widened = if width_already_matches {
            value
        } else if have.is_signed() {
            self.emit(have.with_width(to), Op::SExt { arg: value, to })
        } else {
            self.emit(have.with_width(to), Op::ZExt { arg: value, to })
        };

        // Only the signedness can still differ, and that is a reinterpretation.
        let signedness_already_matches = self.ty_of(widened) == *want;
        if signedness_already_matches {
            widened
        } else {
            self.emit(want.clone(), Op::Cast { arg: widened })
        }
    }
}

/// Lowers a combinational `fun` into a `Module`.
///
/// Output parameters (`out T`) become output ports, which is how a function
/// expresses a module with more than one result -- k2g_shift.sv has three.
pub fn lower_function(
    map: &SourceMap,
    syms: &Symbols,
    bodies: &HashMap<String, &FunctionDecl>,
    decl: &FunctionDecl,
    sink: &mut DiagSink,
) -> Option<Module> {
    let mut low = Lowerer::new(map, syms, bodies);
    let mut env: Env = Env::new();
    let mut out_ports: Vec<(PortId, String)> = Vec::new();

    for arg in &decl.args.entries {
        let name = anumspan_to_str(&arg.arg_name).to_string();
        let ty = match resolve_type_expr(&arg.type_expr, syms) {
            Ok(t) => t,
            Err(e) => {
                sink.err_at(&arg.arg_name, e.message());
                return None;
            }
        };
        // `inout` is by reference and readable, so at a module boundary it is
        // two ports: the value that came in, and the value going back. Named
        // `x` and `x_out` rather than a Verilog `inout`, which is a tri-state
        // and not what this means. Inside a call it is neither -- the call is
        // inlined and the caller's own variable is updated in place.
        let is_inout = matches!(arg.qualifier, ArgTypeQualifier::Inout);
        let dir = match arg.qualifier {
            ArgTypeQualifier::In => PortDir::In,
            ArgTypeQualifier::Out => PortDir::Out,
            ArgTypeQualifier::Inout => PortDir::In,
            _ => {
                sink.err_at(
                    &arg.arg_name,
                    "pipe parameters need a `process` or `sequence`, not a `fun`",
                );
                return None;
            }
        };

        let port_id = PortId(low.ports.len() as u32);
        low.ports.push(Port { name: name.clone(), dir, ty: ty.clone() });

        if is_inout {
            let back = PortId(low.ports.len() as u32);
            low.ports.push(Port {
                name: format!("{}_out", name),
                dir: PortDir::Out,
                ty: ty.clone(),
            });
            let v = low.emit(ty.clone(), Op::Port(port_id));
            low.values[v.0 as usize].name = Some(name.clone());
            out_ports.push((back, name.clone()));
            // Readable because it arrived with a value, assignable because the
            // caller sees what it leaves.
            env.insert(
                name,
                Binding { value: Some(v), ty, is_output: true, is_mutable: false },
            );
            continue;
        }

        match dir {
            PortDir::In => {
                let v = low.emit(ty.clone(), Op::Port(port_id));
                low.values[v.0 as usize].name = Some(name.clone());
                env.insert(name, Binding::constant(v, ty));
            }
            PortDir::Out => {
                out_ports.push((port_id, name.clone()));
                // An `out` parameter is assignable through `is_output`, not through
                // `is_mutable`: it is written once and read back by the caller.
                env.insert(
                    name,
                    Binding { value: None, ty, is_output: true, is_mutable: false },
                );
            }
        }
    }

    if out_ports.is_empty() {
        let span = map.span_of(&decl.name);
        sink.err_span(
            span,
            "a function lowered to hardware needs at least one `out` parameter",
        );
        sink.push(
            Diag::error(span, "no outputs")
                .with_note("write `result: out u32` in the parameter list"),
        );
        return None;
    }

    lower_stmts(&mut low, &decl.body, &mut env, sink)?;

    let mut drivers = Vec::new();
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

    if sink.has_errors() {
        return None;
    }

    Some(Module {
        calls: low.calls.iter().cloned().collect(),
        params: low.params,
        nets: Vec::new(),
        instances: Vec::new(),
        asserts: low.asserts,
        mems: Vec::new(),
        name: anumspan_to_str(&decl.name).to_string(),
        ports: low.ports,
        values: low.values,
        drivers,
        regs: Vec::new(),
    })
}

/// Lowers a `process` into a clocked module.
///
/// The body is what happens on every cycle. A `var` is a register: reads see
/// the value at the start of the cycle plus whatever the body has already
/// assigned, and the value left at the end is what gets clocked in. That is
/// the SystemVerilog `_next` shadow idiom with the shadow removed -- the first
/// line of k2g_decode's next-state block is literally `pfx_next = pfx;`.
///
/// A conditional assignment therefore becomes a clock enable for free: an
/// unassigned register keeps its value, so `if go then c = c + 1` lowers to
/// `c <= go ? c + 1 : c`, which is what synthesis wants to see.
/// Whether a process body contains a blocking operation, looking through the
/// `loop` that usually wraps it.
///
/// Needed before the body is lowered, because whether a `bram` has anywhere to
/// put its read cycle is decided by whether there are states at all.
fn blocks_somewhere(body: &[PrecResInnerStmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        PrecResInnerStmt::Loop(l) => match &l.repeat_expr {
            PrecResExpr::StmtBlock(b) => b.components.iter().any(crate::ir_fsm::contains_barrier),
            other => crate::ir_fsm::contains_barrier(&PrecResInnerStmt::TailVal(other.clone())),
        },
        other => crate::ir_fsm::contains_barrier(other),
    })
}

/// The half-open range a `for` runs over.
///
/// Two spellings, from desc.md:94. `0..n` is the range itself. `for k in arr`
/// iterates a memory, and means `0..len` -- the binding is the INDEX, because
/// a memory element is reached by subscript and handing back a copy would hide
/// that every read is a port.
fn for_bounds(
    low: &mut Lowerer,
    target: &PrecResExpr,
    at: &crate::lex::AlphanumSpan,
    env: &Env,
    sink: &mut DiagSink,
) -> Option<(u128, u128)> {
    if let PrecResExpr::Span(span) = target {
        let lo = const_operand(low, &span.left, at, "the start of this range", env, sink)?;
        let hi = const_operand(low, &span.right, at, "the end of this range", env, sink)?;
        if hi < lo {
            sink.push(
                Diag::error(
                    low.span(at),
                    format!("this range runs backwards: `{}..{}`", lo, hi),
                )
                .with_note("a `for` range is half-open and counts up"),
            );
            return None;
        }
        return Some((lo, hi));
    }

    if let PrecResExpr::Ref(n) = target {
        let name = anumspan_to_str(n);
        if let Some(Ty::Mem { len, .. }) = env.get(name).map(|b| b.ty.clone()) {
            return Some((0, len as u128));
        }
    }

    sink.push(
        Diag::error(low.span(at), "a `for` iterates a range or an array")
            .with_note("write `for i in 0..n`, or name an array to walk its indices"),
    );
    None
}

/// An expression that has to be known at compile time.
///
/// Lowered rather than folded syntactically, so a constant parameter counts:
/// by the time a body is lowered, `n: u8 = 4` is already an `Op::Const` in the
/// environment, and refusing it would make every parameterised design write
/// its sizes twice.
fn const_operand(
    low: &mut Lowerer,
    expr: &PrecResExpr,
    at: &crate::lex::AlphanumSpan,
    what: &str,
    env: &Env,
    sink: &mut DiagSink,
) -> Option<u128> {
    let v = lower_expr(low, expr, env, sink)?;
    match low.values[v.0 as usize].op {
        Op::Const(k) => Some(k),
        _ => {
            sink.push(
                Diag::error(
                    low.span(at),
                    format!("{} is not known at compile time", what),
                )
                .with_note(
                    "a `for` is unrolled, so its trip count has to be a literal or a constant \
                     parameter",
                ),
            );
            None
        }
    }
}

/// The binary operator behind a compound assignment.
///
/// `~=` is absent on purpose: `~` is unary inversion, so `x ~= y` has no
/// reading that is not a guess between "invert" and "not equal". The lexer
/// accepts it, and lowering says it is not supported rather than picking one.
fn compound_op(kind: crate::lex::AssignStmtKind) -> Option<BuiltinOp> {
    use crate::lex::AssignStmtKind as K;
    Some(match kind {
        K::AddAssign => BuiltinOp::Add,
        K::SubAssign => BuiltinOp::Sub,
        K::MulAssign => BuiltinOp::Mul,
        K::DivAssign => BuiltinOp::Div,
        K::ModAssign => BuiltinOp::Mod,
        K::ShlAssign => BuiltinOp::Shl,
        K::ShrAssign => BuiltinOp::Shr,
        K::AndAssign => BuiltinOp::And,
        K::OrAssign => BuiltinOp::Or,
        K::XorAssign => BuiltinOp::Xor,
        K::PlainAssign | K::InvertAssign => return None,
    })
}

/// How the author wrote it, for the diagnostic.
fn compound_spelling(kind: crate::lex::AssignStmtKind) -> &'static str {
    use crate::lex::AssignStmtKind as K;
    match kind {
        K::AddAssign => "+=",
        K::SubAssign => "-=",
        K::MulAssign => "*=",
        K::DivAssign => "/=",
        K::ModAssign => "%=",
        K::ShlAssign => "<<=",
        K::ShrAssign => ">>=",
        K::AndAssign => "&=",
        K::OrAssign => "|=",
        K::XorAssign => "^=",
        K::InvertAssign => "~=",
        K::PlainAssign => "=",
    }
}

pub fn lower_process(
    map: &SourceMap,
    syms: &Symbols,
    bodies: &HashMap<String, &FunctionDecl>,
    decl: &ProcessDecl,
    sink: &mut DiagSink,
) -> Option<Module> {
    let mut low = Lowerer::new(map, syms, bodies);
    let mut env: Env = Env::new();

    // Clock and reset are implicit.
    low.declare_clock(&mut env);

    for arg in &decl.args.entries {
        let name = anumspan_to_str(&arg.arg_name).to_string();
        if name == "clk" || name == "rst_n" {
            sink.err_at(&arg.arg_name, format!("`{}` is implicit on a process", name));
            return None;
        }
        let kind = classify_param(&low, arg, sink)?;
        // A plain parameter is configuration, folded here and gone. Everything
        // that changes cycle to cycle is a pipe.
        let is_input = match kind {
            ParamKind::Constant => {
                low.declare_constant(arg, &mut env, sink)?;
                continue;
            }
            ParamKind::Pipe { is_input } => is_input,
        };
        let ty = match resolve_type_expr(&arg.type_expr, syms) {
            Ok(t) => t,
            Err(e) => {
                sink.err_at(&arg.arg_name, e.message());
                return None;
            }
        };
        if ty.is_memory() {
            sink.push(
                Diag::error(
                    map.span_of(&arg.arg_name),
                    format!("`{}` is a memory, which cannot be a parameter", name),
                )
                .with_note("declare it inside the process with `var`"),
            );
            return None;
        }
        // A pipe becomes three flat ports. That is the flattening
        // k3g_chan.sv:60 already pre-commits to for the yosys-slang risk --
        // "every process port list flattens to ... triples and the rules stay
        // exactly as written". The triple is a salt each way and the pair of
        // entries; the flattening is what was being promised, not the names.
        let mk = |low: &mut Lowerer, suffix: &str, dir: PortDir, ty: Ty| {
            let id = PortId(low.ports.len() as u32);
            low.ports.push(Port { name: format!("{}_{}", name, suffix), dir, ty });
            id
        };
        let (vdir, rdir, ddir) = if is_input {
            (PortDir::In, PortDir::Out, PortDir::In)
        } else {
            (PortDir::Out, PortDir::In, PortDir::Out)
        };
        let wsalt_port = mk(&mut low, "wsalt", vdir, SALT);
        let rsalt_port = mk(&mut low, "rsalt", rdir, SALT);
        let pair = Ty::Array(Box::new(ty.clone()), 2);
        let data_port = mk(&mut low, "data", ddir, pair.clone());

        let data_value = if is_input {
            let v = low.emit(pair, Op::Port(data_port));
            low.values[v.0 as usize].name = Some(format!("{}_data", name));
            Some(v)
        } else {
            None
        };
        low.pipes.push(PipeInfo {
            name: name.clone(),
            ty,
            is_input,
            wsalt_port,
            rsalt_port,
            data_port,
            data_value,
            item: None,
            salt_reg: None,
            idx: None,
            movable: None,
            used: false,
            sent: None,
            send_guard: None,
            fired: None,
            slot_reg: None,
        });
    }

    // Registers are the leading `var` declarations. Taking them before the
    // body runs is what makes them readable from anywhere in it.
    let mut reg_names: Vec<String> = Vec::new();
    let mut reg_resets: Vec<u128> = Vec::new();
    let mut reg_tys: Vec<Ty> = Vec::new();
    let mut body_start = 0usize;

    for stmt in &decl.body {
        let var_decl = match stmt {
            PrecResInnerStmt::VarDecl(d) if d.is_mutable => d,
            _ => break,
        };
        let name = anumspan_to_str(&var_decl.head_name()).to_string();
        let ty = match &var_decl.ty_expr {
            Some(t) => match resolve_type_expr(t, syms) {
                Ok(t) => t,
                Err(e) => {
                    sink.err_at(&var_decl.head_name(), e.message());
                    return None;
                }
            },
            None => {
                sink.err_at(&var_decl.head_name(), "a register needs a declared type");
                return None;
            }
        };
        // A memory is storage rather than a value, so it takes neither a
        // register slot nor a reset value of its own -- the reset, if there is
        // one, applies to every element.
        if let Ty::Mem { elem, len, kind } = ty.clone() {
            // A block RAM reads SYNCHRONOUSLY: the value arrives a cycle
            // after the address. That cycle has to go somewhere the source can
            // point at, and the only place in this language that can hold one
            // is a state of a blocking `process` -- so a `bram` read is a
            // statement of its own, `let x = mem[i]`, and costs a state. Where
            // there are no states there is nowhere to put it.
            //
            // `bkram` is a different thing again: banked, with conflict
            // minimisation, which is a placement problem rather than a
            // scheduling one.
            if kind == MemKind::BankedRam {
                sink.push(
                    Diag::error(
                        map.span_of(&var_decl.head_name()),
                        "`#[impl(bkram)]` is not supported yet",
                    )
                    .with_note(
                        "banking needs a conflict model; `lutram` and `bram` are available",
                    ),
                );
                return None;
            }
            if kind == MemKind::BlockRam && !blocks_somewhere(&decl.body) {
                sink.push(
                    Diag::error(
                        map.span_of(&var_decl.head_name()),
                        "a `bram` read takes a cycle, and this process has no state to put it in",
                    )
                    .with_note(
                        "a `bram` belongs in a process that blocks, where `let x = mem[i]` is a state of its own; use `lutram` for an asynchronous read",
                    ),
                );
                return None;
            }
            let reset = match &var_decl.assign_val {
                None => None,
                Some(e) => {
                    let v = lower_expr_expecting(&mut low, e, Some(&elem), &env, sink)?;
                    match &low.values[v.0 as usize].op {
                        Op::Const(k) => Some(*k),
                        _ => {
                            sink.push(
                                Diag::error(
                                    map.span_of(&var_decl.head_name()),
                                    "a memory resets every element to the same constant",
                                )
                                .with_note(
                                    "write `@zeroed()`, or leave the initialiser off for a memory that powers up undefined",
                                ),
                            );
                            return None;
                        }
                    }
                }
            };
            // A block RAM has no reset. The reset a `lutram` gets is a loop
            // over every element in the clocked block, which for distributed
            // RAM is what it already is -- but a block RAM that is written on
            // reset is not a block RAM at all: the synthesizer cannot infer
            // one, and what comes out is 8192 flip-flops for a 256x32 table.
            // Refusing beats silently producing that.
            if kind == MemKind::BlockRam && reset.is_some() {
                sink.push(
                    Diag::error(
                        map.span_of(&var_decl.head_name()),
                        format!("`{}` is a block RAM, which cannot be reset", name),
                    )
                    .with_note(
                        "leave the initialiser off; a block RAM powers up from the bitstream, and resetting one costs a flip-flop per bit",
                    ),
                );
                return None;
            }
            low.declare_memory(name, *elem, len, kind, reset, &mut env);
            body_start += 1;
            continue;
        }

        let init = match &var_decl.assign_val {
            Some(e) => e,
            None => {
                sink.err_at(&var_decl.head_name(), "a register needs a reset value");
                return None;
            }
        };
        let reset_value = lower_expr_expecting(&mut low, init, Some(&ty), &env, sink)?;
        let reset = match &low.values[reset_value.0 as usize].op {
            Op::Const(k) => *k,
            _ => {
                sink.err_at(
                    &var_decl.head_name(),
                    "a register's reset value must be a constant",
                );
                return None;
            }
        };

        let held = low.emit(ty.clone(), Op::RegRead(reg_names.len() as u32));
        low.values[held.0 as usize].name = Some(name.clone());
        env.insert(name.clone(), Binding::variable(held, ty.clone()));
        reg_names.push(name);
        reg_resets.push(reset);
        reg_tys.push(ty);
        body_start += 1;
    }

    // What the check is really asking is whether the process is observable.
    // A `port` is a way to be observed -- fewer guarantees than a pipe, and
    // still a wire leaving the module -- so a process made only of them is a
    // boundary block rather than a mistake.
    let has_ports = low
        .ports
        .iter()
        .any(|p| p.name != "clk" && p.name != "rst_n");
    if !has_ports {
        sink.push(
            Diag::error(
                map.span_of(&decl.name),
                "a process needs at least one pipe or `port`",
            )
            .with_note(
                "a process with no channels and no ports computes nothing anything else can see; a plain parameter is folded at compile time and is not one",
            ),
        );
        return None;
    }

    // What the body MEANS, and it is the `loop` that decides.
    //
    // A process body is a program: it runs once and then the process stops
    // (desc.md:37, "may stop (reach terminal state)"). `loop` is what makes it
    // repeat. So there are three shapes, not two:
    //
    //   * a `loop` with blocking `@rcv`/`@send` -- a state machine, one state
    //     per barrier, wrapping back to the first;
    //   * a `loop` with none -- one pass per cycle, forever, which is the
    //     elastic form below;
    //   * a linear body -- one pass, then the terminal state.
    //
    // The middle one used to be spelled by leaving the `loop` out, which gave
    // a linear body the meaning of an infinite one and left the terminal state
    // with no way to be written at all.
    let rest = &decl.body[body_start..];
    let loop_body: Option<Vec<PrecResInnerStmt>> = match rest {
        [PrecResInnerStmt::Loop(l)] => match &l.repeat_expr {
            PrecResExpr::StmtBlock(b) => Some(b.components.clone()),
            _ => {
                sink.err_span(
                    map.span_of(&decl.name),
                    "a `loop` in a process needs an indented body",
                );
                return None;
            }
        },
        _ => None,
    };
    let repeats = loop_body.is_some();
    let body_stmts: Vec<PrecResInnerStmt> = match &loop_body {
        Some(b) => b.clone(),
        None => rest.to_vec(),
    };
    let blocks = body_stmts.iter().any(crate::ir_fsm::contains_barrier);

    if blocks {
        let loop_body = body_stmts.clone();
        return crate::ir_fsm::lower_blocking(
            map, decl, low, env, Vec::new(), reg_names, reg_tys, reg_resets, &loop_body,
            repeats, sink,
        );
    }

    // The nonblocking path has lexical scopes too. Resolve declarations
    // before combinational branch lowering joins the outer environment.
    let globals = env.keys().cloned()
        .chain(low.pipes.iter().map(|p| p.name.clone()))
        .chain(syms.funcs.keys().cloned())
        .chain(syms.structs.keys().cloned())
        .chain(syms.enums.keys().cloned())
        .chain(syms.enums.values().flat_map(|e| e.variants.iter().map(|(n, _)| n.clone())))
        .collect();
    let scoped = crate::ir_scope::resolve(&body_stmts, globals, sink)?;
    low.synthetic_spans = scoped.origins.clone();
    sink.set_synthetic_spans(scoped.origins);
    let body_stmts = scoped.stmts;

    // ---- the generated handshake ------------------------------------------
    //
    // One registered entry per output pipe, which is the shape k3g_expand.sv
    // hand-writes: `uops.valid = busy; iops.ready = !busy || uops.ready`.
    // `valid` is a register output and nothing else can drive it, so channel
    // rule 3 -- `valid` must not depend combinationally on `ready` -- holds by
    // construction rather than by review.
    let mut generated: Vec<Reg> = Vec::new();

    // A linear body runs ONCE. `done` is what makes the process stop: it is
    // low for the first cycle after reset and high forever after, and while it
    // is high nothing transfers and no register moves. With a `loop` there is
    // no such register and the body simply repeats.
    let done: Option<ValueId> = if repeats {
        None
    } else {
        let ix = reg_names.len() + generated.len();
        let d = low.emit(Ty::BOOL, Op::RegRead(ix as u32));
        low.values[d.0 as usize].name = Some("done".to_string());
        let one = low.emit(Ty::BOOL, Op::Const(1));
        generated.push(Reg { name: "done".to_string(), ty: Ty::BOOL, reset: 0, next: one });
        Some(d)
    };
    let running: Option<ValueId> = done.map(|d| low.logical_not(d));

    // A CONSUMER holds two bits and nothing else. `rsalt` is the whole of what
    // it publishes: toggling it IS saying "I took that one", and the producer
    // learns at the next edge rather than through a wire it drives this one.
    for ix in 0..low.pipes.len() {
        if !low.pipes[ix].is_input {
            continue;
        }
        let ty = low.pipes[ix].ty.clone();
        let pname = low.pipes[ix].name.clone();

        let slot = reg_names.len() + generated.len();
        let rsalt_q = low.emit(SALT, Op::RegRead(slot as u32));
        low.name_value_safe(rsalt_q, format!("{}_rsalt_q", pname));
        generated.push(Reg {
            name: format!("{}_rsalt_q", pname),
            ty: SALT,
            reset: 0,
            next: rsalt_q,
        });
        low.pipes[ix].salt_reg = Some(slot);

        let ridx = low.salt_idx(rsalt_q, format!("{}_ridx", pname));
        let pair = low.pipes[ix].data_value.expect("an input pipe has a data value");
        let item = low.entry_of(pair, ridx, &ty);
        low.name_value_safe(item, format!("{}_item", pname));
        low.pipes[ix].item = Some(item);
        low.pipes[ix].idx = Some(ridx);
    }

    // A PRODUCER holds the two entries and two bits. It only ever pushes --
    // there is no pop, because taking is the consumer's business and it does
    // not need this side's help to do it.
    for ix in 0..low.pipes.len() {
        if low.pipes[ix].is_input {
            continue;
        }
        let ty = low.pipes[ix].ty.clone();
        let pname = low.pipes[ix].name.clone();

        let base = reg_names.len() + generated.len();
        let e0 = low.emit(ty.clone(), Op::RegRead(base as u32));
        low.name_value_safe(e0, format!("{}_e0", pname));
        let e1 = low.emit(ty.clone(), Op::RegRead((base + 1) as u32));
        low.name_value_safe(e1, format!("{}_e1", pname));
        // `_q`, because a register named `<p>_wsalt` would collide with the
        // port of that name in the emitted Verilog.
        let wsalt_q = low.emit(SALT, Op::RegRead((base + 2) as u32));
        low.name_value_safe(wsalt_q, format!("{}_wsalt_q", pname));
        generated.push(Reg { name: format!("{}_e0", pname), ty: ty.clone(), reset: 0, next: e0 });
        generated.push(Reg { name: format!("{}_e1", pname), ty: ty.clone(), reset: 0, next: e1 });
        generated.push(Reg {
            name: format!("{}_wsalt_q", pname),
            ty: SALT,
            reset: 0,
            next: wsalt_q,
        });

        // Room is "not full", and full is a comparison of two registers: ours
        // and the consumer's. Neither side's answer passes through the other's
        // combinational logic, which is the property the whole protocol is for.
        let full = low.pipe_full(ix, wsalt_q);
        let accept = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: full });
        low.name_value_safe(accept, format!("{}_room", pname));

        low.pipes[ix].fired = Some(narrow_to_path(&mut low, accept, running));
        low.pipes[ix].slot_reg = Some(base);
    }

    for ix in 0..low.pipes.len() {
        if !low.pipes[ix].is_input {
            continue;
        }
        let rsalt_q = {
            let slot = low.pipes[ix].salt_reg.expect("an input pipe has an rsalt register");
            low.emit(SALT, Op::RegRead(slot as u32))
        };
        let empty = low.pipe_empty(ix, rsalt_q);
        let offered = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: empty });
        // Only this input's availability and process execution matter.
        // Dependencies on other transfers must be explicit in source paths.
        let fired = narrow_to_path(&mut low, offered, running);
        low.values[fired.0 as usize].name = Some(format!("{}_xfer", low.pipes[ix].name));
        low.pipes[ix].fired = Some(fired);
    }

    lower_stmts(&mut low, &body_stmts, &mut env, sink)?;
    if let Some(r) = running {
        for ix in 0..low.asserts.len() {
            let cond = low.asserts[ix].cond;
            let stopped = low.logical_not(r);
            low.asserts[ix].cond = low.emit(
                Ty::BOOL,
                Op::Bin {
                    op: BinOp::Or,
                    lhs: stopped,
                    rhs: cond,
                },
            );
        }
    }
    low.stop_writes = running;
    low.settle_memories(&env);

    let mut drivers = Vec::new();

    // One pass per cycle, so what the body offered IS what the port carries
    // this cycle. A `port` has no handshake to register against.
    //


    for ix in 0..low.pipes.len() {
        let pipe = low.pipes[ix].clone();
        if pipe.is_input {
            // A request is necessary: a pipe only observed with @peek, or
            // never referenced, must retain its item. The path further
            // narrows the request to the branch that actually executes it.
            // Taking IS toggling, so the enable has to be the TRANSFER and not
            // merely this side's willingness. `fired` already carries the
            // "something is offered" half; under valid/ready that half lived at
            // the producer, which is why `ready` alone used to be enough here.
            // It is not enough now: a toggle with nothing to take walks the
            // read index past an entry that was never written.
            //
            // This is literally the value `got` is bound to, which is what
            // makes the claim rule hold by construction rather than by review.
            let claimed = low.pipe_transfer(ix);
            low.name_value_safe(claimed, format!("{}_take", pipe.name));

            let slot = pipe.salt_reg.expect("an input pipe has an rsalt register");
            let rsalt_q = low.emit(SALT, Op::RegRead(slot as u32));
            let ridx = pipe.idx.expect("an input pipe has its index computed");
            let next = low.salt_next(rsalt_q, ridx, claimed);
            generated[slot - reg_names.len()].next = next;
            drivers.push((pipe.rsalt_port, rsalt_q));
            continue;
        }
        let base = pipe.slot_reg.expect("an output pipe has a slot");
        let e0 = low.emit(pipe.ty.clone(), Op::RegRead(base as u32));
        let e1 = low.emit(pipe.ty.clone(), Op::RegRead((base + 1) as u32));
        let wsalt_q = low.emit(SALT, Op::RegRead((base + 2) as u32));

        let sent = match pipe.sent {
            Some(v) => v,
            None => e0,
        };

        // THE OFFER IS ITS OWN PATH, and nothing else. One rule, the same one
        // a process with states follows: a pipe is claimed where the program
        // asks for it, under the condition it asks.
        // ...and there has to be somewhere to put it. `@try_send` answers
        // whether there was, and a body that ignores the answer must not
        // overwrite an entry the consumer has not taken.
        let push = low.pipe_transfer(ix);
        low.name_value_safe(push, format!("{}_push", pipe.name));

        // ONE entry is written and it is the one `widx` names. There is no
        // pop, no skid-to-head move and no second copy of the payload: a
        // producer that only ever pushes cannot get those cases wrong.
        let widx = low.salt_idx(wsalt_q, format!("{}_widx", pipe.name));
        let not_widx = low.logical_not(widx);
        let to_e0 = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: push, rhs: not_widx });
        let to_e1 = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: push, rhs: widx });
        let e0_next =
            low.emit(pipe.ty.clone(), Op::Mux { cond: to_e0, then_val: sent, else_val: e0 });
        let e1_next =
            low.emit(pipe.ty.clone(), Op::Mux { cond: to_e1, then_val: sent, else_val: e1 });
        let wsalt_next = low.salt_next(wsalt_q, widx, push);

        let idx = base - reg_names.len();
        generated[idx].next = e0_next;
        generated[idx + 1].next = e1_next;
        generated[idx + 2].next = wsalt_next;

        // Both are register outputs. Nothing the consumer drives reaches them.
        let pair = low.pack_entries(e0, e1, &pipe.ty);
        drivers.push((pipe.wsalt_port, wsalt_q));
        drivers.push((pipe.data_port, pair));
    }

    let mut regs = Vec::new();
    for (ix, name) in reg_names.iter().enumerate() {
        let next = env
            .get(name)
            .and_then(|b| b.value)
            .expect("a register is bound when it is declared");
        // The terminal state holds whatever the single pass left behind.
        let next = match done {
            None => next,
            Some(d) => {
                let held = low.emit(reg_tys[ix].clone(), Op::RegRead(ix as u32));
                low.emit(
                    reg_tys[ix].clone(),
                    Op::Mux { cond: d, then_val: held, else_val: next },
                )
            }
        };
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

    Some(Module {
        calls: low.calls.iter().cloned().collect(),
        params: low.params,
        nets: Vec::new(),
        instances: Vec::new(),
        asserts: low.asserts,
        mems: low.mems,
        name: anumspan_to_str(&decl.name).to_string(),
        ports: low.ports,
        values: low.values,
        drivers,
        regs,
    })
}


/// `let (item, got) = @try_rcv(p)` and `let (item, present) = @peek(p)`.
///
/// The only tuple-producing forms in the language, so this is deliberately
/// narrow rather than a general tuple type.
///
/// They differ in one thing and it is the whole difference: `@try_rcv`
/// completes a transfer and `@peek` does not. A peek reads `valid` and `data`
/// and asks for nothing, so it does not spend the pipe's one operation for the
/// cycle and the item is still there afterwards.
fn lower_try_rcv_binding(
    low: &mut Lowerer,
    decl: &crate::parse::VarDeclStmt,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    let takes_two = decl.names().len() == 2;
    if !takes_two {
        sink.err_at(&decl.head_name(), "a tuple binding takes exactly two names here");
        return None;
    }
    let init = match &decl.assign_val {
        Some(e) => e,
        None => {
            sink.err_at(&decl.head_name(), "a tuple binding needs an initialiser");
            return None;
        }
    };
    let (pipe_expr, kind) = match init {
        PrecResExpr::Call { base, args } => match &**base {
            PrecResExpr::Builtin(BuiltinOp::TryRecieve) if args.len() == 1 => {
                (&args[0], Some(BuiltinOp::TryRecieve))
            }
            PrecResExpr::Builtin(BuiltinOp::Peek) if args.len() == 1 => {
                (&args[0], Some(BuiltinOp::Peek))
            }
            _ => (init, None),
        },
        _ => (init, None),
    };
    let Some(kind) = kind else {
        sink.err_at(&decl.head_name(), "only `@try_rcv(p)` and `@peek(p)` produce a pair");
        return None;
    };
    let takes = kind == BuiltinOp::TryRecieve;
    let what = if takes { "@try_rcv" } else { "@peek" };
    let pipe_name = match pipe_expr {
        PrecResExpr::Ref(n) => anumspan_to_str(n).to_string(),
        _ => {
            sink.err_at(&decl.head_name(), format!("`{}` needs a pipe name", what));
            return None;
        }
    };
    // A `port in` answers both questions at once and spends nothing doing it.

    let ix = match low.pipes.iter().position(|p| p.name == pipe_name) {
        Some(i) => i,
        None => {
            sink.err_at(&decl.head_name(), format!("`{}` is not a pipe of this process", pipe_name));
            return None;
        }
    };
    if !low.pipes[ix].is_input {
        let verb = if takes { "received from" } else { "peeked at" };
        sink.err_at(
            &decl.head_name(),
            format!("`{}` is an `out` pipe; it cannot be {}", pipe_name, verb),
        );
        return None;
    }
    if low.in_pipeline {
        sink.err_at(&decl.head_name(), "nonblocking buffer operations are not supported in a sequence; use its head `@rcv` and tail `@send`, or use a process");
        return None;
    }
    if takes {
        low.claim_transfer(&pipe_name, low.pipes[ix].used, true, sink)?;
    }
    let ty = low.pipes[ix].ty.clone();
    // The entry this side is owed, not the pair on the wire.
    let data = low.pipes[ix].item.expect("an input pipe has an item");
    // A peek answers "is one being offered", which is the two salts disagreeing
    // and nothing else. A `@try_rcv` answers "did one transfer", which also
    // needs this side to have taken it -- so on a cycle the process is not
    // accepting, the two disagree, and that disagreement is what a peek is for.
    let answer = if takes {
        low.request_pipe(ix, None)
    } else {
        let rsalt_q = {
            let slot = low.pipes[ix].salt_reg.expect("an input pipe has an rsalt register");
            low.emit(SALT, Op::RegRead(slot as u32))
        };
        let empty = low.pipe_empty(ix, rsalt_q);
        let v = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: empty });
        low.name_value_safe(v, format!("{}_present", pipe_name));
        v
    };

    let item = anumspan_to_str(&decl.head_name()).to_string();
    let got = anumspan_to_str(&decl.names()[1]).to_string();
    env.insert(item, Binding::constant(data, ty));
    env.insert(got, Binding::constant(answer, Ty::BOOL));
    Some(())
}

/// One statement, for the FSM scheduler, which lowers segment by segment.
pub fn lower_stmt_pub(
    low: &mut Lowerer,
    stmt: &PrecResInnerStmt,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    lower_stmt(low, stmt, env, sink)
}

pub fn lower_stmts(
    low: &mut Lowerer,
    stmts: &[PrecResInnerStmt],
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    // One bad statement does not stop the rest being checked. A file with
    // three unhandled matches should report three, not the first and nothing
    // else -- the sink already collects them, and the caller only looks at
    // whether it holds any errors.
    let mut all_ok = true;
    for stmt in stmts {
        let ok = lower_stmt(low, stmt, env, sink).is_some();
        all_ok = all_ok && ok;
    }
    if all_ok { Some(()) } else { None }
}

/// The identifier a statement should be blamed on.
///
/// There is no span on a statement node, so this picks the one that reads as
/// the subject: the name being declared, the target being assigned, the
/// function being called. Failing that, the first identifier anywhere inside
/// it -- which is not always the ideal column but is reliably the right LINE,
/// and the line is what a reader needs to find the statement.
pub fn stmt_anchor(stmt: &PrecResInnerStmt) -> Option<AlphanumSpan> {
    match stmt {
        PrecResInnerStmt::VarDecl(d) => Some(d.head_name()),
        PrecResInnerStmt::AssignStmt(a) => expr_anchor(&a.lvalue).or_else(|| expr_anchor(&a.rvalue)),
        PrecResInnerStmt::CallStmt(c) => {
            expr_anchor(&c.base).or_else(|| c.args.iter().find_map(expr_anchor))
        }
        PrecResInnerStmt::IfThenElse(i) => expr_anchor(&i.condition),
        PrecResInnerStmt::MatchStmt(m) => m.scrutinees.iter().find_map(expr_anchor),
        PrecResInnerStmt::ForLoop(f) => Some(f.binding.clone()),
        PrecResInnerStmt::TailVal(e) => expr_anchor(e),
        PrecResInnerStmt::ReturnStmt(e) => e.as_ref().and_then(expr_anchor),
        PrecResInnerStmt::Loop(l) => expr_anchor(&l.repeat_expr),
        // `break` is one keyword and no identifier. The enclosing statement's
        // anchor is still on the stack, so this keeps that rather than
        // replacing it with nothing.
        PrecResInnerStmt::Break => None,
    }
}

/// The first identifier in an expression, left to right.
///
/// A builtin is skipped when it has arguments: `@zext(x, 32)` should point at
/// `x`, not at a `@zext` that has no span of its own anyway.
pub fn expr_anchor(expr: &PrecResExpr) -> Option<AlphanumSpan> {
    match expr {
        PrecResExpr::Ref(n) => Some(n.clone()),
        PrecResExpr::FieldAccess { base, field_name } => {
            expr_anchor(base).or(Some(field_name.clone()))
        }
        PrecResExpr::SubscriptAccess(s) => expr_anchor(&s.base).or_else(|| expr_anchor(&s.index)),
        PrecResExpr::Call { base, args } => {
            expr_anchor(base).or_else(|| args.iter().find_map(expr_anchor))
        }
        PrecResExpr::Splice(parts) => parts.iter().find_map(expr_anchor),
        PrecResExpr::Span(sp) => expr_anchor(&sp.left).or_else(|| expr_anchor(&sp.right)),
        PrecResExpr::StmtBlock(b) => b.components.iter().find_map(stmt_anchor),
        PrecResExpr::Literal(_) | PrecResExpr::Builtin(_) => None,
    }
}

/// Lowers one statement, with its location on the anchor stack for the whole
/// of it.
///
/// A wrapper rather than a push and a pop inside the body: the body returns
/// early from about forty places, and every one of them would have to remember
/// to pop.
fn lower_stmt(
    low: &mut Lowerer,
    stmt: &PrecResInnerStmt,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    let depth = stmt_anchor(stmt).map(|at| low.push_anchor(low.span_of(&at)));
    let result = lower_stmt_at(low, stmt, env, sink);
    if let Some(depth) = depth {
        low.pop_anchor(depth);
    }
    result
}

fn lower_stmt_at(
    low: &mut Lowerer,
    stmt: &PrecResInnerStmt,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    match stmt {
        PrecResInnerStmt::VarDecl(decl) => {
            let name = anumspan_to_str(&decl.head_name()).to_string();

            // A tuple binding takes one name per result. There are two things
            // that produce several: `@try_rcv`, which answers with the item and
            // whether there was one, and a `fun` with several `out` parameters.
            if decl.names().len() > 1 {
                let callee = match &decl.assign_val {
                    Some(PrecResExpr::Call { base, .. }) => match &**base {
                        PrecResExpr::Ref(n) => Some(n.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(callee) = callee {
                    let args = match &decl.assign_val {
                        Some(PrecResExpr::Call { args, .. }) => args.clone(),
                        _ => unreachable!("matched a call above"),
                    };
                    let names = decl.names().to_vec();
                    return crate::ir_match::inline_call_multi(
                        low, &callee, &args, &names, env, sink,
                    );
                }
                return lower_try_rcv_binding(low, decl, env, sink);
            }

            // A single-name binding of a call that also has `inout`
            // parameters needs the statement form: the expression form has no
            // way to write the argument back, because it never sees the
            // caller's environment mutably.
            if let Some(PrecResExpr::Call { base, args }) = &decl.assign_val
                && let PrecResExpr::Ref(callee) = &**base {
                    let has_inouts = low
                        .syms
                        .funcs
                        .get(anumspan_to_str(callee))
                        .is_some_and(|sig| sig.inouts().next().is_some());
                    if has_inouts {
                        let names = vec![decl.head_name()];
                        return crate::ir_match::inline_call_multi(
                            low, callee, args, &names, env, sink,
                        );
                    }
                }

            let declared = match &decl.ty_expr {
                Some(t) => match resolve_type_expr(t, low.syms) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        sink.err_at(&decl.head_name(), e.message());
                        return None;
                    }
                },
                None => None,
            };
            if declared.as_ref().is_some_and(|t| t.is_memory()) {
                sink.push(
                    Diag::error(
                        low.span(&decl.head_name()),
                        format!("`{}` is a memory, which is storage rather than a value", name),
                    )
                    .with_note(
                        "declare it as a `var` at the top of the body, where a `process` and a `sequence` both put one; a `fun` is combinational and has nowhere to keep it",
                    ),
                );
                return None;
            }
            let init = match &decl.assign_val {
                Some(e) => e,
                None => {
                    sink.err_at(
                        &decl.head_name(),
                        "a binding in combinational logic must have an initialiser",
                    );
                    return None;
                }
            };
            let mut value = lower_expr_expecting(low, init, declared.as_ref(), env, sink)?;
            if let Some(want) = &declared {
                let have = low.ty_of(value);
                if have != *want {
                    match low.coerce_const(value, want) {
                        Some(v) => value = v,
                        None => {
                            sink.push(
                                Diag::error(
                                    low.span(&decl.head_name()),
                                    format!(
                                        "`{}` is declared `{}` but its initialiser is `{}`",
                                        name,
                                        want.display(),
                                        have.display()
                                    ),
                                )
                                .with_note(cast_hint(&have, want)),
                            );
                            return None;
                        }
                    }
                }
            }
            let ty = low.ty_of(value);
            low.values[value.0 as usize].name.get_or_insert(name.clone());
            env.insert(
                name,
                Binding { value: Some(value), ty, is_output: false, is_mutable: decl.is_mutable },
            );
            Some(())
        }

        PrecResInnerStmt::AssignStmt(assign) => {
            // An assignment target is a path: a name, optionally followed by
            // field accesses. `uop.cond_reg = arg1` is how k2g_decode builds a
            // 28-field struct, so a bare name is not enough.
            // `m[addr] = v` is a memory write, and the only assignment whose
            // target is not a path.
            if let PrecResExpr::SubscriptAccess(sub) = &assign.lvalue
                && let PrecResExpr::Ref(mem_name) = &sub.base {
                    let is_memory = env
                        .get(anumspan_to_str(mem_name))
                        .is_some_and(|b| b.ty.is_memory());
                    if is_memory {
                        let plain =
                            assign.kind == crate::lex::AssignStmtKind::PlainAssign;
                        if !plain {
                            sink.err_at(mem_name, "compound assignment to a memory is not supported yet");
                            return None;
                        }
                        return lower_mem_write(
                            low, mem_name.clone(), &sub.index, &assign.rvalue, env, sink,
                        );
                    }
                }

            let path = match lvalue_path(&assign.lvalue) {
                Some(p) => p,
                None => {
                    sink.err_span(
                        low.here(),
                        "only a name or a field of one can be assigned",
                    );
                    return None;
                }
            };
            let target = path.base;
            let name = anumspan_to_str(&target).to_string();
            let binding = match env.get(&name) {
                Some(b) => b.clone(),
                None => {
                    sink.err_at(&target, format!("`{}` is not declared", name));
                    return None;
                }
            };
            let base_ty = binding.ty.clone();

            // `let` is a constant. An `out` parameter is assignable without
            // being a variable: it is written once and read back by the
            // caller, which is a different thing from state that changes.
            let assignable = binding.is_mutable || binding.is_output;
            if !assignable {
                let is_constant_parameter = low.params.iter().any(|(n, _, _)| *n == name);
                let diag = if is_constant_parameter {
                    Diag::error(
                        low.span_of(&target),
                        format!("`{}` is a constant parameter and cannot be assigned", name),
                    )
                    .with_note(
                        "a plain parameter is folded at compile time; per-cycle data arrives through a `buffer in` pipe",
                    )
                } else {
                    Diag::error(
                        low.span_of(&target),
                        format!("`{}` is a `let` binding and cannot be assigned", name),
                    )
                    .with_note("declare it `var` if it has to change")
                };
                sink.push(diag);
                return None;
            }
            // `x += y` is `x = x + y`, and desc.md's own `fun` example writes
            // `arg2[0] += arg1`.
            //
            // Desugared here rather than in the parser for two reasons: the
            // read of `x` then goes through the same field-path lowering as
            // the write, so `uop.cond_reg += 1` splices bits exactly the way
            // the plain form does; and an operator with no support yet can
            // still be named in the diagnostic as the author spelled it.
            let desugared;
            let rvalue = if assign.kind == crate::lex::AssignStmtKind::PlainAssign {
                &assign.rvalue
            } else {
                match compound_op(assign.kind) {
                    Some(op) => {
                        desugared = PrecResExpr::Call {
                            base: Box::new(PrecResExpr::Builtin(op)),
                            args: vec![assign.lvalue.clone(), assign.rvalue.clone()],
                        };
                        &desugared
                    }
                    None => {
                        sink.err_at(
                            &target,
                            format!(
                                "`{}` is not supported yet",
                                compound_spelling(assign.kind)
                            ),
                        );
                        return None;
                    }
                }
            };

            // Resolve the step chain to one absolute bit range.
            //
            // Every step is static except possibly the last, which may be an
            // array index this cycle computes. A dynamic step has no bit range
            // to be part of, so nothing may follow it: the whole point of the
            // range is that the steps after it know where they are.
            let mut want = base_ty.clone();
            let mut offset = 0u32;
            let mut span: Option<(u32, u32)> = None;
            let mut dynamic: Option<(ValueId, Ty, u32)> = None;
            for (step_ix, step) in path.steps.iter().enumerate() {
                let field = match step {
                    LvalueStep::Field(f) => f,
                    LvalueStep::Index(index) => {
                        let (elem, n) = match &want {
                            Ty::Array(elem, n) => ((**elem).clone(), *n),
                            other => {
                                sink.push(
                                    Diag::error(
                                        low.here(),
                                        format!("`{}` is not an array", other.display()),
                                    )
                                    .with_note(
                                        "only an `[T; n]` can have an element assigned; a bit of an `uN` is not an lvalue",
                                    ),
                                );
                                return None;
                            }
                        };
                        let w = elem.bit_width();
                        let last = step_ix + 1 == path.steps.len();
                        // A constant index is an ordinary bit range and stays
                        // one wherever it sits: `a[1].f = x` knows exactly
                        // where it is writing.
                        //
                        // `const_eval` reads the syntax, so it misses a
                        // constant parameter and misses the induction variable
                        // of an unrolled `for`. Both arrive as an `Op::Const`
                        // and must not cost a mux tree, so the fallback lowers
                        // once and asks.
                        let konst = match const_eval(index) {
                            Ok(k) => Some(k),
                            Err(_) => {
                                let idx = lower_expr(low, index, env, sink)?;
                                match low.values[idx.0 as usize].op {
                                    Op::Const(k) => Some(k),
                                    _ => {
                                        if !last {
                                            sink.push(
                                                Diag::error(
                                                    low.here(),
                                                    "a computed index must be the last step of an assignment target",
                                                )
                                                .with_note(
                                                    "bind the element first -- `let e = a[i]` -- then assign its parts and write `a[i] = e` back",
                                                ),
                                            );
                                            return None;
                                        }
                                        let idx_ty = low.ty_of(idx);
                                        if idx_ty.is_signed() {
                                            sink.push(
                                                Diag::error(
                                                    low.here(),
                                                    format!(
                                                        "an index must be unsigned, found `{}`",
                                                        idx_ty.display()
                                                    ),
                                                )
                                                .with_note("convert with `@unsigned(x)`"),
                                            );
                                            return None;
                                        }
                                        dynamic = Some((idx, elem.clone(), n));
                                        want = elem;
                                        continue;
                                    }
                                }
                            }
                        };
                        let k = konst.expect("the dynamic path continued above");
                        if k >= n as u128 {
                            sink.push(
                                Diag::error(
                                    low.here(),
                                    format!(
                                        "element {} is out of bounds for `{}`",
                                        k,
                                        want.display()
                                    ),
                                )
                                .with_note(format!("it has {} element(s), indexed from 0", n)),
                            );
                            return None;
                        }
                        let k = k as u32;
                        span = Some((offset + (k + 1) * w - 1, offset + k * w));
                        offset += k * w;
                        want = elem;
                        continue;
                    }
                };
                let struct_name = match &want {
                    Ty::Struct { name, .. } => name.clone(),
                    other => {
                        sink.err_at(
                            field,
                            format!("`{}` has no fields", other.display()),
                        );
                        return None;
                    }
                };
                let def = low.syms.structs.get(&struct_name)?.clone();
                let fname = anumspan_to_str(field);
                let (hi, lo) = match def.field_range(fname) {
                    Some(r) => r,
                    None => {
                        let known: Vec<&str> =
                            def.fields.iter().map(|(n, _)| n.as_str()).collect();
                        sink.push(
                            Diag::error(
                                low.span(field),
                                format!("`{}` has no field `{}`", struct_name, fname),
                            )
                            .with_note(format!("fields are: {}", known.join(", "))),
                        );
                        return None;
                    }
                };
                want = def.field_ty(fname).expect("range implies a type");
                span = Some((offset + hi, offset + lo));
                offset += lo;
            }

            let mut value = lower_expr_expecting(low, rvalue, Some(&want), env, sink)?;
            let have = low.ty_of(value);
            if have != want {
                match low.coerce_const(value, &want) {
                    Some(v) => value = v,
                    None => {
                        sink.push(
                            Diag::error(
                                low.span(&target),
                                format!(
                                    "cannot assign `{}` to `{}`, which is `{}`",
                                    have.display(),
                                    name,
                                    want.display()
                                ),
                            )
                            .with_note(cast_hint(&have, &want)),
                        );
                        return None;
                    }
                }
            }

            // A computed index has no bit range, so the read-modify-write
            // below cannot place it. It becomes a whole-array value instead:
            // every element muxed against the index, which is the same shape
            // the synthesizer would build from a `case` and does not need a
            // procedural block to express.
            //
            // `+:` is what a computed READ lowers to (`lower_array_index`),
            // and there is no `+:` on the left of an assignment in a pure
            // value graph -- the array is a wire here, not a variable.
            if let Some((idx, elem, n)) = dynamic {
                let array_ty = Ty::Array(Box::new(elem.clone()), n);
                let w = elem.bit_width();
                let current = match env.get(&name).and_then(|b| b.value) {
                    Some(v) => v,
                    None => {
                        sink.err_at(
                            &target,
                            format!("`{}` is assigned an element before it has a value", name),
                        );
                        return None;
                    }
                };
                let whole = match span {
                    None => current,
                    Some((hi, lo)) => low.emit(array_ty.clone(), Op::Slice { arg: current, hi, lo }),
                };
                let idx_ty = low.ty_of(idx);
                // High-to-low, matching `Op::Concat` and the packed layout the
                // reads use: element k is bits [(k+1)*w-1 : k*w].
                let mut parts = Vec::with_capacity(n as usize);
                for k in (0..n).rev() {
                    let old_k =
                        low.emit(elem.clone(), Op::Slice { arg: whole, hi: (k + 1) * w - 1, lo: k * w });
                    let konst = low.emit(idx_ty.clone(), Op::Const(k as u128));
                    let hit = low.emit(
                        Ty::BOOL,
                        Op::Cmp { op: CmpOp::Eq, lhs: idx, rhs: konst },
                    );
                    parts.push(low.emit(
                        elem.clone(),
                        Op::Mux { cond: hit, then_val: value, else_val: old_k },
                    ));
                }
                value = low.emit(array_ty, Op::Concat(parts));
            }

            // Writing a field is a read-modify-write on the whole value: keep
            // the bits above and below, splice the new ones in between.
            let new_base = match span {
                None => value,
                Some((hi, lo)) => {
                    let current = match env.get(&name).and_then(|b| b.value) {
                        Some(v) => v,
                        None => {
                            sink.err_at(
                                &target,
                                format!("`{}` is assigned a field before it has a value", name),
                            );
                            return None;
                        }
                    };
                    let total = base_ty.bit_width();
                    let mut parts = Vec::new();
                    let has_bits_above = hi + 1 < total;
                    if has_bits_above {
                        parts.push(low.emit(
                            Ty::UInt(total - hi - 1),
                            Op::Slice { arg: current, hi: total - 1, lo: hi + 1 },
                        ));
                    }
                    parts.push(value);
                    let has_bits_below = lo > 0;
                    if has_bits_below {
                        parts.push(low.emit(
                            Ty::UInt(lo),
                            Op::Slice { arg: current, hi: lo - 1, lo: 0 },
                        ));
                    }
                    low.emit(base_ty.clone(), Op::Concat(parts))
                }
            };

            if let Some(b) = env.get_mut(&name) {
                b.value = Some(new_base);
            }
            Some(())
        }

        PrecResInnerStmt::IfThenElse(ite) => {
            let cond = lower_expr(low, &ite.condition, env, sink)?;
            let cond_ty = low.ty_of(cond);
            if cond_ty != Ty::BOOL {
                sink.err_span(
                    low.here(),
                    format!(
                        "an `if` condition must be `u1`, found `{}`",
                        cond_ty.display()
                    ),
                );
                return None;
            }

            // Both arms start from the same write-port count, which is what
            // makes them share a slot and so share a port.
            let base = low.mem_slot_counts();

            let mut then_env = env.clone();
            let depth = low.push_cond(cond, true);
            lower_branch(low, &ite.then_case, &mut then_env, sink)?;
            low.pop_path(depth);
            let then_n = low.mem_slot_counts();

            low.set_mem_slot_counts(&base);
            let mut else_env = env.clone();
            if let Some(else_case) = &ite.else_case {
                let depth = low.push_cond(cond, false);
                lower_branch(low, else_case, &mut else_env, sink)?;
                low.pop_path(depth);
            }
            let else_n = low.mem_slot_counts();

            // SSA join: any binding the two arms disagree about becomes a mux.
            //
            // Write ports are joined separately, below: they are created as
            // the arms run, so the keys an arm added are not in `env` here to
            // be walked, and the two arms can have added different numbers.
            let names: Vec<String> = env.keys().filter(|k| !k.contains('#')).cloned().collect();
            for name in names {
                let t = then_env.get(&name).and_then(|b| b.value);
                let e = else_env.get(&name).and_then(|b| b.value);
                let merged = match (t, e) {
                    (Some(t), Some(e)) if t == e => Some(t),
                    (Some(t), Some(e)) => {
                        let tt = low.ty_of(t);
                        let et = low.ty_of(e);
                        if tt != et {
                            sink.err_span(
                                low.here(),
                                format!(
                                    "`{}` is `{}` on one branch and `{}` on the other",
                                    name,
                                    tt.display(),
                                    et.display()
                                ),
                            );
                            return None;
                        }
                        Some(low.emit(
                            tt,
                            Op::Mux { cond, then_val: t, else_val: e },
                        ))
                    }
                    // Assigned on only one arm. For an output that is a latch,
                    // which combinational hardware cannot express.
                    (Some(_), None) | (None, Some(_)) => {
                        sink.err_span(
                            low.here(),
                            format!(
                                "`{}` is assigned on only one branch of this `if`",
                                name
                            ),
                        );
                        sink.push(Diag::error(low.here(), "incomplete assignment")
                            .with_note(
                                "combinational logic has no memory, so every branch must assign it; add an `else`",
                            ));
                        return None;
                    }
                    (None, None) => None,
                };
                if let Some(v) = merged
                    && let Some(b) = env.get_mut(&name) {
                        b.value = Some(v);
                    }
            }
            low.join_write_slots(cond, &base, &then_env, &then_n, &else_env, &else_n, env);
            Some(())
        }

        PrecResInnerStmt::CallStmt(call) => {
            // A bare `@try_send(p, v)` discards the answer. Binding it would be
            // the only way to write one inside an `if`, and a binding made on
            // one branch and not the other is rejected by the SSA join.
            if let PrecResExpr::Builtin(BuiltinOp::TrySend) = &call.base {
                lower_builtin(low, BuiltinOp::TrySend, &call.args, env, sink)?;
                return Some(());
            }
            // A bare `@drop(p)` discards the answer as well as the item.
            if let PrecResExpr::Builtin(BuiltinOp::Drop) = &call.base {
                lower_builtin(low, BuiltinOp::Drop, &call.args, env, sink)?;
                return Some(());
            }
            let checking = match &call.base {
                PrecResExpr::Builtin(BuiltinOp::Assert) => Some(false),
                PrecResExpr::Builtin(BuiltinOp::Fatal) => Some(true),
                _ => None,
            };
            if let Some(is_fatal) = checking {
                return lower_assert(low, &call.args, is_fatal, env, sink);
            }
            // `f(a, acc)` where `acc` is `inout`: the call has no result to
            // bind because its result went back into its argument.
            if let PrecResExpr::Ref(callee) = &call.base {
                return crate::ir_match::inline_call_effect(low, callee, &call.args, env, sink);
            }
            sink.err_span(
                low.here(),
                "this statement has no effect in combinational logic",
            );
            None
        }

        PrecResInnerStmt::TailVal(_) => {
            sink.err_span(
                low.here(),
                "this statement has no effect in combinational logic",
            );
            None
        }

        PrecResInnerStmt::ReturnStmt(_) => {
            // Results leave through `out` parameters, so a bare `return` is a
            // no-op terminator and a returning one has nowhere to go.
            Some(())
        }

        PrecResInnerStmt::MatchStmt(m) => crate::ir_match::lower_match(low, m, env, sink),

        // Reached only where there are no states to break out of. A `break`
        // in a blocking `loop` never gets here: the scheduler turns it into an
        // edge to the terminal state.
        PrecResInnerStmt::Break => {
            sink.push(
                Diag::error(low.here(), "there is nothing here to `break` out of")
                    .with_note(
                        "`break` stops a `loop` that blocks, by leaving its state machine. A loop with no `@rcv` or `@send` is the per-cycle form and has no states; a linear body already runs once and stops",
                    ),
            );
            None
        }

        PrecResInnerStmt::Loop(_) => {
            // A `loop` nested inside another one is scheduled -- its back edge
            // is a state graph, and `ir_fsm` builds those. Reaching HERE means
            // there is no state graph to put it in: a `fun`, a `sequence`, or a
            // process whose body never blocks and is therefore one pass per
            // cycle.
            sink.push(
                Diag::error(
                    low.here(),
                    "a `loop` needs a `process` that blocks, so its repetition has a cycle to spend",
                )
                .with_note(
                    "a `fun` and a `sequence` settle once; a process with no `@rcv`/`@send` has no states to come back to",
                ),
            );
            None
        }

        // `for i in 0..n` unrolls. There is no loop counter in hardware unless
        // something asks for one, and a `for` with a static trip count is not
        // asking -- it is a way to write the same wiring n times without
        // writing it n times.
        //
        // The bound is folded rather than parsed, so `0..N` with `N` a
        // constant parameter works: a constant parameter is already an
        // `Op::Const` by the time a body is lowered.
        PrecResInnerStmt::ForLoop(f) => {
            let sync_mem = |name: &str| low.mem_index(name).filter(|ix| low.mems[*ix].kind != ty::MemKind::LutRam);
            if crate::ir_fsm::needs_states(stmt, &sync_mem) {
                sink.err_at(&f.binding, "a `for` body must be combinational; blocking transfers, synchronous reads, `loop`, and `break` require an explicit process loop");
                return None;
            }
            let name = anumspan_to_str(&f.binding).to_string();
            let body = match &f.body {
                PrecResExpr::StmtBlock(b) => &b.components,
                _ => {
                    sink.err_at(&f.binding, "expected an indented body after this `for`");
                    return None;
                }
            };

            let (lo, hi) = for_bounds(low, &f.target, &f.binding, env, sink)?;

            // A trip count that is a mistake rather than a design: unrolling
            // it would build the logic before anyone noticed, and the report
            // would be a compiler that stopped responding.
            const MAX_TRIP: u128 = 4096;
            if hi.saturating_sub(lo) > MAX_TRIP {
                sink.push(
                    Diag::error(
                        low.span(&f.binding),
                        format!("this `for` would unroll {} times", hi - lo),
                    )
                    .with_note(format!(
                        "a `for` is unrolled, so every iteration is its own logic; the limit is {}",
                        MAX_TRIP
                    )),
                );
                return None;
            }

            let shadowed = env.get(&name).cloned();
            for i in lo..hi {
                // Typed as narrowly as the value allows, which is what an
                // unsized literal does -- so `acc + i` adopts `acc`'s width
                // rather than demanding a cast at every use.
                let ty = Ty::UInt(ty::bits_for(i));
                let v = low.emit(ty.clone(), Op::Const(i));
                // Deliberately unnamed: a named value gets its own `wire`, and
                // eight wires holding the numbers 0 to 7 is not what anybody
                // wants to read in the output.
                env.insert(name.clone(), Binding::constant(v, ty));
                lower_stmts(low, body, env, sink)?;
            }
            match shadowed {
                Some(b) => {
                    env.insert(name, b);
                }
                None => {
                    env.remove(&name);
                }
            }
            Some(())
        }
    }
}

/// A branch arm is either an inline statement or an indented block.
pub fn lower_branch(
    low: &mut Lowerer,
    arm: &PrecResExpr,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    match arm {
        PrecResExpr::StmtBlock(block) => lower_stmts(low, &block.components, env, sink),
        other => {
            sink.err_span(
                low.here(),
                format!("expected assignments in this branch, found {:?}", other),
            );
            None
        }
    }
}

pub fn cast_hint_pub(have: &Ty, want: &Ty) -> String {
    cast_hint(have, want)
}

fn cast_hint(have: &Ty, want: &Ty) -> String {
    let only_signedness_differs = have.bit_width() == want.bit_width();
    if only_signedness_differs {
        return format!(
            "convert with `@signed(x)` or `@unsigned(x)` to reach `{}`",
            want.display()
        );
    }
    let needs_widening = have.bit_width() < want.bit_width();
    if needs_widening {
        let cast = if have.is_signed() { "@sext" } else { "@zext" };
        format!("widen with `{}(x, {})`", cast, want.bit_width())
    } else {
        format!("narrow with `@trunc(x, {})`", want.bit_width())
    }
}


/// A name, plus the chain of fields written after it.
/// One step of an assignment target, as written.
///
/// `uop.cond_reg` is one `Field`; `line.words[1]` is a `Field` then an
/// `Index`. Both resolve to a bit range of the same base name, which is what
/// makes a write to either a splice rather than a separate storage location.
enum LvalueStep {
    Field(AlphanumSpan),
    Index(PrecResExpr),
}

struct LvaluePath {
    base: AlphanumSpan,
    /// As written, outermost first: `u.a.b` gives `[a, b]`.
    steps: Vec<LvalueStep>,
}

/// A port named where a value was expected.
///

fn lvalue_path(expr: &PrecResExpr) -> Option<LvaluePath> {
    match expr {
        PrecResExpr::Ref(base) => Some(LvaluePath { base: base.clone(), steps: Vec::new() }),
        PrecResExpr::FieldAccess { base, field_name } => {
            let mut path = lvalue_path(base)?;
            path.steps.push(LvalueStep::Field(field_name.clone()));
            Some(path)
        }
        // `a[k] = v`. A memory write is caught before this and never arrives
        // here: `m[i] = v` reaches the one write port, where `a[i] = v` on an
        // array VALUE is a splice of the value the name already holds.
        PrecResExpr::SubscriptAccess(sub) => {
            let mut path = lvalue_path(&sub.base)?;
            path.steps.push(LvalueStep::Index(sub.index.clone()));
            Some(path)
        }
        _ => None,
    }
}

/// Lowers an expression that has a known expected type.
///
/// Only `@zeroed()` needs this: it has no type of its own and takes the one
/// the context wants, exactly as SystemVerilog's `'0` does. Everything else
/// ignores the hint, so the ordinary path stays untouched.
pub fn lower_expr_expecting(
    low: &mut Lowerer,
    expr: &PrecResExpr,
    want: Option<&Ty>,
    env: &Env,
    sink: &mut DiagSink,
) -> Option<ValueId> {
    if let PrecResExpr::Call { base, args } = expr
        && let PrecResExpr::Builtin(b) = &**base {
            match b {
                BuiltinOp::Zeroed if args.is_empty() => {
                    return match want {
                        Some(ty) => Some(low.emit(ty.clone(), Op::Const(0))),
                        None => {
                            sink.err_span(
                                low.here(),
                                "`@zeroed()` needs a type from its context",
                            );
                            None
                        }
                    };
                }
                // `@cast(x)` reinterprets the bits as the type the context
                // wants. The width has to match exactly -- changing it is what
                // @zext/@sext/@trunc are for, and doing it silently here is
                // the coercion this type system exists to prevent.
                BuiltinOp::Cast if args.len() == 1 => {
                    let ty = match want {
                        Some(t) => t.clone(),
                        None => {
                            sink.err_span(
                                low.here(),
                                "`@cast()` needs a type from its context",
                            );
                            return None;
                        }
                    };
                    let arg = lower_expr(low, &args[0], env, sink)?;
                    let have = low.ty_of(arg);
                    let widths_agree = have.bit_width() == ty.bit_width();
                    if !widths_agree {
                        sink.push(
                            Diag::error(
                                low.here(),
                                format!(
                                    "`@cast` cannot change width: `{}` is {} bits, `{}` is {}",
                                    have.display(),
                                    have.bit_width(),
                                    ty.display(),
                                    ty.bit_width()
                                ),
                            )
                            .with_note("use `@zext`, `@sext` or `@trunc` to change the width"),
                        );
                        return None;
                    }
                    return Some(low.emit(ty, Op::Cast { arg }));
                }
                _ => {}
            }
        }
    lower_expr(low, expr, env, sink)
}

pub fn lower_expr(
    low: &mut Lowerer,
    expr: &PrecResExpr,
    env: &Env,
    sink: &mut DiagSink,
) -> Option<ValueId> {
    match expr {
        PrecResExpr::Ref(span) => {
            let name = anumspan_to_str(span);
            // A local shadows a variant, so the environment is consulted first.
            let is_local = env.contains_key(name);
            if !is_local
                && let Some((def, discriminant)) = low.syms.lookup_variant(name) {
                    // A variant that carries something is not a value until it
                    // is given one. `Read` alone would be `Read` with an
                    // undefined address, which is exactly the bug the payload
                    // exists to prevent.
                    if let Some(payload) = def.payload_of(name) {
                        let (enum_name, payload) = (def.name.clone(), payload.display());
                        sink.push(
                            Diag::error(
                                low.span_of(span),
                                format!("`{}` carries a `{}`, so it needs one", name, payload),
                            )
                            .with_note(format!(
                                "write `{}(<{}>)`; a bare `{}` would leave the payload undefined and `{}` gives no way to say it is",
                                name, payload, name, enum_name
                            )),
                        );
                        return None;
                    }
                    let ty = def.ty();
                    let whole = def.bare_value(discriminant);
                    return Some(low.emit(ty, Op::Const(whole)));
                }
            match env.get(name) {
                Some(Binding { value: Some(v), .. }) => Some(*v),
                Some(Binding { is_output: true, .. }) => {
                    sink.err_at(span, format!("`{}` is an output and has not been assigned yet", name));
                    None
                }
                Some(_) => {
                    sink.err_at(span, format!("`{}` is used before it is assigned", name));
                    None
                }
                None => {
                    sink.err_at(span, format!("`{}` is not defined", name));
                    None
                }
            }
        }

        PrecResExpr::Literal(Literal::IntLiteral { value, width }) => {
            let ty = match width {
                Some(w) => Ty::UInt(*w),
                // Unsized: provisionally as narrow as possible. The operand
                // rules re-materialise it at the width of its partner.
                None => Ty::UInt(ty::bits_for(*value)),
            };
            if let Some(w) = width
                && !literal_fits(*value, &Ty::UInt(*w)) {
                    sink.err_span(
                        low.here(),
                        format!("literal {} does not fit in {} bits", value, w),
                    );
                    return None;
                }
            Some(low.emit(ty, Op::Const(*value)))
        }

        // `{a, b, c}` -- the operands laid down high-to-low, which is the
        // order they are written in and the order `Op::Concat` emits.
        //
        // The result is `uN` however the parts were typed. A concatenation has
        // no sign: the top bit of the leftmost operand is just a bit once
        // something is below it, and an `iN` that wanted to keep its sign
        // wanted `@sext`, not this. Width is the sum, so an operand of the
        // wrong width is a width error at the USE, naming a number the reader
        // can check against the braces.
        PrecResExpr::Splice(parts) => {
            let mut values = Vec::with_capacity(parts.len());
            let mut total = 0u32;
            for p in parts {
                let v = lower_expr(low, p, env, sink)?;
                total += low.ty_of(v).bit_width();
                values.push(v);
            }
            Some(low.emit(Ty::UInt(total), Op::Concat(values)))
        }

        PrecResExpr::Literal(other) => {
            sink.err_span(
                low.here(),
                format!("{:?} cannot be lowered to hardware", other),
            );
            None
        }

        PrecResExpr::SubscriptAccess(sub) => lower_subscript(low, sub, env, sink),

        PrecResExpr::FieldAccess { base, field_name } => {
            let value = lower_expr(low, base, env, sink)?;
            let base_ty = low.ty_of(value);
            let struct_name = match &base_ty {
                Ty::Struct { name, .. } => name.clone(),
                other => {
                    sink.err_at(
                        field_name,
                        format!("`{}` has no fields", other.display()),
                    );
                    return None;
                }
            };
            let field = anumspan_to_str(field_name);
            let def = low.syms.structs.get(&struct_name)?.clone();
            let (hi, lo) = match def.field_range(field) {
                Some(r) => r,
                None => {
                    let known: Vec<&str> =
                        def.fields.iter().map(|(n, _)| n.as_str()).collect();
                    sink.push(
                        Diag::error(
                            low.span_of(field_name),
                            format!("`{}` has no field `{}`", struct_name, field),
                        )
                        .with_note(format!("fields are: {}", known.join(", "))),
                    );
                    return None;
                }
            };
            let field_ty = def.field_ty(field).expect("range implies a type");
            let bits = low.emit(Ty::UInt(hi - lo + 1), Op::Slice { arg: value, hi, lo });
            // The slice is a bag of bits; give it back the field's own type so
            // an enum field stays matchable and a signed field stays signed.
            let needs_retype = field_ty != low.ty_of(bits);
            if needs_retype {
                Some(low.emit(field_ty, Op::Cast { arg: bits }))
            } else {
                Some(bits)
            }
        }

        PrecResExpr::Call { base, args } => {
            match &**base {
                PrecResExpr::Builtin(op) => lower_builtin(low, *op, args, env, sink),
                PrecResExpr::Ref(name) => {
                    // `MyStruct(a, b)` builds a struct. Call syntax is reused
                    // so the grammar needs no separate literal form, and a
                    // struct name cannot also be a function name because the
                    // symbol table rejects duplicates.
                    // `Read(addr)` builds an enum value: the tag in the high
                    // bits, the payload below it. Call syntax again, for the
                    // same reason, and a variant name cannot also be a
                    // function name because the symbol table rejects that too.
                    let is_variant = low.syms.lookup_variant(anumspan_to_str(name)).is_some();
                    if is_variant {
                        return build_enum_value(low, name, args, env, sink);
                    }
                    let is_struct = low
                        .syms
                        .structs
                        .contains_key(anumspan_to_str(name));
                    if is_struct {
                        build_struct_value(low, name, args, env, sink)
                    } else {
                        crate::ir_match::inline_call(low, name, args, env, sink)
                    }
                }
                _ => {
                    sink.err_span(
                        low.here(),
                        "this is not something that can be called",
                    );
                    None
                }
            }
        }

        // A block holding one `if`, in a position that wants a value.
        //
        // `let x = if a then b else c` on one line resolves to `Select` in the
        // parser. Wrap the same text onto a continuation line and the indented
        // form makes it a block with an `IfThenElse` statement inside -- so the
        // meaning of the text changed with a line break, and lowering refused
        // it. The arms are already expressions (a chained `else if` resolves as
        // one), so the block is a value and unwrapping it is the whole fix.
        //
        // Every other block shape is refused by the PARSER before it gets
        // here -- an `if` with no `else`, two statements, two `if`s -- so this
        // arm handles the one that arrives and anything else falls through to
        // the refusal below rather than to a message that cannot be produced.
        PrecResExpr::StmtBlock(block)
            if matches!(
                &block.components[..],
                [PrecResInnerStmt::IfThenElse(ite)] if ite.else_case.is_some()
            ) =>
        {
            let [PrecResInnerStmt::IfThenElse(ite)] = &block.components[..] else {
                unreachable!("guarded above")
            };
            let else_case = ite.else_case.clone().expect("guarded above");
            lower_builtin(
                low,
                BuiltinOp::Select,
                &[ite.condition.clone(), ite.then_case.clone(), else_case],
                env,
                sink,
            )
        }

        other => {
            sink.err_span(
                low.here(),
                format!("cannot lower {:?} to hardware yet", other),
            );
            None
        }
    }
}

/// `a[i]` and `a[hi..lo]` on a packed `[T; n]`, in elements.
///
/// Element `k` occupies bits `[(k+1)*w-1 : k*w]`, which is how a packed array
/// lays out in SystemVerilog too, so a DDL `[u32; 4]` and its `.svh`
/// counterpart still meet at a module boundary.
fn lower_array_index(
    low: &mut Lowerer,
    sub: &crate::parse::SubscriptAccess,
    base: ValueId,
    elem: &Ty,
    n: u32,
    env: &Env,
    sink: &mut DiagSink,
) -> Option<ValueId> {
    let w = elem.bit_width();
    let display = Ty::Array(Box::new(elem.clone()), n).display();

    let out_of_bounds = |low: &Lowerer, sink: &mut DiagSink, k: u128| {
        sink.push(
            Diag::error(
                low.here(),
                format!("element {} is out of bounds for `{}`", k, display),
            )
            .with_note(format!("it has {} element(s), indexed from 0", n)),
        );
    };

    // `a[hi..lo]` -- a run of elements, which is a shorter array.
    if let PrecResExpr::Span(range) = &sub.index {
        let hi = const_eval(&range.left).ok()?;
        let lo = const_eval(&range.right).ok()?;
        if hi < lo {
            sink.err_span(low.here(), format!("this range runs backwards: `{}..{}`", hi, lo));
            return None;
        }
        if hi >= n as u128 {
            out_of_bounds(low, sink, hi);
            return None;
        }
        let (hi, lo) = (hi as u32, lo as u32);
        let ty = Ty::Array(Box::new(elem.clone()), hi - lo + 1);
        return Some(low.emit(ty, Op::Slice { arg: base, hi: (hi + 1) * w - 1, lo: lo * w }));
    }

    // `a[k]` -- one element, at a constant index.
    let element = |low: &mut Lowerer, sink: &mut DiagSink, k: u128| {
        if k >= n as u128 {
            out_of_bounds(low, sink, k);
            return None;
        }
        let k = k as u32;
        Some(low.emit(elem.clone(), Op::Slice { arg: base, hi: (k + 1) * w - 1, lo: k * w }))
    };
    if let Ok(k) = const_eval(&sub.index) {
        return element(low, sink, k);
    }

    // Lowered once, and only then asked whether it folded: `const_eval` works
    // on the syntax, so it misses a constant parameter and misses the
    // induction variable of an unrolled `for`, and both arrive here as an
    // `Op::Const`. Lowering twice to find that out would double whatever the
    // index expression does on the way.
    let idx = lower_expr(low, &sub.index, env, sink)?;
    if let Op::Const(k) = low.values[idx.0 as usize].op {
        return element(low, sink, k);
    }

    // `a[i]` -- one element, at a computed index, which is a `+:` part-select
    // whose base is the index scaled by the element width.
    //
    // The index is widened first: `i` addressing 32 elements is 5 bits and the
    // base it has to produce is 10, so scaling in the index's own width would
    // shift the top of it off.
    let idx_ty = low.ty_of(idx);
    if idx_ty.is_signed() {
        sink.push(
            Diag::error(
                low.here(),
                format!("an index must be unsigned, found `{}`", idx_ty.display()),
            )
            .with_note("convert with `@unsigned(x)`"),
        );
        return None;
    }
    let addr_w = u32::BITS - (n * w - 1).leading_zeros();
    let wide = low.emit(Ty::UInt(addr_w), Op::ZExt { arg: idx, to: addr_w });
    // A shift where the element width allows one. `*` is a multiplier as far
    // as GowinSynthesis is concerned, and an address is the last place to
    // spend a DSP on a constant.
    let scaled = if w.is_power_of_two() {
        let shift = low.emit(Ty::UInt(addr_w), Op::Const(w.trailing_zeros() as u128));
        low.emit(Ty::UInt(addr_w), Op::Bin { op: BinOp::Shl, lhs: wide, rhs: shift })
    } else {
        let width = low.emit(Ty::UInt(addr_w), Op::Const(w as u128));
        low.emit(Ty::UInt(addr_w), Op::Bin { op: BinOp::Mul, lhs: wide, rhs: width })
    };
    Some(low.emit(elem.clone(), Op::DynSlice { arg: base, base: scaled, width: w }))
}

fn lower_subscript(
    low: &mut Lowerer,
    sub: &crate::parse::SubscriptAccess,
    env: &Env,
    sink: &mut DiagSink,
) -> Option<ValueId> {
    // `m[i]` where `m` is a memory is a read of the array, not a bit select.
    if let PrecResExpr::Ref(mem_name) = &sub.base {
        let name = anumspan_to_str(mem_name);
        let is_memory = env.get(name).is_some_and(|b| b.ty.is_memory());
        if is_memory {
            return lower_mem_read(low, mem_name.clone(), &sub.index, env, sink);
        }
    }

    let base = lower_expr(low, &sub.base, env, sink)?;
    let base_ty = low.ty_of(base);
    let base_w = base_ty.bit_width();

    // `a[i]` where `a` is an ARRAY VALUE selects an element, not a bit.
    //
    // An `[T; n]` that is not a memory is still an array: it is a packed
    // vector so that it can be a struct field, a pipe payload or an operand
    // (ty.rs:49), and packing is a representation rather than a change of
    // meaning. Subscripting one used to fall through to the bit selects
    // below, so `line.words[1]` -- a struct field of type `[u32; 4]` -- read
    // bit 1 of the flattened struct and typed as `u1`. That is a wrong answer
    // rather than a diagnostic, and it is the shape of wrong answer that
    // synthesizes and runs.
    if let Ty::Array(elem, n) = &base_ty {
        return lower_array_index(low, sub, base, elem, *n, env, sink);
    }

    // `x[hi..lo]` -- a constant range.
    if let PrecResExpr::Span(range) = &sub.index {
        let hi = const_eval(&range.left).ok()?;
        let lo = const_eval(&range.right).ok()?;
        let range_is_in_bounds = hi >= lo && hi < base_w as u128;
        if !range_is_in_bounds {
            sink.err_span(
                low.here(),
                format!("bit range [{}..{}] is out of bounds for `{}`", hi, lo, base_ty.display()),
            );
            return None;
        }
        let (hi, lo) = (hi as u32, lo as u32);
        return Some(low.emit(Ty::UInt(hi - lo + 1), Op::Slice { arg: base, hi, lo }));
    }

    // `x[k]` -- a single bit, constant index.
    if let Ok(k) = const_eval(&sub.index) {
        let bit_is_in_bounds = k < base_w as u128;
        if !bit_is_in_bounds {
            sink.err_span(
                low.here(),
                format!("bit {} is out of bounds for `{}`", k, base_ty.display()),
            );
            return None;
        }
        let k = k as u32;
        return Some(low.emit(Ty::BOOL, Op::Slice { arg: base, hi: k, lo: k }));
    }

    let idx = lower_expr(low, &sub.index, env, sink)?;

    // An index that folded to a constant is still a single bit. `const_eval`
    // above works on the syntax, so it misses a constant parameter and misses
    // the induction variable of an unrolled `for`; both arrive here as a
    // `Ref` and leave lowering as an `Op::Const`. Without this, `v[i]` inside
    // a `for` becomes `v[i_3 +: 1]` -- a part-select with a constant base,
    // which is correct and is also the construct this backend exists to avoid
    // handing GowinSynthesis.
    if let Op::Const(k) = low.values[idx.0 as usize].op {
        if k >= base_w as u128 {
            sink.err_span(
                low.here(),
                format!("bit {} is out of bounds for `{}`", k, base_ty.display()),
            );
            return None;
        }
        let k = k as u32;
        return Some(low.emit(Ty::BOOL, Op::Slice { arg: base, hi: k, lo: k }));
    }

    // `x[i]` with a computed index -- a one-bit `+:` part-select.
    Some(low.emit(Ty::BOOL, Op::DynSlice { arg: base, base: idx, width: 1 }))
}

/// `@assert(cond)` or `@assert(cond, "message")`.
///
/// The message is a plain string because it is going into `$error`, which
/// takes a format string; there is no interpolation, so nothing in it can
/// depend on a value and it costs nothing to build.
fn lower_assert(
    low: &mut Lowerer,
    args: &[PrecResExpr],
    is_fatal: bool,
    env: &Env,
    sink: &mut DiagSink,
) -> Option<()> {
    let name = if is_fatal { "@fatal" } else { "@assert" };
    let arity_is_right = args.len() == 1 || args.len() == 2;
    if !arity_is_right {
        sink.err_span(
            low.here(),
            format!("{} takes a condition and an optional message", name),
        );
        return None;
    }
    let cond = lower_expr(low, &args[0], env, sink)?;
    let cond_ty = low.ty_of(cond);
    if cond_ty != Ty::BOOL {
        sink.err_span(
            low.here(),
            format!("{} needs an `u1` condition, found `{}`", name, cond_ty.display()),
        );
        return None;
    }
    let message = match args.get(1) {
        None => format!("{} failed", name),
        Some(PrecResExpr::Literal(Literal::StrLiteral(s))) => {
            let mut text = String::new();
            for piece in &s.pieces {
                text.push_str(piece.as_str());
            }
            text
        }
        Some(_) => {
            sink.err_span(
                low.here(),
                format!("the second argument to {} must be a string literal", name),
            );
            return None;
        }
    };
    low.add_assert(cond, message, is_fatal);
    Some(())
}

/// `m[addr]` -- an asynchronous read.
fn lower_mem_read(
    low: &mut Lowerer,
    mem_name: AlphanumSpan,
    index: &PrecResExpr,
    env: &Env,
    sink: &mut DiagSink,
) -> Option<ValueId> {
    let name = anumspan_to_str(&mem_name).to_string();
    let ix = match low.mem_index(&name) {
        Some(ix) => ix,
        None => {
            // The binding says memory but no memory was declared, which can
            // only happen if a memory-typed parameter slipped through.
            sink.err_at(&mem_name, format!("`{}` is not a memory of this process", name));
            return None;
        }
    };
    if low.mems[ix].kind != MemKind::LutRam {
        sink.push(
            Diag::error(
                low.span(&mem_name),
                format!("a read of `{}` takes a cycle, so it cannot sit inside an expression", name),
            )
            // The cycle has to be somewhere the source can point at, and the
            // two kinds of declaration spell that place differently: a state
            // in a process, a stage cut in a sequence.
            .with_note(if low.in_pipeline {
                format!(
                    "bind it on its own line -- `let x = {}[i]` -- and put a `|||` after it, which is the cycle",
                    name
                )
            } else {
                format!(
                    "bind it on its own line -- `let x = {}[i]` -- which makes the cycle a state",
                    name
                )
            }),
        );
        return None;
    }
    let (elem, addr_width) = (low.mems[ix].elem.clone(), low.mems[ix].addr_width);
    let raw = lower_expr(low, index, env, sink)?;
    let addr = low.fit_address(raw, addr_width, &mem_name, sink)?;
    let array = low.emit(elem.clone(), Op::MemRead { mem: ix as u32, addr });

    // A write earlier in this state -- or this stage -- has not reached the
    // array yet. The array updates on the clock edge, so `m[a] = v` followed
    // by `m[a]` read the OLD contents, which is not what the two lines say
    // happened. Forwarding the pending write is what makes the source mean
    // what it reads as.
    //
    // Synchronous reads use the same pending-write decision in ir_fsm and
    // ir_pipe, registering it across the read's clock edge.
    match low.pending_write(ix, addr, env) {
        None => Some(array),
        // An UNCONDITIONAL write to the address being read answers the read by
        // itself. Keeping the mux would leave the array read feeding a branch
        // that can never be taken, which is a wire and a read port asked of
        // the memory for nothing.
        Some((hit, wdata)) if matches!(low.values[hit.0 as usize].op, Op::Const(1)) => Some(wdata),
        Some((hit, wdata)) => Some(low.emit(
            elem,
            Op::Mux { cond: hit, then_val: wdata, else_val: array },
        )),
    }
}

/// `m[addr] = value` -- an offer to the one write port.
///
/// The write is recorded as three ordinary bindings, so an assignment under an
/// `if` becomes a write enable through the same SSA join that muxes everything
/// else, and two writes on different branches share the port instead of asking
/// for a second one.
fn lower_mem_write(
    low: &mut Lowerer,
    mem_name: AlphanumSpan,
    index: &PrecResExpr,
    rvalue: &PrecResExpr,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    let name = anumspan_to_str(&mem_name).to_string();
    let ix = match low.mem_index(&name) {
        Some(ix) => ix,
        None => {
            sink.err_at(&mem_name, format!("`{}` is not a memory of this process", name));
            return None;
        }
    };
    let (elem, addr_width) = (low.mems[ix].elem.clone(), low.mems[ix].addr_width);

    let raw = lower_expr(low, index, env, sink)?;
    let addr = low.fit_address(raw, addr_width, &mem_name, sink)?;

    let mut value = lower_expr_expecting(low, rvalue, Some(&elem), env, sink)?;
    let have = low.ty_of(value);
    if have != elem {
        match low.coerce_const(value, &elem) {
            Some(v) => value = v,
            None => {
                sink.push(
                    Diag::error(
                        low.span_of(&mem_name),
                        format!(
                            "`{}` holds `{}` but this write offers `{}`",
                            name,
                            elem.display(),
                            have.display()
                        ),
                    )
                    .with_note(cast_hint(&have, &elem)),
                );
                return None;
            }
        }
    }

    // A NEW slot, not an overwrite of the last one. Two writes in a row can
    // both happen this cycle, so they are two ports; before this the second
    // replaced the first in the environment and the first was silently lost.
    // Writes that cannot both happen -- the arms of an `if` -- still share a
    // slot, because both arms start from the same count and the join brings
    // them back together.
    let one = low.emit(Ty::BOOL, Op::Const(1));
    let slot = low.mem_slots(ix);
    low.set_mem_slots(ix, slot + 1);
    let (we_key, addr_key, data_key) = Lowerer::mem_port_keys(&name, slot);
    env.insert(we_key, Binding::constant(one, Ty::BOOL));
    env.insert(addr_key, Binding::constant(addr, Ty::UInt(addr_width)));
    env.insert(data_key, Binding::constant(value, elem));
    Some(())
}

fn lower_builtin(
    low: &mut Lowerer,
    op: BuiltinOp,
    args: &[PrecResExpr],
    env: &Env,
    sink: &mut DiagSink,
) -> Option<ValueId> {
    use BuiltinOp::*;

    // `@try_send(p, v)`. The value is offered; the answer is whether the slot
    // took it. Only the offer is recorded here -- the handshake itself is
    // generated after the body, so it cannot be got wrong per call site.
    // `@drop(p)` -- take what `p` offers and discard it. A `@try_rcv` with no
    // binding, so it spends the pipe's one operation for the cycle and answers
    // the same question: did anything transfer.
    if op == Drop {
        if args.len() != 1 {
            sink.err_span(low.here(), "`@drop` takes one pipe");
            return None;
        }
        let PrecResExpr::Ref(n) = &args[0] else {
            sink.err_span(low.here(), "`@drop` needs a pipe name");
            return None;
        };
        let pipe_name = anumspan_to_str(n).to_string();
        let Some(ix) = low.pipes.iter().position(|p| p.name == pipe_name) else {
            sink.err_span(
                low.here(),
                format!("`{}` is not a pipe of this process", pipe_name),
            );
            return None;
        };
        if !low.pipes[ix].is_input {
            sink.err_span(
                low.here(),
                format!("`{}` is an `out` pipe; there is nothing on it to drop", pipe_name),
            );
            return None;
        }
        if low.in_pipeline {
            sink.err_span(low.here(), "nonblocking buffer operations are not supported in a sequence; use its head `@rcv` and tail `@send`, or use a process");
            return None;
        }
        low.claim_transfer(&pipe_name, low.pipes[ix].used, true, sink)?;
        return Some(low.request_pipe(ix, None));
    }

    if op == TrySend {
        let arity_is_right = args.len() == 2;
        if !arity_is_right {
            sink.err_span(low.here(), "`@try_send` takes a pipe and a value");
            return None;
        }
        let pipe_name = match &args[0] {
            PrecResExpr::Ref(n) => anumspan_to_str(n).to_string(),
            _ => {
                sink.err_span(low.here(), "`@try_send` needs a pipe name");
                return None;
            }
        };

        let ix = match low.pipes.iter().position(|p| p.name == pipe_name) {
            Some(i) => i,
            None => {
                sink.err_span(
                    low.here(),
                    format!("`{}` is not a pipe of this process", pipe_name),
                );
                return None;
            }
        };
        if low.pipes[ix].is_input {
            sink.err_span(
                low.here(),
                format!("`{}` is an `in` pipe; it cannot be sent to", pipe_name),
            );
            return None;
        }
        if low.in_pipeline {
            sink.err_span(low.here(), "nonblocking buffer operations are not supported in a sequence; use its head `@rcv` and tail `@send`, or use a process");
            return None;
        }
        low.claim_transfer(&pipe_name, low.pipes[ix].used, false, sink)?;
        let want = low.pipes[ix].ty.clone();
        let mut value = lower_expr(low, &args[1], env, sink)?;
        let have = low.ty_of(value);
        if have != want {
            match low.coerce_const(value, &want) {
                Some(v) => value = v,
                None => {
                    sink.push(
                        Diag::error(
                            low.here(),
                            format!(
                                "`{}` carries `{}` but `{}` was sent",
                                pipe_name,
                                want.display(),
                                have.display()
                            ),
                        )
                        .with_note(cast_hint(&have, &want)),
                    );
                    return None;
                }
            }
        }
        // Whether the offer was taken is decided by the generated handshake.
        // A process with no states computes it once before the body; a state
        // machine computes one per state, because a pipe can be offered to in
        // one state and sampled in another.
        return Some(low.request_pipe(ix, Some(value)));
    }

    // `if c then a else b`, desugared by the parser. Both arms must agree on a
    // type -- there is no value an expression could take otherwise.
    if op == Select {
        let arity_is_right = args.len() == 3;
        if !arity_is_right {
            sink.err_span(low.here(), "`if ... then ... else` takes three operands");
            return None;
        }
        let cond = lower_expr(low, &args[0], env, sink)?;
        let cond_is_bool = low.ty_of(cond) == Ty::BOOL;
        if !cond_is_bool {
            sink.push(
                Diag::error(
                    low.here(),
                    format!(
                        "an `if` condition is `u1`, found `{}`",
                        low.ty_of(cond).display()
                    ),
                )
                .with_note("compare it against something, or slice a single bit out"),
            );
            return None;
        }
        // Expressions can send to ports or inline assertions. Their effects
        // belong to the selected arm, just as in a statement-form `if`.
        let depth = low.push_cond(cond, true);
        let then_result = lower_expr(low, &args[1], env, sink);
        low.pop_path(depth);
        let mut then_val = then_result?;
        let depth = low.push_cond(cond, false);
        let else_result = lower_expr(low, &args[2], env, sink);
        low.pop_path(depth);
        let mut else_val = else_result?;

        // An unsized literal on one side adopts the other side's type.
        let tt = low.ty_of(then_val);
        let et = low.ty_of(else_val);
        if tt != et {
            if let Some(v) = low.coerce_const(else_val, &tt) {
                else_val = v;
            } else if let Some(v) = low.coerce_const(then_val, &et) {
                then_val = v;
            } else {
                sink.push(
                    Diag::error(
                        low.here(),
                        format!(
                            "the two arms of this `if` are `{}` and `{}`",
                            tt.display(),
                            et.display()
                        ),
                    )
                    .with_note(cast_hint(&et, &tt)),
                );
                return None;
            }
        }
        let ty = low.ty_of(then_val);
        return Some(low.emit(ty, Op::Mux { cond, then_val, else_val }));
    }

    // Casts take a value and a constant width, so the width argument is folded
    // rather than lowered.
    match op {
        Zext | Sext | Trunc => {
            let (arg, want_w) = cast_args(low, args, env, sink)?;
            let have = low.ty_of(arg);
            let have_w = have.bit_width();
            return match op {
                Zext => {
                    let would_narrow = want_w < have_w;
                    if would_narrow {
                        sink.err_span(low.here(), "`@zext` cannot narrow; use `@trunc`");
                        return None;
                    }
                    Some(low.emit(Ty::UInt(want_w), Op::ZExt { arg, to: want_w }))
                }
                Sext => {
                    let would_narrow = want_w < have_w;
                    if would_narrow {
                        sink.err_span(low.here(), "`@sext` cannot narrow; use `@trunc`");
                        return None;
                    }
                    Some(low.emit(Ty::SInt(want_w), Op::SExt { arg, to: want_w }))
                }
                _ => {
                    let would_widen = want_w > have_w;
                    if would_widen {
                        sink.err_span(
                            low.here(),
                            "`@trunc` cannot widen; use `@zext` or `@sext`",
                        );
                        return None;
                    }
                    Some(low.emit(have.with_width(want_w), Op::Trunc { arg, to: want_w }))
                }
            };
        }
        Signed | Unsigned => {
            if args.len() != 1 {
                sink.err_span(low.here(), "this cast takes exactly one argument");
                return None;
            }
            let arg = lower_expr(low, &args[0], env, sink)?;
            let w = low.ty_of(arg).bit_width();
            let ty = if op == Signed { Ty::SInt(w) } else { Ty::UInt(w) };
            return Some(low.emit(ty, Op::Cast { arg }));
        }
        Slice => {
            // `@slice(x, base, width)`. The width is constant because the type
            // of the result is: a value whose width the cycle decides has no
            // type this compiler can check anything against.
            if args.len() != 3 {
                sink.err_span(
                    low.here(),
                    "`@slice` takes three arguments: a value, a base and a constant width",
                );
                return None;
            }
            let arg = lower_expr(low, &args[0], env, sink)?;
            let arg_ty = low.ty_of(arg);
            let total = arg_ty.bit_width();
            let width = match const_eval(&args[2]) {
                Ok(k) => k,
                Err(e) => {
                    sink.err_span(
                        low.here(),
                        format!("the width of a `@slice` {}", e.reason()),
                    );
                    return None;
                }
            };
            let width_fits = width > 0 && width <= total as u128;
            if !width_fits {
                sink.err_span(
                    low.here(),
                    format!(
                        "a `@slice` of {} bits does not fit in `{}`",
                        width,
                        arg_ty.display()
                    ),
                );
                return None;
            }
            let width = width as u32;
            let base = lower_expr(low, &args[1], env, sink)?;
            let base_ty = low.ty_of(base);
            if base_ty.is_signed() {
                sink.push(
                    Diag::error(
                        low.here(),
                        format!("a `@slice` base must be unsigned, found `{}`", base_ty.display()),
                    )
                    .with_note("convert with `@unsigned(x)`"),
                );
                return None;
            }
            // A constant base is an ordinary bit range, and an ordinary bit
            // range is what the reader wanted to write. `+:` with a literal
            // base is correct and is also the construct this backend exists to
            // keep away from GowinSynthesis.
            if let Op::Const(k) = low.values[base.0 as usize].op {
                let range_fits = k + width as u128 <= total as u128;
                if !range_fits {
                    sink.err_span(
                        low.here(),
                        format!(
                            "`@slice({}, {})` runs past the end of `{}`",
                            k,
                            width,
                            arg_ty.display()
                        ),
                    );
                    return None;
                }
                let lo = k as u32;
                return Some(low.emit(
                    Ty::UInt(width),
                    Op::Slice { arg, hi: lo + width - 1, lo },
                ));
            }
            return Some(low.emit(Ty::UInt(width), Op::DynSlice { arg, base, width }));
        }
        Rep => {
            let (arg, times) = cast_args(low, args, env, sink)?;
            if times == 0 {
                sink.err_span(low.here(), "`@rep` count must be at least 1");
                return None;
            }
            let w = low.ty_of(arg).bit_width() * times;
            return Some(low.emit(Ty::UInt(w), Op::Repeat { arg, times }));
        }
        _ => {}
    }

    // Unary.
    if matches!(op, BitInvert | Neg | LogNot) {
        if args.len() != 1 {
            sink.err_span(low.here(), "this operator takes one operand");
            return None;
        }
        let arg = lower_expr(low, &args[0], env, sink)?;
        let ty = low.ty_of(arg);
        let (un, out_ty) = match op {
            BitInvert => (UnOp::BitNot, ty.clone()),
            Neg => (UnOp::Neg, ty.clone()),
            _ => {
                if ty != Ty::BOOL {
                    sink.err_span(
                        low.here(),
                        format!("`!` needs `u1`, found `{}`", ty.display()),
                    );
                    return None;
                }
                (UnOp::LogNot, Ty::BOOL)
            }
        };
        return Some(low.emit(out_ty, Op::Un { op: un, arg }));
    }

    if args.len() != 2 {
        sink.err_span(low.here(), "this operator takes two operands");
        return None;
    }
    let mut lhs = lower_expr(low, &args[0], env, sink)?;
    let mut rhs = lower_expr(low, &args[1], env, sink)?;

    // Comparisons widen internally and never report a mismatch.
    if let Some(cmp) = cmp_of(op) {
        // Except against a tagged union, where a comparison would take in the
        // payload as well as the tag. `r == Nop` reads as "is it a Nop" and
        // is not: it is "is it a Nop AND are the payload bits all zero", which
        // is true for a value this compiler built and says nothing about one
        // that arrived through a port. `match` reads the tag and only the tag.
        for side in [lhs, rhs] {
            if let Ty::Enum { name, .. } = low.ty_of(side) {
                let is_union = low.syms.enums.get(&name).is_some_and(|d| d.is_tagged_union());
                if is_union {
                    sink.push(
                        Diag::error(
                            low.here(),
                            format!("`{}` carries payloads, so comparing it compares those too", name),
                        )
                        .with_note(
                            "use `match`, which reads the tag; a comparison would also have to agree on bits that mean nothing for the variant it is not",
                        ),
                    );
                    return None;
                }
            }
        }
        let (l, r) = low.extend_for_compare(lhs, rhs);
        return Some(low.emit(Ty::BOOL, Op::Cmp { op: cmp, lhs: l, rhs: r }));
    }

    // A shift's operands are independent, so only the value side matters.
    let operands_are_independent = matches!(op, Shl | Shr);
    if !operands_are_independent {
        // Give an unsized literal the width of its partner before checking.
        let lt = low.ty_of(lhs);
        let rt = low.ty_of(rhs);
        let types_differ = lt != rt;
        if types_differ {
            if let Some(v) = low.coerce_const(rhs, &lt) {
                rhs = v;
            } else if let Some(v) = low.coerce_const(lhs, &rt) {
                lhs = v;
            }
        }
    }

    let lt = low.ty_of(lhs);
    let rt = low.ty_of(rhs);
    let out_ty = match binop_result(op, &lt, &rt) {
        Ok(t) => t,
        Err(e) => {
            let mut d = Diag::error(low.here(), e.message());
            if let Some(note) = e.note() {
                d = d.with_note(note);
            }
            sink.push(d);
            return None;
        }
    };

    // `*` produces a full-width product, so both operands must first be
    // extended to that width -- Verilog would otherwise truncate to the
    // operand width and silently lose the top half.
    let widens_to_full_product = op == Mul;
    if widens_to_full_product {
        lhs = low.extend_to(lhs, &lt.with_width(out_ty.bit_width()));
        rhs = low.extend_to(rhs, &rt.with_width(out_ty.bit_width()));
    }

    let bin = match op {
        Add => BinOp::Add,
        Sub => BinOp::Sub,
        Mul => BinOp::Mul,
        Div => BinOp::Div,
        Mod => BinOp::Mod,
        Shl => BinOp::Shl,
        Shr => BinOp::Shr,
        And => BinOp::And,
        Or => BinOp::Or,
        Xor => BinOp::Xor,
        LogAnd => BinOp::And,
        LogOr => BinOp::Or,
        other => {
            sink.err_span(
                low.here(),
                format!("`{:?}` is not available in combinational logic", other),
            );
            return None;
        }
    };
    Some(low.emit(out_ty, Op::Bin { op: bin, lhs, rhs }))
}

/// `(value, constant)` argument pair shared by `@zext`/`@sext`/`@trunc`/`@rep`.
/// Drives a `port out` with `value`, on whatever path the caller is on.
///

fn cast_args(
    low: &mut Lowerer,
    args: &[PrecResExpr],
    env: &Env,
    sink: &mut DiagSink,
) -> Option<(ValueId, u32)> {
    if args.len() != 2 {
        sink.err_span(
            low.here(),
            "expected two arguments: a value and a constant width",
        );
        return None;
    }
    let value = lower_expr(low, &args[0], env, sink)?;
    let k = match const_eval(&args[1]) {
        Ok(k) => k,
        Err(e) => {
            sink.err_span(low.here(), format!("the second argument {}", e.reason()));
            return None;
        }
    };
    let width_is_sane = k > 0 && k <= 65536;
    if !width_is_sane {
        sink.err_span(low.here(), "width out of range");
        return None;
    }
    Some((value, k as u32))
}

fn cmp_of(op: BuiltinOp) -> Option<CmpOp> {
    Some(match op {
        BuiltinOp::Eq => CmpOp::Eq,
        BuiltinOp::Ne => CmpOp::Ne,
        BuiltinOp::Lt => CmpOp::Lt,
        BuiltinOp::Gt => CmpOp::Gt,
        BuiltinOp::Le => CmpOp::Le,
        BuiltinOp::Ge => CmpOp::Ge,
        _ => return None,
    })
}

/// Silences the unused-import warning for `OpTyError`, which is referenced
/// only through `binop_result`'s error type.
#[allow(dead_code)]
fn _assert_op_ty_error_is_used(e: &OpTyError) -> String {
    e.message()
}

/// `Read(addr)` -- an enum value with its payload.
///
/// The layout is `{tag, payload}`, the tag in the high bits, matching the way
/// a struct puts its first field there. A variant whose payload is narrower
/// than the widest one is padded below it, so every variant is the same width
/// and the tag always sits in the same place -- which is what lets a `match`
/// read the tag without knowing which variant it is looking at.
fn build_enum_value(
    low: &mut Lowerer,
    name: &AlphanumSpan,
    args: &[PrecResExpr],
    env: &Env,
    sink: &mut DiagSink,
) -> Option<ValueId> {
    let variant = anumspan_to_str(name).to_string();
    let (def, discriminant) = match low.syms.lookup_variant(&variant) {
        Some(hit) => hit,
        None => {
            sink.err_at(name, format!("`{}` is not an enum variant", variant));
            return None;
        }
    };
    let (enum_ty, enum_name) = (def.ty(), def.name.clone());
    let (tag_width, payload_width) = (def.tag_width, def.payload_width);
    let payload_ty = def.payload_of(&variant).cloned();

    let want = match payload_ty {
        Some(t) => t,
        None => {
            sink.push(
                Diag::error(
                    low.span_of(name),
                    format!("`{}` carries no payload, so it takes no arguments", variant),
                )
                .with_note(format!("write `{}` on its own", variant)),
            );
            return None;
        }
    };
    if args.len() != 1 {
        sink.push(
            Diag::error(
                low.span_of(name),
                format!(
                    "`{}` carries one `{}`, but {} arguments were given",
                    variant,
                    want.display(),
                    args.len()
                ),
            )
            .with_note("a variant carries at most one value; group several in a struct"),
        );
        return None;
    }

    let mut value = lower_expr_expecting(low, &args[0], Some(&want), env, sink)?;
    let have = low.ty_of(value);
    if have != want {
        match low.coerce_const(value, &want) {
            Some(v) => value = v,
            None => {
                sink.push(
                    Diag::error(
                        low.span_of(name),
                        format!(
                            "`{}` carries a `{}` but a `{}` was given",
                            variant,
                            want.display(),
                            have.display()
                        ),
                    )
                    .with_note(cast_hint(&have, &want)),
                );
                return None;
            }
        }
    }

    // Tag, payload, and the padding a narrow payload leaves under it.
    let tag = low.emit(Ty::UInt(tag_width), Op::Const(discriminant));
    let mut parts = vec![tag];
    let bits = want.bit_width();
    if bits != payload_width {
        let payload_bits = low.emit(Ty::UInt(bits), Op::Cast { arg: value });
        parts.push(payload_bits);
        // Zero rather than left undefined: `x` propagates through a comparison
        // in simulation and reads as a bug somewhere else entirely.
        parts.push(low.emit(Ty::UInt(payload_width - bits), Op::Const(0)));
    } else {
        parts.push(low.emit(Ty::UInt(bits), Op::Cast { arg: value }));
    }
    let packed = low.emit(Ty::UInt(tag_width + payload_width), Op::Concat(parts));
    let _ = enum_name;
    Some(low.emit(enum_ty, Op::Cast { arg: packed }))
}

/// `MyStruct(field0, field1, ...)` -- fields in declaration order.
///
/// The first field lands in the HIGH bits, matching SystemVerilog packed
/// structs, so a DDL struct and its SV counterpart have the same layout and
/// can cross a module boundary as one vector.
fn build_struct_value(
    low: &mut Lowerer,
    name: &AlphanumSpan,
    args: &[PrecResExpr],
    env: &Env,
    sink: &mut DiagSink,
) -> Option<ValueId> {
    let struct_name = anumspan_to_str(name).to_string();
    let def = low.syms.structs.get(&struct_name)?.clone();

    let arity_matches = args.len() == def.fields.len();
    if !arity_matches {
        sink.err_at(
            name,
            format!(
                "`{}` has {} field(s), found {}",
                struct_name,
                def.fields.len(),
                args.len()
            ),
        );
        return None;
    }

    let mut parts = Vec::new();
    for (arg, (field_name, field_ty)) in args.iter().zip(def.fields.iter()) {
        let mut value = lower_expr(low, arg, env, sink)?;
        let have = low.ty_of(value);
        if have != *field_ty {
            match low.coerce_const(value, field_ty) {
                Some(v) => value = v,
                None => {
                    sink.push(
                        Diag::error(
                            low.span_of(name),
                            format!(
                                "field `{}` of `{}` is `{}` but `{}` was given",
                                field_name,
                                struct_name,
                                field_ty.display(),
                                have.display()
                            ),
                        )
                        .with_note(cast_hint(&have, field_ty)),
                    );
                    return None;
                }
            }
        }
        parts.push(value);
    }
    Some(low.emit(def.ty(), Op::Concat(parts)))
}
