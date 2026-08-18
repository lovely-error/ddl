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

use std::collections::HashMap;

use crate::diag::{Diag, DiagSink, SourceMap, Span};
use crate::lex::{AlphanumSpan, ArgTypeQualifier};
use crate::parse::{
    BuiltinOp, FunctionDecl, Literal, PrecResExpr, PrecResInnerStmt, ProcessDecl,
    anumspan_to_str,
};
use crate::symbols::Symbols;
use crate::ty::{
    self, OpTyError, Ty, binop_result, comparison_operand_ty, const_eval, literal_fits,
    resolve_type_expr,
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

#[derive(Debug)]
pub struct Module {
    pub name: String,
    pub ports: Vec<Port>,
    pub values: Vec<ValueDef>,
    /// Final driver of each output port, in port order.
    pub drivers: Vec<(PortId, ValueId)>,
    /// Empty for a combinational `fun`; a `process` has clk/rst_n and these.
    pub regs: Vec<Reg>,
}

impl Module {
    pub fn is_clocked(&self) -> bool {
        !self.regs.is_empty()
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

// ---- lowering ------------------------------------------------------------

/// What a name is bound to while lowering.
#[derive(Clone)]
pub struct Binding {
    pub value: Option<ValueId>,
    pub ty: Ty,
    /// Output parameters are assigned rather than read; reading one before it
    /// has been written is an error rather than an undefined wire.
    pub is_output: bool,
}

pub type Env = HashMap<String, Binding>;

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
    pub ready_port: PortId,
    pub data_port: PortId,
    /// The value read from the data port; inputs only.
    pub data_value: Option<ValueId>,
    /// Set once the body has done a `@try_rcv` / `@try_send` on this pipe.
    pub used: bool,
    /// What `@try_send` offered; outputs only.
    pub sent: Option<ValueId>,
    /// Inputs: this pipe transferred this cycle. Outputs: the slot can take a
    /// new item. Both are computed before the body, from registers and the
    /// `ready` inputs only.
    pub fired: Option<ValueId>,
    /// Outputs only: the `busy` and `hold` registers backing the slot.
    pub busy_reg: Option<usize>,
    pub hold_reg: Option<usize>,
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
            values: Vec::new(),
            ports: Vec::new(),
        }
    }

    pub fn take_values(self) -> (Vec<ValueDef>, Vec<Port>) {
        (self.values, self.ports)
    }

    pub fn name_value(&mut self, v: ValueId, name: String) {
        self.values[v.0 as usize].name = Some(name);
    }

    pub fn emit(&mut self, ty: Ty, op: Op) -> ValueId {
        let id = ValueId(self.values.len() as u32);
        self.values.push(ValueDef { id, ty, op, name: None });
        id
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
    let mut env: Env = HashMap::new();
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
                env.insert(name, Binding { value: Some(v), ty, is_output: false });
            }
            PortDir::Out => {
                out_ports.push((port_id, name.clone()));
                env.insert(name, Binding { value: None, ty, is_output: true });
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
    let mut env: Env = HashMap::new();
    let mut out_ports: Vec<(PortId, String)> = Vec::new();

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
        env.insert(
            implicit.to_string(),
            Binding { value: Some(v), ty: Ty::BOOL, is_output: false },
        );
    }

    for arg in &decl.args.entries {
        let name = anumspan_to_str(&arg.arg_name).to_string();
        if name == "clk" || name == "rst_n" {
            sink.err_at(&arg.arg_name, format!("`{}` is implicit on a process", name));
            return None;
        }
        let ty = match resolve_type_expr(&arg.type_expr, syms) {
            Ok(t) => t,
            Err(e) => {
                sink.err_at(&arg.arg_name, e.message());
                return None;
            }
        };
        // A pipe becomes three flat ports. That is the flattening
        // k3g_chan.sv:60 already pre-commits to for the yosys-slang risk --
        // "every process port list flattens to valid/ready/data triples and
        // the rules stay exactly as written".
        let pipe_dir = match arg.qualifier {
            ArgTypeQualifier::BufferIn => Some(true),
            ArgTypeQualifier::BufferOut => Some(false),
            ArgTypeQualifier::StreamIn | ArgTypeQualifier::StreamOut => {
                sink.err_at(
                    &arg.arg_name,
                    "`stream` pipes are not supported yet; `buffer` is",
                );
                return None;
            }
            _ => None,
        };
        if let Some(is_input) = pipe_dir {
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
            let ready_port = mk(&mut low, "ready", rdir, Ty::BOOL);
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
                data_port,
                data_value,
                used: false,
                sent: None,
                fired: None,
                busy_reg: None,
                hold_reg: None,
            });
            continue;
        }

        let dir = match arg.qualifier {
            ArgTypeQualifier::In => PortDir::In,
            ArgTypeQualifier::Out => PortDir::Out,
            ArgTypeQualifier::Inout => {
                sink.err_at(&arg.arg_name, "`inout` parameters are not supported yet");
                return None;
            }
            _ => unreachable!("pipe qualifiers handled above"),
        };

        let port_id = PortId(low.ports.len() as u32);
        low.ports.push(Port { name: name.clone(), dir, ty: ty.clone() });

        match dir {
            PortDir::In => {
                let v = low.emit(ty.clone(), Op::Port(port_id));
                low.values[v.0 as usize].name = Some(name.clone());
                env.insert(name, Binding { value: Some(v), ty, is_output: false });
            }
            PortDir::Out => {
                out_ports.push((port_id, name.clone()));
                env.insert(name, Binding { value: None, ty, is_output: true });
            }
        }
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
        env.insert(
            name.clone(),
            Binding { value: Some(held), ty: ty.clone(), is_output: false },
        );
        reg_names.push(name);
        reg_resets.push(reset);
        reg_tys.push(ty);
        body_start += 1;
    }

    let has_pipes = !low.pipes.is_empty();
    if out_ports.is_empty() && !has_pipes {
        sink.err_span(
            map.span_of(&decl.name),
            "a process needs at least one `out` parameter or pipe",
        );
        return None;
    }

    // A body that is one `loop` uses blocking channel operations and becomes a
    // state machine instead of one pass per cycle.
    let rest = &decl.body[body_start..];
    let is_blocking = rest.len() == 1 && matches!(&rest[0], PrecResInnerStmt::Loop(_));
    if is_blocking {
        let loop_body = match &rest[0] {
            PrecResInnerStmt::Loop(l) => match &l.repeat_expr {
                PrecResExpr::StmtBlock(b) => b.components.clone(),
                _ => {
                    sink.err_span(
                        map.span_of(&decl.name),
                        "a `loop` in a process needs an indented body",
                    );
                    return None;
                }
            },
            _ => unreachable!(),
        };
        return crate::ir_fsm::lower_blocking(
            map, decl, low, env, out_ports, reg_names, reg_tys, reg_resets, &loop_body, sink,
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

    for ix in 0..low.pipes.len() {
        if low.pipes[ix].is_input {
            continue;
        }
        let ty = low.pipes[ix].ty.clone();
        let pname = low.pipes[ix].name.clone();

        let busy_ix = reg_names.len() + generated.len();
        let busy = low.emit(Ty::BOOL, Op::RegRead(busy_ix as u32));
        low.values[busy.0 as usize].name = Some(format!("{}_busy", pname));
        let hold_ix = busy_ix + 1;
        let hold = low.emit(ty.clone(), Op::RegRead(hold_ix as u32));
        low.values[hold.0 as usize].name = Some(format!("{}_hold", pname));

        // The slot can take a new item when it is empty, or is draining now.
        let ready_in = low.emit(Ty::BOOL, Op::Port(low.pipes[ix].ready_port));
        let not_busy = low.emit(Ty::BOOL, Op::Un { op: UnOp::LogNot, arg: busy });
        let accept = low.emit(
            Ty::BOOL,
            Op::Bin { op: BinOp::Or, lhs: not_busy, rhs: ready_in },
        );
        accepts.push(accept);

        generated.push(Reg { name: format!("{}_busy", pname), ty: Ty::BOOL, reset: 0, next: busy });
        generated.push(Reg { name: format!("{}_hold", pname), ty, reset: 0, next: hold });

        low.pipes[ix].fired = Some(accept);
        low.pipes[ix].busy_reg = Some(busy_ix);
        low.pipes[ix].hold_reg = Some(hold_ix);
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

    for ix in 0..low.pipes.len() {
        if !low.pipes[ix].is_input {
            continue;
        }
        let up_valid = low.emit(Ty::BOOL, Op::Port(low.pipes[ix].valid_port));
        let fired = low.emit(
            Ty::BOOL,
            Op::Bin { op: BinOp::And, lhs: up_valid, rhs: ready_out },
        );
        low.values[fired.0 as usize].name = Some(format!("{}_xfer", low.pipes[ix].name));
        low.pipes[ix].fired = Some(fired);
    }

    lower_stmts(&mut low, &decl.body[body_start..], &mut env, sink)?;

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
            drivers.push((pipe.ready_port, ready_out));
            continue;
        }
        let busy_ix = pipe.busy_reg.expect("an output pipe has a busy register");
        let hold_ix = pipe.hold_reg.expect("an output pipe has a hold register");
        let busy = low.emit(Ty::BOOL, Op::RegRead(busy_ix as u32));
        let hold = low.emit(pipe.ty.clone(), Op::RegRead(hold_ix as u32));
        let ready_in = low.emit(Ty::BOOL, Op::Port(pipe.ready_port));

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

        // busy <= fired ? 1 : (ready ? 0 : busy)
        let one = low.emit(Ty::BOOL, Op::Const(1));
        let zero = low.emit(Ty::BOOL, Op::Const(0));
        let drained = low.emit(
            Ty::BOOL,
            Op::Mux { cond: ready_in, then_val: zero, else_val: busy },
        );
        let busy_next = low.emit(
            Ty::BOOL,
            Op::Mux { cond: fired_any, then_val: one, else_val: drained },
        );
        let hold_next = low.emit(
            pipe.ty.clone(),
            Op::Mux { cond: fired_any, then_val: sent, else_val: hold },
        );

        let idx = busy_ix - reg_names.len();
        generated[idx].next = busy_next;
        generated[idx + 1].next = hold_next;

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
    if regs.is_empty() && !has_pipes {
        sink.err_span(
            map.span_of(&decl.name),
            "a process with no state should be a `fun`",
        );
        return None;
    }

    Some(Module {
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
    env.insert(item, Binding { value: Some(data), ty, is_output: false });
    env.insert(got, Binding { value: Some(fired), ty: Ty::BOOL, is_output: false });
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

            // `let (item, got) = @try_rcv(p)`. A non-blocking receive answers
            // with both, and there is no way to use it without taking both.
            if !decl.rest.is_empty() {
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
            env.insert(name, Binding { value: Some(value), ty, is_output: false });
            Some(())
        }

        PrecResInnerStmt::AssignStmt(assign) => {
            // An assignment target is a path: a name, optionally followed by
            // field accesses. `uop.cond_reg = arg1` is how k2g_decode builds a
            // 28-field struct, so a bare name is not enough.
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
            let base_ty = match env.get(&name) {
                Some(b) => b.ty.clone(),
                None => {
                    sink.err_at(&target, format!("`{}` is not declared", name));
                    return None;
                }
            };
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
            lower_branch(low, &ite.then_case, &mut then_env, sink)?;

            let mut else_env = env.clone();
            if let Some(else_case) = &ite.else_case {
                lower_branch(low, else_case, &mut else_env, sink)?;
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

        PrecResInnerStmt::TailVal(_) | PrecResInnerStmt::CallStmt(_) => {
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

        PrecResInnerStmt::Loop(_)
        | PrecResInnerStmt::Break
        | PrecResInnerStmt::ForLoop(_) => {
            sink.err_span(
                crate::driver::nowhere(),
                "loops need a state machine; they are not available in a `fun` yet",
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
