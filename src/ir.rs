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
    /// `!x`, i1 in and out.
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

/// An array with a backing store: one write port, asynchronous reads.
///
/// One write port is not a simplification, it is the constraint. K2G paid for
/// learning it: a second write port on the 32x32 value array inferred no RAM
/// at all and the array collapsed to 1120 flip-flops and ~3700 LUTs of read
/// muxing, against 32 SSRAM primitives and ~100 LUTs for the single-port
/// version (k2g_regfile.sv:18-24). So the write port is a fixed part of the
/// shape here, and several writes in the source mux onto it rather than
/// multiplying it.
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
    /// The write port, as ordinary values in the same graph.
    pub we: ValueId,
    pub addr: ValueId,
    pub data: ValueId,
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

#[derive(Debug)]
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
}

impl Module {
    pub fn is_clocked(&self) -> bool {
        !self.regs.is_empty() || !self.mems.is_empty()
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
            mem.we.0,
            mem.addr.0,
            mem.data.0
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
/// The compiler writes the handshake, not the user. That is the whole point:
/// channel rule 3 -- `valid` must not depend combinationally on `ready` -- then
/// holds BY CONSTRUCTION, because an output's `valid` is a register output and
/// nothing else can be wired to it. k2g_chan.sv records the bug that rule
/// exists to prevent: routing a stall back into `cp_valid` closed a loop
/// through stall -> decode -> CSP request -> stall.
#[derive(Debug, Clone)]
pub struct PipeInfo {
    pub name: String,
    pub ty: Ty,
    pub is_input: bool,
    /// `<name>_valid`, `<name>_ready`, `<name>_data` in that order.
    pub valid_port: PortId,
    /// `None` on a stream. A stream sink never refuses an item -- the oldest
    /// is overwritten instead -- so there is nothing for a `ready` to say, and
    /// emitting one that is tied high would invite someone to wire it up.
    pub ready_port: Option<PortId>,
    pub is_stream: bool,
    pub data_port: PortId,
    /// The value read from the data port; inputs only.
    pub data_value: Option<ValueId>,
    /// Set once the body has done a `@try_rcv` / `@try_send` on this pipe.
    pub used: bool,
    /// What `@try_send` offered; outputs only.
    pub sent: Option<ValueId>,
    /// The branch the `@try_send` sat on, if it was not at the top level.
    ///
    /// A decoder does not produce a micro-op every cycle. Without this an
    /// offer written inside an `if` would leak out of it -- `sent` lives on
    /// the lowerer rather than in the environment, so the SSA join that muxes
    /// everything else never sees it.
    pub send_guard: Option<ValueId>,
    /// Inputs: this pipe transferred this cycle. Outputs: the slot can take a
    /// new item. Both are computed before the body, from registers and the
    /// `ready` inputs only.
    pub fired: Option<ValueId>,
    /// Outputs only: the `busy` and `hold` registers backing the slot.
    /// First register of the output slot. A `buffer` takes four -- head
    /// valid/data and skid valid/data -- and a `stream` takes two.
    pub slot_reg: Option<usize>,
}

/// One narrowing of the path an assertion sits on.
#[derive(Debug, Clone)]
enum PathTerm {
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
    Pipe { is_input: bool, is_stream: bool },
    Constant,
}

/// Classifies one parameter, or reports why it cannot be one.
pub fn classify_param(
    low: &Lowerer,
    arg: &crate::parse::PrecArgTupleEntry,
    sink: &mut DiagSink,
) -> Option<ParamKind> {
    match arg.qualifier {
        ArgTypeQualifier::BufferIn => Some(ParamKind::Pipe { is_input: true, is_stream: false }),
        ArgTypeQualifier::BufferOut => Some(ParamKind::Pipe { is_input: false, is_stream: false }),
        ArgTypeQualifier::StreamIn => Some(ParamKind::Pipe { is_input: true, is_stream: true }),
        ArgTypeQualifier::StreamOut => Some(ParamKind::Pipe { is_input: false, is_stream: true }),
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
                .with_note("results leave through a `buffer out` or `stream out` pipe"),
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
    /// Pipe parameters in declaration order.
    pub pipes: Vec<PipeInfo>,
    /// Memories in declaration order. Reads name one by index.
    pub mems: Vec<Memory>,
    /// Immediate assertions, in source order.
    pub asserts: Vec<Assertion>,
    /// Constant parameters, in declaration order.
    pub params: Vec<(String, Ty, u128)>,
    /// `!done` for a process that stops, so its memories stop being written
    /// when it does. `None` for one that repeats.
    pub stop_writes: Option<ValueId>,
    /// The conditions under which the statements being lowered right now run,
    /// outermost first. Empty means unconditionally.
    ///
    /// Kept as a DESCRIPTION rather than as emitted values, because only
    /// assertions ever consult it. Materialising each branch guard eagerly
    /// would put a comparison and an `and` into the graph for every `if` and
    /// every match arm in the program -- dead, stripped by the backend, and
    /// still enough to renumber every generated wire in every module.
    path: Vec<PathTerm>,
    values: Vec<ValueDef>,
    ports: Vec<Port>,
}

impl<'a> Lowerer<'a> {
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
            pipes: Vec::new(),
            mems: Vec::new(),
            asserts: Vec::new(),
            params: Vec::new(),
            stop_writes: None,
            path: Vec::new(),
            values: Vec::new(),
            ports: Vec::new(),
        }
    }

    /// Declares the pipe parameters of a process or sequence.
    ///
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
                ParamKind::Pipe { is_stream: true, .. } => {
                    sink.err_at(&arg.arg_name, "`stream` pipes are not supported yet; `buffer` is");
                    return None;
                }
                ParamKind::Pipe { is_input, .. } => is_input,
            };
            let ty = match resolve_type_expr(&arg.type_expr, self.syms) {
                Ok(t) => t,
                Err(e) => {
                    sink.err_at(&arg.arg_name, e.message());
                    return None;
                }
            };
            let (vd, rd, dd) = if is_input {
                (PortDir::In, PortDir::Out, PortDir::In)
            } else {
                (PortDir::Out, PortDir::In, PortDir::Out)
            };
            let mut mk = |low: &mut Self, suffix: &str, dir: PortDir, t: Ty| {
                let id = PortId(low.ports.len() as u32);
                low.ports.push(Port { name: format!("{}_{}", name, suffix), dir, ty: t });
                id
            };
            let valid_port = mk(self, "valid", vd, Ty::BOOL);
            let ready_port = Some(mk(self, "ready", rd, Ty::BOOL));
            let data_port = mk(self, "data", dd, ty.clone());
            let data_value = if is_input {
                let v = self.emit(ty.clone(), Op::Port(data_port));
                self.values[v.0 as usize].name = Some(format!("{}_data", name));
                Some(v)
            } else {
                None
            };
            self.pipes.push(PipeInfo {
                name,
                ty,
                is_input,
                valid_port,
                ready_port,
                is_stream: false,
                data_port,
                data_value,
                used: false,
                sent: None,
                send_guard: None,
                fired: None,
                slot_reg: None,
            });
        }
        Some(())
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
                        "a plain parameter is a compile-time constant and needs `= <value>`; data arrives through a `buffer in` or `stream in` pipe",
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

    /// Env keys holding a memory's write port while the body runs.
    ///
    /// `#` cannot appear in an identifier, so these cannot collide with a name
    /// the source chose. Keeping them in the ordinary environment is what
    /// makes a write inside an `if` work with no extra machinery: the SSA join
    /// muxes them exactly as it muxes any other binding, so
    /// `if we_value: values[w_addr] = w_value` becomes a write enable.
    pub fn mem_port_keys(name: &str) -> (String, String, String) {
        (
            format!("{}#we", name),
            format!("{}#addr", name),
            format!("{}#data", name),
        )
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
        let we = self.emit(Ty::BOOL, Op::Const(0));
        let addr = self.emit(Ty::UInt(addr_width), Op::Const(0));
        let data = self.emit(elem.clone(), Op::Const(0));

        let (we_key, addr_key, data_key) = Self::mem_port_keys(&name);
        env.insert(we_key, Binding::constant(we, Ty::BOOL));
        env.insert(addr_key, Binding::constant(addr, Ty::UInt(addr_width)));
        env.insert(data_key, Binding::constant(data, elem.clone()));

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
            we,
            addr,
            data,
        });
        ix
    }

    /// Reads back the write port each memory was left with at the end of the
    /// body, so what the source did decides what the write port carries.
    pub fn settle_memories(&mut self, env: &Env) {
        for ix in 0..self.mems.len() {
            let (we_key, addr_key, data_key) = Self::mem_port_keys(&self.mems[ix].name);
            if let Some(v) = env.get(&we_key).and_then(|b| b.value) {
                self.mems[ix].we = v;
            }
            if let Some(v) = env.get(&addr_key).and_then(|b| b.value) {
                self.mems[ix].addr = v;
            }
            if let Some(v) = env.get(&data_key).and_then(|b| b.value) {
                self.mems[ix].data = v;
            }
            let we = self.mems[ix].we;
            let we = match self.stop_writes {
                None => we,
                Some(r) => self.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: we, rhs: r }),
            };
            self.mems[ix].we = we;
            self.mems[ix].addr = self.drop_gated_mux(self.mems[ix].addr, we);
            self.mems[ix].data = self.drop_gated_mux(self.mems[ix].data, we);
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
    fn drop_gated_mux(&self, mut value: ValueId, gate: ValueId) -> ValueId {
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
        // single stream output that identity is all there is.
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
        self.map.span_of(at)
    }

    pub fn span_of(&self, at: &AlphanumSpan) -> Span {
        self.map.span_of(at)
    }

    /// Variant list of a named enum, or a diagnostic-free `None` if the table
    /// does not have it (which the caller has already reported).
    pub fn enum_variants(&self, name: &str) -> Option<Vec<(String, u128)>> {
        self.syms.enums.get(name).map(|d| d.variants.clone())
    }

    /// An equality test between two already-matching operands.
    pub fn emit_eq(&mut self, lhs: ValueId, rhs: ValueId) -> ValueId {
        self.emit(Ty::BOOL, Op::Cmp { op: CmpOp::Eq, lhs, rhs })
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
        let dir = match arg.qualifier {
            ArgTypeQualifier::In => PortDir::In,
            ArgTypeQualifier::Out => PortDir::Out,
            ArgTypeQualifier::Inout => {
                sink.err_at(
                    &arg.arg_name,
                    "`inout` parameters are not supported in a combinational function",
                );
                return None;
            }
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
                .with_note("write `result: out i32` in the parameter list"),
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
        params: low.params,
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
pub fn lower_process(
    map: &SourceMap,
    syms: &Symbols,
    bodies: &HashMap<String, &FunctionDecl>,
    decl: &ProcessDecl,
    sink: &mut DiagSink,
) -> Option<Module> {
    let mut low = Lowerer::new(map, syms, bodies);
    let mut env: Env = Env::new();

    // Clock and reset are implicit. "A process has channel ports and
    // clock/reset. Nothing else." -- k3g_chan.sv:31.
    for implicit in ["clk", "rst_n"] {
        let port_id = PortId(low.ports.len() as u32);
        low.ports.push(Port {
            name: implicit.to_string(),
            dir: PortDir::In,
            ty: Ty::BOOL,
        });
        let v = low.emit(Ty::BOOL, Op::Port(port_id));
        low.values[v.0 as usize].name = Some(implicit.to_string());
        env.insert(implicit.to_string(), Binding::constant(v, Ty::BOOL));
    }

    for arg in &decl.args.entries {
        let name = anumspan_to_str(&arg.arg_name).to_string();
        if name == "clk" || name == "rst_n" {
            sink.err_at(&arg.arg_name, format!("`{}` is implicit on a process", name));
            return None;
        }
        let kind = match classify_param(&low, arg, sink) {
            Some(k) => k,
            None => return None,
        };
        // A plain parameter is configuration, folded here and gone. Everything
        // that changes cycle to cycle is a pipe.
        let (is_input, is_stream) = match kind {
            ParamKind::Constant => {
                low.declare_constant(arg, &mut env, sink)?;
                continue;
            }
            ParamKind::Pipe { is_input, is_stream } => (is_input, is_stream),
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
        // "every process port list flattens to valid/ready/data triples and
        // the rules stay exactly as written".
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
        let valid_port = mk(&mut low, "valid", vdir, Ty::BOOL);
        // A stream has no `ready`: its producer never waits, and its consumer
        // takes whatever is being offered on the cycle it looks.
        let ready_port = if is_stream {
            None
        } else {
            Some(mk(&mut low, "ready", rdir, Ty::BOOL))
        };
        let data_port = mk(&mut low, "data", ddir, ty.clone());

        let data_value = if is_input {
            let v = low.emit(ty.clone(), Op::Port(data_port));
            low.values[v.0 as usize].name = Some(format!("{}_data", name));
            Some(v)
        } else {
            None
        };
        low.pipes.push(PipeInfo {
            name: name.clone(),
            ty,
            is_input,
            valid_port,
            ready_port,
            is_stream,
            data_port,
            data_value,
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
        let name = anumspan_to_str(&var_decl.name).to_string();
        let ty = match &var_decl.ty_expr {
            Some(t) => match resolve_type_expr(t, syms) {
                Ok(t) => t,
                Err(e) => {
                    sink.err_at(&var_decl.name, e.message());
                    return None;
                }
            },
            None => {
                sink.err_at(&var_decl.name, "a register needs a declared type");
                return None;
            }
        };
        // A memory is storage rather than a value, so it takes neither a
        // register slot nor a reset value of its own -- the reset, if there is
        // one, applies to every element.
        if let Ty::Mem { elem, len, kind } = ty.clone() {
            // A block RAM reads SYNCHRONOUSLY: the value arrives a cycle after
            // the address. Accepting the annotation and then emitting an
            // asynchronous read would quietly give the caller distributed RAM
            // under a `bram` label, and accepting it with a real block RAM
            // needs a scheduling model that does not exist yet.
            let read_is_synchronous = kind != MemKind::LutRam;
            if read_is_synchronous {
                sink.push(
                    Diag::error(
                        map.span_of(&var_decl.name),
                        format!(
                            "`#[impl({})]` is not supported yet; only `lutram` is",
                            kind.display()
                        ),
                    )
                    .with_note(
                        "its read takes a cycle, and there is no way yet to say where that cycle goes",
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
                                    map.span_of(&var_decl.name),
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
            low.declare_memory(name, *elem, len, kind, reset, &mut env);
            body_start += 1;
            continue;
        }

        let init = match &var_decl.assign_val {
            Some(e) => e,
            None => {
                sink.err_at(&var_decl.name, "a register needs a reset value");
                return None;
            }
        };
        let reset_value = lower_expr_expecting(&mut low, init, Some(&ty), &env, sink)?;
        let reset = match &low.values[reset_value.0 as usize].op {
            Op::Const(k) => *k,
            _ => {
                sink.err_at(
                    &var_decl.name,
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

    let has_pipes = !low.pipes.is_empty();
    if !has_pipes {
        sink.push(
            Diag::error(
                map.span_of(&decl.name),
                "a process needs at least one pipe",
            )
            .with_note("a process with no channels computes nothing anything else can see"),
        );
        return None;
    }

    // What the body MEANS, and it is the `loop` that decides.
    //
    // A process body is a program: it runs once and then the process stops
    // (desc.md:39, "may stop (reach terminal state)"). `loop` is what makes it
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
        if !low.mems.is_empty() {
            sink.push(
                Diag::error(
                    map.span_of(&decl.name),
                    "a memory in a process that blocks is not supported yet",
                )
                .with_note(
                    "the accesses would have to be scheduled into the states; keep the memory in a process without `@rcv`/`@send`",
                ),
            );
            return None;
        }
        return crate::ir_fsm::lower_blocking(
            map, decl, low, env, Vec::new(), reg_names, reg_tys, reg_resets, &loop_body,
            repeats, sink,
        );
    }

    // ---- the generated handshake ------------------------------------------
    //
    // One registered entry per output pipe, which is the shape k3g_expand.sv
    // hand-writes: `uops.valid = busy; iops.ready = !busy || uops.ready`.
    // `valid` is a register output and nothing else can drive it, so channel
    // rule 3 -- `valid` must not depend combinationally on `ready` -- holds by
    // construction rather than by review.
    let mut generated: Vec<Reg> = Vec::new();
    let mut accepts: Vec<ValueId> = Vec::new();

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

    for ix in 0..low.pipes.len() {
        if low.pipes[ix].is_input {
            continue;
        }
        let ty = low.pipes[ix].ty.clone();
        let pname = low.pipes[ix].name.clone();

        let base = reg_names.len() + generated.len();
        let busy = low.emit(Ty::BOOL, Op::RegRead(base as u32));
        low.values[busy.0 as usize].name = Some(format!("{}_busy", pname));
        let hold = low.emit(ty.clone(), Op::RegRead((base + 1) as u32));
        low.values[hold.0 as usize].name = Some(format!("{}_hold", pname));
        generated.push(Reg { name: format!("{}_busy", pname), ty: Ty::BOOL, reset: 0, next: busy });
        generated
            .push(Reg { name: format!("{}_hold", pname), ty: ty.clone(), reset: 0, next: hold });

        // A BUFFER is two deep, and the second entry is what makes `ready` a
        // register.
        //
        // With one entry the only honest thing `ready` can say is "I am empty,
        // or I am draining this cycle" -- and "draining" means the consumer's
        // `ready`. So the producer's `ready` became a wire straight through to
        // the consumer's, and a chain of N processes was one combinational path
        // N modules long. Rule 3 was still satisfied (`valid` never looked at
        // `ready`), but the path was there.
        //
        // With a skid entry, `ready` is `the skid is empty` -- register-derived,
        // like `up.ready = !full` in k3g_chan.sv:193, whose comment insists on
        // exactly this: "never a function of the opposite side's handshake".
        // The producer now learns about a stall one cycle late and has a place
        // to put the item it already committed to, which is the whole job of
        // the second entry.
        //
        // A STREAM keeps one entry and never refuses: the oldest is
        // overwritten, which is the difference between the two kinds, and it is
        // why a process feeding only streams never stalls its input.
        let accept = match low.pipes[ix].ready_port {
            None => low.emit(Ty::BOOL, Op::Const(1)),
            Some(_) => {
                let skid_busy = low.emit(Ty::BOOL, Op::RegRead((base + 2) as u32));
                low.values[skid_busy.0 as usize].name = Some(format!("{}_skid_busy", pname));
                let skid = low.emit(ty.clone(), Op::RegRead((base + 3) as u32));
                low.values[skid.0 as usize].name = Some(format!("{}_skid", pname));
                generated.push(Reg {
                    name: format!("{}_skid_busy", pname),
                    ty: Ty::BOOL,
                    reset: 0,
                    next: skid_busy,
                });
                generated.push(Reg {
                    name: format!("{}_skid", pname),
                    ty,
                    reset: 0,
                    next: skid,
                });
                let a = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: skid_busy });
                low.values[a.0 as usize].name = Some(format!("{}_room", pname));
                accepts.push(a);
                a
            }
        };

        low.pipes[ix].fired = Some(accept);
        low.pipes[ix].slot_reg = Some(base);
    }

    // An input transfers when upstream offers and every output slot can take
    // the result. With one input and one output that is exactly
    // `iops.ready = !busy || uops.ready`.
    let mut can_accept: Option<ValueId> = None;
    for a in &accepts {
        can_accept = Some(match can_accept {
            None => *a,
            Some(prev) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: prev, rhs: *a }),
        });
    }
    let always_true = low.emit(Ty::BOOL, Op::Const(1));
    let ready_out = can_accept.unwrap_or(always_true);
    // Once the pass is over the process refuses everything, which is what
    // "reaches a terminal state" has to mean at a channel boundary.
    let ready_out = match running {
        None => ready_out,
        Some(r) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: ready_out, rhs: r }),
    };

    for ix in 0..low.pipes.len() {
        if !low.pipes[ix].is_input {
            continue;
        }
        let up_valid = low.emit(Ty::BOOL, Op::Port(low.pipes[ix].valid_port));
        // A buffer input transfers only when every buffer output can take the
        // result. A stream input has no such contract -- it is a sample of
        // whatever is being offered, and it happens whether or not this cycle
        // produces anything.
        let fired = if low.pipes[ix].is_stream {
            match running {
                None => up_valid,
                Some(r) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: up_valid, rhs: r }),
            }
        } else {
            low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: up_valid, rhs: ready_out })
        };
        low.values[fired.0 as usize].name = Some(format!("{}_xfer", low.pipes[ix].name));
        low.pipes[ix].fired = Some(fired);
    }

    lower_stmts(&mut low, &body_stmts, &mut env, sink)?;
    low.stop_writes = running;
    low.settle_memories(&env);

    let mut drivers = Vec::new();

    // What the body offered decides the slot's next state. The offer is taken
    // on the same cycle the input transferred, which is the contract this
    // milestone supports: one item in, one item out, one registered entry.
    let fired_any = {
        let inputs: Vec<ValueId> =
            low.pipes.iter().filter(|p| p.is_input).filter_map(|p| p.fired).collect();
        match inputs.first() {
            Some(f) => *f,
            None => low.emit(Ty::BOOL, Op::Const(0)),
        }
    };

    for ix in 0..low.pipes.len() {
        let pipe = low.pipes[ix].clone();
        if pipe.is_input {
            if let Some(ready) = pipe.ready_port {
                drivers.push((ready, ready_out));
            }
            continue;
        }
        let base = pipe.slot_reg.expect("an output pipe has a slot");
        let busy = low.emit(Ty::BOOL, Op::RegRead(base as u32));
        let hold = low.emit(pipe.ty.clone(), Op::RegRead((base + 1) as u32));

        let sent = match pipe.sent {
            Some(v) => v,
            None => {
                sink.err_span(
                    map.span_of(&decl.name),
                    format!("`{}` is never sent to", pipe.name),
                );
                return None;
            }
        };

        // An offer written inside an `if` only happens on that branch.
        let offering = match pipe.send_guard {
            None => fired_any,
            Some(g) => low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: fired_any, rhs: g }),
        };

        let idx = base - reg_names.len();
        match pipe.ready_port {
            // A stream holds nothing. Its `valid` is one cycle per item,
            // because there is no `ready` to tell it the item was read and
            // holding it would turn "the oldest is overwritten" into "the
            // newest is dropped". That strobe is `uop_valid` in k2g_decode.sv.
            None => {
                let hold_next = low.emit(
                    pipe.ty.clone(),
                    Op::Mux { cond: offering, then_val: sent, else_val: hold },
                );
                generated[idx].next = offering;
                generated[idx + 1].next = hold_next;
            }
            // A two-deep buffer: head, then skid.
            //
            // An offer can only arrive while the skid is empty, because that is
            // what `ready` said -- so "push into the skid while the skid is
            // moving into the head" cannot happen, and the four cases below are
            // all of them.
            Some(ready) => {
                let skid_busy = low.emit(Ty::BOOL, Op::RegRead((base + 2) as u32));
                let skid = low.emit(pipe.ty.clone(), Op::RegRead((base + 3) as u32));
                let ready_in = low.emit(Ty::BOOL, Op::Port(ready));

                let pop = low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: busy, rhs: ready_in });
                low.values[pop.0 as usize].name = Some(format!("{}_pop", pipe.name));
                let not_pop = low.logical_not(pop);

                // The head takes a new item when it is empty or emptying, and
                // takes the skid when the skid has something waiting.
                let not_busy = low.logical_not(busy);
                let head_free =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: not_busy, rhs: pop });
                let to_head =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: offering, rhs: head_free });
                let from_skid =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: pop, rhs: skid_busy });
                let keep_head =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: busy, rhs: not_pop });

                let held_or_filled =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: keep_head, rhs: from_skid });
                let busy_next = low.emit(
                    Ty::BOOL,
                    Op::Bin { op: BinOp::Or, lhs: held_or_filled, rhs: to_head },
                );
                // `from_skid` and `to_head` cannot both hold: an offer needs an
                // empty skid, and `from_skid` needs a full one.
                let taken_from_skid = low.emit(
                    pipe.ty.clone(),
                    Op::Mux { cond: to_head, then_val: sent, else_val: hold },
                );
                let hold_next = low.emit(
                    pipe.ty.clone(),
                    Op::Mux { cond: from_skid, then_val: skid, else_val: taken_from_skid },
                );

                // The skid takes the offer only when the head is occupied and
                // staying that way.
                let head_stays =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: busy, rhs: not_pop });
                let to_skid =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: offering, rhs: head_stays });
                let skid_keeps =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::And, lhs: skid_busy, rhs: not_pop });
                let skid_busy_next =
                    low.emit(Ty::BOOL, Op::Bin { op: BinOp::Or, lhs: skid_keeps, rhs: to_skid });
                let skid_next = low.emit(
                    pipe.ty.clone(),
                    Op::Mux { cond: to_skid, then_val: sent, else_val: skid },
                );

                generated[idx].next = busy_next;
                generated[idx + 1].next = hold_next;
                generated[idx + 2].next = skid_busy_next;
                generated[idx + 3].next = skid_next;
            }
        }

        // `valid` is the register, never anything combinational.
        drivers.push((pipe.valid_port, busy));
        drivers.push((pipe.data_port, hold));
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
        params: low.params,
        asserts: low.asserts,
        mems: low.mems,
        name: anumspan_to_str(&decl.name).to_string(),
        ports: low.ports,
        values: low.values,
        drivers,
        regs,
    })
}


/// `let (item, got) = @try_rcv(p)`.
///
/// The only tuple-producing form in the language, so this is deliberately
/// narrow rather than a general tuple type.
fn lower_try_rcv_binding(
    low: &mut Lowerer,
    decl: &crate::parse::VarDeclStmt,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    let takes_two = decl.rest.len() == 1;
    if !takes_two {
        sink.err_at(&decl.name, "a tuple binding takes exactly two names here");
        return None;
    }
    let init = match &decl.assign_val {
        Some(e) => e,
        None => {
            sink.err_at(&decl.name, "a tuple binding needs an initialiser");
            return None;
        }
    };
    let (pipe_expr, is_try_rcv) = match init {
        PrecResExpr::Call { base, args } => match &**base {
            PrecResExpr::Builtin(BuiltinOp::TryRecieve) if args.len() == 1 => {
                (&args[0], true)
            }
            _ => (init, false),
        },
        _ => (init, false),
    };
    if !is_try_rcv {
        sink.err_at(&decl.name, "only `@try_rcv(p)` produces a pair");
        return None;
    }
    let pipe_name = match pipe_expr {
        PrecResExpr::Ref(n) => anumspan_to_str(n).to_string(),
        _ => {
            sink.err_at(&decl.name, "`@try_rcv` needs a pipe name");
            return None;
        }
    };
    let ix = match low.pipes.iter().position(|p| p.name == pipe_name) {
        Some(i) => i,
        None => {
            sink.err_at(&decl.name, format!("`{}` is not a pipe of this process", pipe_name));
            return None;
        }
    };
    if !low.pipes[ix].is_input {
        sink.err_at(&decl.name, format!("`{}` is an `out` pipe; it cannot be received from", pipe_name));
        return None;
    }
    if low.pipes[ix].used {
        sink.err_at(&decl.name, format!("`{}` is received from more than once in one cycle", pipe_name));
        return None;
    }
    low.pipes[ix].used = true;

    let ty = low.pipes[ix].ty.clone();
    let data = low.pipes[ix].data_value.expect("an input pipe has a data value");
    let fired = low.pipes[ix].fired.expect("computed before the body");

    let item = anumspan_to_str(&decl.name).to_string();
    let got = anumspan_to_str(&decl.rest[0]).to_string();
    env.insert(item, Binding::constant(data, ty));
    env.insert(got, Binding::constant(fired, Ty::BOOL));
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

fn lower_stmt(
    low: &mut Lowerer,
    stmt: &PrecResInnerStmt,
    env: &mut Env,
    sink: &mut DiagSink,
) -> Option<()> {
    match stmt {
        PrecResInnerStmt::VarDecl(decl) => {
            let name = anumspan_to_str(&decl.name).to_string();

            // A tuple binding takes one name per result. There are two things
            // that produce several: `@try_rcv`, which answers with the item and
            // whether there was one, and a `fun` with several `out` parameters.
            if !decl.rest.is_empty() {
                let callee = match &decl.assign_val {
                    Some(PrecResExpr::Call { base, .. }) => match &**base {
                        PrecResExpr::Ref(n) => Some(*n),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(callee) = callee {
                    let args = match &decl.assign_val {
                        Some(PrecResExpr::Call { args, .. }) => args.clone(),
                        _ => unreachable!("matched a call above"),
                    };
                    let mut names = vec![decl.name];
                    names.extend(decl.rest.iter().copied());
                    return crate::ir_match::inline_call_multi(
                        low, &callee, &args, &names, env, sink,
                    );
                }
                return lower_try_rcv_binding(low, decl, env, sink);
            }
            let declared = match &decl.ty_expr {
                Some(t) => match resolve_type_expr(t, low.syms) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        sink.err_at(&decl.name, e.message());
                        return None;
                    }
                },
                None => None,
            };
            if declared.as_ref().is_some_and(|t| t.is_memory()) {
                sink.push(
                    Diag::error(
                        low.span(&decl.name),
                        format!("`{}` is a memory, which is state rather than a value", name),
                    )
                    .with_note(
                        "declare it as a `var` at the top of a `process`; a `sequence` must not contain memory (desc.md:44)",
                    ),
                );
                return None;
            }
            let init = match &decl.assign_val {
                Some(e) => e,
                None => {
                    sink.err_at(
                        &decl.name,
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
                                    low.span(&decl.name),
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
            if let PrecResExpr::SubscriptAccess(sub) = &assign.lvalue {
                if let PrecResExpr::Ref(mem_name) = &sub.base {
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
                            low, *mem_name, &sub.index, &assign.rvalue, env, sink,
                        );
                    }
                }
            }

            let path = match lvalue_path(&assign.lvalue) {
                Some(p) => p,
                None => {
                    sink.err_span(
                        crate::driver::nowhere(),
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
                        "a plain parameter is folded at compile time; per-cycle data arrives through a `buffer in` or `stream in` pipe",
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
            // Compound assignment (`x += y`) is not desugared yet; the parser
            // records the kind, so reject it explicitly rather than silently
            // treating it as a plain assignment.
            if assign.kind != crate::lex::AssignStmtKind::PlainAssign {
                sink.err_at(&target, "compound assignment is not supported yet");
                return None;
            }

            // Resolve the field chain to one absolute bit range.
            let mut want = base_ty.clone();
            let mut offset = 0u32;
            let mut span: Option<(u32, u32)> = None;
            for field in &path.fields {
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

            let mut value = lower_expr_expecting(low, &assign.rvalue, Some(&want), env, sink)?;
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
                    crate::driver::nowhere(),
                    format!(
                        "an `if` condition must be `i1`, found `{}`",
                        cond_ty.display()
                    ),
                );
                return None;
            }

            let mut then_env = env.clone();
            let depth = low.push_cond(cond, true);
            lower_branch(low, &ite.then_case, &mut then_env, sink)?;
            low.pop_path(depth);

            let mut else_env = env.clone();
            if let Some(else_case) = &ite.else_case {
                let depth = low.push_cond(cond, false);
                lower_branch(low, else_case, &mut else_env, sink)?;
                low.pop_path(depth);
            }

            // SSA join: any binding the two arms disagree about becomes a mux.
            let names: Vec<String> = env.keys().cloned().collect();
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
                                crate::driver::nowhere(),
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
                            crate::driver::nowhere(),
                            format!(
                                "`{}` is assigned on only one branch of this `if`",
                                name
                            ),
                        );
                        sink.push(Diag::error(crate::driver::nowhere(), "incomplete assignment")
                            .with_note(
                                "combinational logic has no memory, so every branch must assign it; add an `else`",
                            ));
                        return None;
                    }
                    (None, None) => None,
                };
                if let Some(v) = merged {
                    if let Some(b) = env.get_mut(&name) {
                        b.value = Some(v);
                    }
                }
            }
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
            let checking = match &call.base {
                PrecResExpr::Builtin(BuiltinOp::Assert) => Some(false),
                PrecResExpr::Builtin(BuiltinOp::Fatal) => Some(true),
                _ => None,
            };
            match checking {
                Some(is_fatal) => lower_assert(low, &call.args, is_fatal, env, sink),
                None => {
                    sink.err_span(
                        crate::driver::nowhere(),
                        "this statement has no effect in combinational logic",
                    );
                    None
                }
            }
        }

        PrecResInnerStmt::TailVal(_) => {
            sink.err_span(
                crate::driver::nowhere(),
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

        PrecResInnerStmt::Break => {
            sink.push(
                Diag::error(
                    crate::driver::nowhere(),
                    "`break` is not scheduled yet",
                )
                .with_note(
                    "it needs a control-flow graph: a `break` inside a conditional has to stop the statements after it on that path only, and the statements in a state are joined by muxes rather than ordered. A linear process body already runs once and stops.",
                ),
            );
            None
        }

        PrecResInnerStmt::Loop(_) | PrecResInnerStmt::ForLoop(_) => {
            sink.err_span(
                crate::driver::nowhere(),
                "a `loop` belongs at the top of a `process` body, not nested inside it",
            );
            None
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
                crate::driver::nowhere(),
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
struct LvaluePath {
    base: AlphanumSpan,
    /// As written, outermost first: `u.a.b` gives `[a, b]`.
    fields: Vec<AlphanumSpan>,
}

fn lvalue_path(expr: &PrecResExpr) -> Option<LvaluePath> {
    match expr {
        PrecResExpr::Ref(base) => Some(LvaluePath { base: *base, fields: Vec::new() }),
        PrecResExpr::FieldAccess { base, field_name } => {
            let mut path = lvalue_path(base)?;
            path.fields.push(*field_name);
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
    if let PrecResExpr::Call { base, args } = expr {
        if let PrecResExpr::Builtin(b) = &**base {
            match b {
                BuiltinOp::Zeroed if args.is_empty() => {
                    return match want {
                        Some(ty) => Some(low.emit(ty.clone(), Op::Const(0))),
                        None => {
                            sink.err_span(
                                crate::driver::nowhere(),
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
                                crate::driver::nowhere(),
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
                                crate::driver::nowhere(),
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
            if !is_local {
                if let Some((def, discriminant)) = low.syms.lookup_variant(name) {
                    let ty = def.ty();
                    return Some(low.emit(ty, Op::Const(discriminant)));
                }
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
            if let Some(w) = width {
                if !literal_fits(*value, &Ty::UInt(*w)) {
                    sink.err_span(
                        crate::driver::nowhere(),
                        format!("literal {} does not fit in {} bits", value, w),
                    );
                    return None;
                }
            }
            Some(low.emit(ty, Op::Const(*value)))
        }

        PrecResExpr::Literal(other) => {
            sink.err_span(
                crate::driver::nowhere(),
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
                        crate::driver::nowhere(),
                        "this is not something that can be called",
                    );
                    None
                }
            }
        }

        other => {
            sink.err_span(
                crate::driver::nowhere(),
                format!("cannot lower {:?} to hardware yet", other),
            );
            None
        }
    }
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
            return lower_mem_read(low, *mem_name, &sub.index, env, sink);
        }
    }

    let base = lower_expr(low, &sub.base, env, sink)?;
    let base_ty = low.ty_of(base);
    let base_w = base_ty.bit_width();

    // `x[hi..lo]` -- a constant range.
    if let PrecResExpr::Span(range) = &sub.index {
        let hi = const_eval(&range.left).ok()?;
        let lo = const_eval(&range.right).ok()?;
        let range_is_in_bounds = hi >= lo && hi < base_w as u128;
        if !range_is_in_bounds {
            sink.err_span(
                crate::driver::nowhere(),
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
                crate::driver::nowhere(),
                format!("bit {} is out of bounds for `{}`", k, base_ty.display()),
            );
            return None;
        }
        let k = k as u32;
        return Some(low.emit(Ty::BOOL, Op::Slice { arg: base, hi: k, lo: k }));
    }

    // `x[i]` with a computed index -- a one-bit `+:` part-select.
    let idx = lower_expr(low, &sub.index, env, sink)?;
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
            crate::driver::nowhere(),
            format!("{} takes a condition and an optional message", name),
        );
        return None;
    }
    let cond = lower_expr(low, &args[0], env, sink)?;
    let cond_ty = low.ty_of(cond);
    if cond_ty != Ty::BOOL {
        sink.err_span(
            crate::driver::nowhere(),
            format!("{} needs an `i1` condition, found `{}`", name, cond_ty.display()),
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
                crate::driver::nowhere(),
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
    let (elem, addr_width) = (low.mems[ix].elem.clone(), low.mems[ix].addr_width);
    let raw = lower_expr(low, index, env, sink)?;
    let addr = low.fit_address(raw, addr_width, &mem_name, sink)?;
    Some(low.emit(elem, Op::MemRead { mem: ix as u32, addr }))
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

    let one = low.emit(Ty::BOOL, Op::Const(1));
    let (we_key, addr_key, data_key) = Lowerer::mem_port_keys(&name);
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
    if op == TrySend {
        let arity_is_right = args.len() == 2;
        if !arity_is_right {
            sink.err_span(crate::driver::nowhere(), "`@try_send` takes a pipe and a value");
            return None;
        }
        let pipe_name = match &args[0] {
            PrecResExpr::Ref(n) => anumspan_to_str(n).to_string(),
            _ => {
                sink.err_span(crate::driver::nowhere(), "`@try_send` needs a pipe name");
                return None;
            }
        };
        let ix = match low.pipes.iter().position(|p| p.name == pipe_name) {
            Some(i) => i,
            None => {
                sink.err_span(
                    crate::driver::nowhere(),
                    format!("`{}` is not a pipe of this process", pipe_name),
                );
                return None;
            }
        };
        if low.pipes[ix].is_input {
            sink.err_span(
                crate::driver::nowhere(),
                format!("`{}` is an `in` pipe; it cannot be sent to", pipe_name),
            );
            return None;
        }
        if low.pipes[ix].used {
            sink.err_span(
                crate::driver::nowhere(),
                format!("`{}` is sent to more than once in one cycle", pipe_name),
            );
            return None;
        }
        let want = low.pipes[ix].ty.clone();
        let mut value = lower_expr(low, &args[1], env, sink)?;
        let have = low.ty_of(value);
        if have != want {
            match low.coerce_const(value, &want) {
                Some(v) => value = v,
                None => {
                    sink.push(
                        Diag::error(
                            crate::driver::nowhere(),
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
        low.pipes[ix].used = true;
        low.pipes[ix].sent = Some(value);
        low.pipes[ix].send_guard = low.materialise_path();
        // Whether the offer was taken is decided by the generated handshake;
        // a placeholder stands in until it is built.
        return Some(low.pipes[ix].fired.expect("computed before the body"));
    }

    // `if c then a else b`, desugared by the parser. Both arms must agree on a
    // type -- there is no value an expression could take otherwise.
    if op == Select {
        let arity_is_right = args.len() == 3;
        if !arity_is_right {
            sink.err_span(crate::driver::nowhere(), "`if ... then ... else` takes three operands");
            return None;
        }
        let cond = lower_expr(low, &args[0], env, sink)?;
        let cond_is_bool = low.ty_of(cond) == Ty::BOOL;
        if !cond_is_bool {
            sink.push(
                Diag::error(
                    crate::driver::nowhere(),
                    format!(
                        "an `if` condition is `i1`, found `{}`",
                        low.ty_of(cond).display()
                    ),
                )
                .with_note("compare it against something, or slice a single bit out"),
            );
            return None;
        }
        let mut then_val = lower_expr(low, &args[1], env, sink)?;
        let mut else_val = lower_expr(low, &args[2], env, sink)?;

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
                        crate::driver::nowhere(),
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
                        sink.err_span(crate::driver::nowhere(), "`@zext` cannot narrow; use `@trunc`");
                        return None;
                    }
                    Some(low.emit(Ty::UInt(want_w), Op::ZExt { arg, to: want_w }))
                }
                Sext => {
                    let would_narrow = want_w < have_w;
                    if would_narrow {
                        sink.err_span(crate::driver::nowhere(), "`@sext` cannot narrow; use `@trunc`");
                        return None;
                    }
                    Some(low.emit(Ty::SInt(want_w), Op::SExt { arg, to: want_w }))
                }
                _ => {
                    let would_widen = want_w > have_w;
                    if would_widen {
                        sink.err_span(
                            crate::driver::nowhere(),
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
                sink.err_span(crate::driver::nowhere(), "this cast takes exactly one argument");
                return None;
            }
            let arg = lower_expr(low, &args[0], env, sink)?;
            let w = low.ty_of(arg).bit_width();
            let ty = if op == Signed { Ty::SInt(w) } else { Ty::UInt(w) };
            return Some(low.emit(ty, Op::Cast { arg }));
        }
        Concat => {
            if args.is_empty() {
                sink.err_span(crate::driver::nowhere(), "`@concat` needs at least one argument");
                return None;
            }
            let mut parts = Vec::new();
            let mut total = 0u32;
            for a in args {
                let v = lower_expr(low, a, env, sink)?;
                total += low.ty_of(v).bit_width();
                parts.push(v);
            }
            return Some(low.emit(Ty::UInt(total), Op::Concat(parts)));
        }
        Rep => {
            let (arg, times) = cast_args(low, args, env, sink)?;
            if times == 0 {
                sink.err_span(crate::driver::nowhere(), "`@rep` count must be at least 1");
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
            sink.err_span(crate::driver::nowhere(), "this operator takes one operand");
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
                        crate::driver::nowhere(),
                        format!("`!` needs `i1`, found `{}`", ty.display()),
                    );
                    return None;
                }
                (UnOp::LogNot, Ty::BOOL)
            }
        };
        return Some(low.emit(out_ty, Op::Un { op: un, arg }));
    }

    if args.len() != 2 {
        sink.err_span(crate::driver::nowhere(), "this operator takes two operands");
        return None;
    }
    let mut lhs = lower_expr(low, &args[0], env, sink)?;
    let mut rhs = lower_expr(low, &args[1], env, sink)?;

    // Comparisons widen internally and never report a mismatch.
    if let Some(cmp) = cmp_of(op) {
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
            let mut d = Diag::error(crate::driver::nowhere(), e.message());
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
                crate::driver::nowhere(),
                format!("`{:?}` is not available in combinational logic", other),
            );
            return None;
        }
    };
    Some(low.emit(out_ty, Op::Bin { op: bin, lhs, rhs }))
}

/// `(value, constant)` argument pair shared by `@zext`/`@sext`/`@trunc`/`@rep`.
fn cast_args(
    low: &mut Lowerer,
    args: &[PrecResExpr],
    env: &Env,
    sink: &mut DiagSink,
) -> Option<(ValueId, u32)> {
    if args.len() != 2 {
        sink.err_span(
            crate::driver::nowhere(),
            "expected two arguments: a value and a constant width",
        );
        return None;
    }
    let value = lower_expr(low, &args[0], env, sink)?;
    let k = match const_eval(&args[1]) {
        Ok(k) => k,
        Err(_) => {
            sink.err_span(crate::driver::nowhere(), "the second argument must be a constant");
            return None;
        }
    };
    let width_is_sane = k > 0 && k <= 65536;
    if !width_is_sane {
        sink.err_span(crate::driver::nowhere(), "width out of range");
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
