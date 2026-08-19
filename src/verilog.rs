// Verilog-2005 backend.
//
// Verilog-2005 rather than SystemVerilog because it sidesteps, by
// construction, every construct the target toolchain is known to die on. From
// docs/bring-up.md and docs/development.md, GowinSynthesis exits 1 with an
// EMPTY LOG -- not even its startup banner -- on:
//
//   * `$clog2` inline in a part-select index
//   * a width cast in an expression, e.g. `21'(STEP * (N-1))` or `(AW+1)'(x)`
//   * a function call inside a ternary in a continuous assign
//
// None of the three can be emitted from here. Every width is already a
// concrete number in the IR's `Ty`, so widths are printed as literals and
// `$clog2` never appears; extension is written as a concatenation, which is
// what k2g_cdc_fifo.sv:91 does by hand for exactly this reason; and there are
// no functions in the output at all.
//
// The same output is accepted by Questa `vlog -sv`, `gw_sh`, and yosys-slang,
// so one backend serves simulation, the FPGA build and the ASIC flow.

use crate::ir::{BinOp, CmpOp, Memory, Module, Op, Port, PortDir, UnOp, ValueDef, ValueId};
use crate::ty::{MemKind, Ty};

pub struct EmitOptions {
    /// Printed in the banner so a reader knows how to regenerate the file.
    pub regenerate_cmd: String,
}

impl Default for EmitOptions {
    fn default() -> Self {
        EmitOptions { regenerate_cmd: "ddl build <source.ddl>".to_string() }
    }
}

/// The file banner, emitted once regardless of how many modules follow.
///
/// Matches the shape emu/src/sv_gen.rs uses for k2g_pkg.sv, including naming
/// the regenerate command: that is the established convention here, and
/// `gen_defs --check` depends on generated files being recognisable as such.
pub fn emit_banner(opts: &EmitOptions) -> String {
    let mut out = String::new();
    out.push_str("// GENERATED FILE -- DO NOT EDIT BY HAND
");
    out.push_str("//
");
    out.push_str(&format!("// Regenerate with: {}
", opts.regenerate_cmd));
    out.push_str("//
");
    out.push_str("// Verilog-2005. No `$clog2`, no width casts in expressions and no
");
    out.push_str("// function calls: all three make GowinSynthesis exit with an empty log.
");
    out
}

pub fn emit_module(module: &Module, _opts: &EmitOptions) -> String {
    let mut out = String::new();
    let names = NameTable::build(module);


    emit_header(&mut out, module);

    // Built separately so an empty body -- a module that is pure wiring --
    // does not leave a double blank line behind.
    let mut body = String::new();
    emit_body(&mut body, module, &names);
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&body);
    }

    out.push_str("\nendmodule\n");
    out
}

fn emit_header(out: &mut String, module: &Module) {
    // Constants are folded to literals before they get here, so without this
    // the file gives no hint that the shape it has was a choice.
    if !module.params.is_empty() {
        out.push_str("// Built with:\n");
        for (name, ty, value) in &module.params {
            out.push_str(&format!(
                "//   {} : {} = {}\n",
                name,
                ty.display(),
                render_const(*value, ty)
            ));
        }
        out.push_str("//\n");
    }
    out.push_str(&format!("module {} (\n", sanitize(&module.name)));

    // Widest declaration prefix, so the port names line up in a column.
    let decls: Vec<String> = module
        .ports
        .iter()
        .map(|p| {
            let dir = match p.dir {
                PortDir::In => "input ",
                PortDir::Out => "output",
            };
            // `signed` belongs on the port declaration too, or a signed
            // result silently becomes unsigned at the module boundary.
            let decor = signed_and_range(&p.ty);
            if decor.is_empty() {
                format!("    {}", dir)
            } else {
                format!("    {} {}", dir, decor.trim_end())
            }
        })
        .collect();
    let width = decls.iter().map(|d| d.len()).max().unwrap_or(0);

    for (ix, (port, decl)) in module.ports.iter().zip(decls.iter()).enumerate() {
        let comma = if ix + 1 == module.ports.len() { "" } else { "," };
        out.push_str(&format!(
            "{:<width$} {}{}\n",
            decl,
            sanitize(&port.name),
            comma,
            width = width
        ));
    }
    out.push_str(");\n");
}

/// Values that actually reach an output port.
///
/// Lowering produces some values that nothing reads -- most commonly the
/// initialiser of a binding that every branch of a following `if` overwrites.
/// Synthesis would strip them, but leaving them in the file makes the output
/// harder to read and harder to diff against hand-written RTL.
fn live_values(module: &Module) -> Vec<bool> {
    let mut live = vec![false; module.values.len()];
    // Roots: everything an output drives, and everything a register clocks in.
    let mut stack: Vec<ValueId> = module.drivers.iter().map(|(_, v)| *v).collect();
    stack.extend(module.regs.iter().map(|r| r.next));
    for mem in &module.mems {
        stack.push(mem.we);
        stack.push(mem.addr);
        stack.push(mem.data);
    }
    // An assertion is a use even though nothing downstream reads it, or the
    // whole cone feeding it would look dead and be stripped.
    stack.extend(module.asserts.iter().map(|a| a.cond));

    while let Some(id) = stack.pop() {
        let ix = id.0 as usize;
        if live[ix] {
            continue;
        }
        live[ix] = true;
        let mut push = |v: &ValueId| stack.push(*v);
        match &module.values[ix].op {
            // Leaves. A memory's read register is driven by the memory's own
            // clocked block, so nothing in the value graph computes it.
            Op::Port(_) | Op::RegRead(_) | Op::Const(_) | Op::MemReadReg { .. } => {}
            Op::Bin { lhs, rhs, .. } | Op::Cmp { lhs, rhs, .. } => {
                push(lhs);
                push(rhs);
            }
            Op::Un { arg, .. }
            | Op::Slice { arg, .. }
            | Op::Repeat { arg, .. }
            | Op::ZExt { arg, .. }
            | Op::SExt { arg, .. }
            | Op::Trunc { arg, .. }
            | Op::Cast { arg } => push(arg),
            Op::DynSlice { arg, base, .. } => {
                push(arg);
                push(base);
            }
            Op::Concat(parts) => parts.iter().for_each(&mut push),
            Op::Mux { cond, then_val, else_val } => {
                push(cond);
                push(then_val);
                push(else_val);
            }
            Op::Case { scrutinee, arms, default } => {
                push(scrutinee);
                for (_, v) in arms {
                    push(v);
                }
                push(default);
            }
            Op::MemRead { addr, .. } => push(addr),
        }
    }
    live
}

/// How many times each live value is read.
fn use_counts(module: &Module, live: &[bool]) -> Vec<u32> {
    let mut counts = vec![0u32; module.values.len()];
    let bump = |id: &ValueId, counts: &mut Vec<u32>| counts[id.0 as usize] += 1;

    for def in &module.values {
        if !live[def.id.0 as usize] {
            continue;
        }
        match &def.op {
            Op::Port(_) | Op::RegRead(_) | Op::Const(_) | Op::MemReadReg { .. } => {}
            Op::Bin { lhs, rhs, .. } | Op::Cmp { lhs, rhs, .. } => {
                bump(lhs, &mut counts);
                bump(rhs, &mut counts);
            }
            Op::Un { arg, .. }
            | Op::Slice { arg, .. }
            | Op::Repeat { arg, .. }
            | Op::ZExt { arg, .. }
            | Op::SExt { arg, .. }
            | Op::Trunc { arg, .. }
            | Op::Cast { arg } => bump(arg, &mut counts),
            Op::DynSlice { arg, base, .. } => {
                bump(arg, &mut counts);
                bump(base, &mut counts);
            }
            Op::Concat(parts) => {
                for p in parts {
                    bump(p, &mut counts);
                }
            }
            Op::Mux { cond, then_val, else_val } => {
                bump(cond, &mut counts);
                bump(then_val, &mut counts);
                bump(else_val, &mut counts);
            }
            Op::Case { scrutinee, arms, default } => {
                bump(scrutinee, &mut counts);
                for (_, v) in arms {
                    bump(v, &mut counts);
                }
                bump(default, &mut counts);
            }
            Op::MemRead { addr, .. } => bump(addr, &mut counts),
        }
    }
    for (_, v) in &module.drivers {
        bump(v, &mut counts);
    }
    // A register's next value is a use too, or anything feeding only a
    // register would look dead and be eliminated.
    for reg in &module.regs {
        bump(&reg.next, &mut counts);
    }
    for mem in &module.mems {
        bump(&mem.we, &mut counts);
        bump(&mem.addr, &mut counts);
        bump(&mem.data, &mut counts);
    }
    for a in &module.asserts {
        bump(&a.cond, &mut counts);
    }
    counts
}

/// Values folded into their consumer's expression instead of getting a wire.
///
/// This is not cosmetic. GowinSynthesis emits
/// `ERROR (SP00018) : ... error bus name set` once per BIT of every named
/// intermediate bus its optimizer eliminates -- the same logic written with
/// named wires produced 64 errors where one nested expression produced none,
/// with an identical netlist either way. The errors are spurious, but a log
/// with hundreds of them is one where a real error cannot be seen, and this
/// toolchain's genuine failures are already documented as easy to miss.
///
/// A value is folded when it is read exactly once and did not come from a
/// named `let`. Constants are always folded: `32'd1` reads better inline, and
/// a constant has no bus to eliminate.
fn foldable(module: &Module, live: &[bool]) -> Vec<bool> {
    let counts = use_counts(module, live);

    // A part-select needs a NAME to select from. Verilog-2005 allows neither
    // `15'd0[12:0]` nor `{a, b}[14:8]`, and both fall out of folding: the
    // first from always folding constants, the second from field assignment,
    // which rebuilds a struct by concatenation and then slices it apart again.
    let mut must_be_named = vec![false; module.values.len()];
    for def in &module.values {
        if !live[def.id.0 as usize] {
            continue;
        }
        match &def.op {
            // `case (x)` selects on a signal, and the arm values are assigned
            // inside the block, so none of them may be folded away.
            Op::Case { scrutinee, .. } => must_be_named[scrutinee.0 as usize] = true,
            Op::Slice { arg, .. } => must_be_named[arg.0 as usize] = true,
            Op::DynSlice { arg, base, .. } => {
                must_be_named[arg.0 as usize] = true;
                must_be_named[base.0 as usize] = true;
            }
            _ => {}
        }
    }

    module
        .values
        .iter()
        .map(|def| match &def.op {
            // A port or a register is a signal in its own right, and a case
            // drives a reg from a procedural block.
            // A memory read is an array subscript, which needs its own wire
            // for the same reason: it may not be folded into a bigger
            // expression that is then sliced.
            Op::Port(_) | Op::RegRead(_) | Op::Case { .. } | Op::MemRead { .. } => false,
            Op::Const(_) => !must_be_named[def.id.0 as usize],
            // A cast that changes nothing about the bits renders as its
            // operand. One that changes signedness must keep its wire, since
            // that is where `signed` is written.
            Op::Cast { .. } => {
                is_pure_rename(module, def) && !must_be_named[def.id.0 as usize]
            }
            _ if must_be_named[def.id.0 as usize] => false,
            _ => {
                let read_once = counts[def.id.0 as usize] == 1;
                // A `let` keeps its wire: the name is the reader's link back
                // to the DDL source.
                let is_anonymous = def.name.is_none();

                // Signedness lives on the wire DECLARATION, so folding a
                // signed value into an expression silently loses it. That is
                // not cosmetic: `>>>` on an unsigned operand is a logical
                // shift, so folding turned SHRA into SHR and sign extension
                // vanished -- caught as `ref=ffffdead ddl=0000dead`, on a
                // netlist that was 2.6% SMALLER for being wrong.
                let carries_signedness = def.ty.is_signed();

                read_once && is_anonymous && !carries_signedness
            }
        })
        .collect()
}

/// Renders a reference to `id`: its name, or its whole expression when folded.
///
/// A folded expression is parenthesised, which makes precedence irrelevant --
/// the alternative is a precedence table that has to agree with Verilog's
/// exactly, and being wrong there is silent.
fn operand(module: &Module, names: &NameTable, fold: &[bool], id: ValueId) -> String {
    let def = module.value(id);
    if !fold[id.0 as usize] {
        return names.get(id);
    }
    // A folded cast renders as exactly its operand, so delegate rather than
    // wrapping it in a second layer of parentheses.
    if let Op::Cast { arg } = &def.op {
        return operand(module, names, fold, *arg);
    }
    let rendered = render_op(module, names, fold, &def.op, &def.ty);
    // Self-delimiting forms need no parentheses and read worse with them.
    let is_self_delimiting = matches!(
        def.op,
        Op::Const(_)
            | Op::Concat(_)
            | Op::Repeat { .. }
            | Op::Slice { .. }
            | Op::DynSlice { .. }
            // A memory's read register renders as a bare identifier, exactly
            // like the register read it is.
            | Op::MemReadReg { .. }
            | Op::MemRead { .. }
    );
    if is_self_delimiting {
        rendered
    } else {
        format!("({})", rendered)
    }
}


/// One `always @*` per case-driven value.
///
/// Measured, not stylistic: on the GW1NR-9C a 20-way selection costs 536 cells
/// written as nested ternaries and 210 written as a case. The ternary chain is
/// a priority structure that synthesis must honour in order; a case says the
/// arms are parallel.
///
/// Plain `case` rather than `unique case` -- the 210 figure is without it, and
/// `unique` is not Verilog-2005 anyway.
fn emit_case_blocks(
    out: &mut String,
    module: &Module,
    names: &NameTable,
    fold: &[bool],
    live: &[bool],
) {
    for def in &module.values {
        if !live[def.id.0 as usize] {
            continue;
        }
        let (scrutinee, arms, default) = match &def.op {
            Op::Case { scrutinee, arms, default } => (scrutinee, arms, default),
            _ => continue,
        };
        let target = names.get(def.id);
        let sel_ty = &module.value(*scrutinee).ty;

        out.push('\n');
        out.push_str("  always @* begin\n");
        out.push_str(&format!(
            "    case ({})\n",
            operand(module, names, fold, *scrutinee)
        ));
        for (labels, value) in arms {
            // Several labels on one arm is an or-pattern, and is exactly the
            // `LB_ADD, LB_SUB, LB_MUL, LB_DIV:` form of the SystemVerilog.
            let rendered: Vec<String> =
                labels.iter().map(|k| render_const(*k, sel_ty)).collect();
            out.push_str(&format!(
                "      {}: {} = {};\n",
                rendered.join(", "),
                target,
                operand(module, names, fold, *value)
            ));
        }
        out.push_str(&format!(
            "      default: {} = {};\n",
            target,
            operand(module, names, fold, *default)
        ));
        out.push_str("    endcase\n");
        out.push_str("  end\n");
    }
}

fn emit_body(out: &mut String, module: &Module, names: &NameTable) {
    let live = live_values(module);
    let fold = foldable(module, &live);

    // A graph's wires. Declared before anything else because the instances
    // that drive them follow, and a Verilog-2005 net must be declared before
    // it is used.
    for net in &module.nets {
        out.push_str(&format!(
            "  wire {}{};\n",
            signed_and_range(&net.ty),
            sanitize(&net.name)
        ));
    }
    if !module.nets.is_empty() {
        out.push('\n');
    }

    for reg in &module.regs {
        out.push_str(&format!(
            "  reg {}{};\n",
            signed_and_range(&reg.ty),
            sanitize(&reg.name)
        ));
    }
    if !module.regs.is_empty() {
        out.push('\n');
    }

    for mem in &module.mems {
        out.push_str(&format!(
            "  reg {}{} [0:{}];\n",
            signed_and_range(&mem.elem),
            sanitize(&mem.name),
            mem.len - 1
        ));
        // The read port's register belongs to the memory and is declared
        // with it, so the two read as one thing.
        if mem.read.is_some() {
            out.push_str(&format!(
                "  reg {}{}_q;
",
                signed_and_range(&mem.elem),
                sanitize(&mem.name)
            ));
        }
        // Verilog-2005 has no `for (integer i = ...)`, so the loop variable is
        // a module-level `integer`. It is touched only in the reset branch.
        if mem.reset.is_some() {
            out.push_str(&format!("  integer {}_ix;\n", sanitize(&mem.name)));
        }
    }
    if !module.mems.is_empty() {
        out.push('\n');
    }

    for def in &module.values {
        if !live[def.id.0 as usize] {
            continue;
        }
        // A port or a register is already a signal; re-declaring it would
        // collide with its own declaration.
        if matches!(def.op, Op::Port(_) | Op::RegRead(_)) {
            continue;
        }
        // A folded value lives inside its consumer instead.
        if fold[def.id.0 as usize] {
            continue;
        }
        // A case assigns from a procedural block, so it needs a `reg` and its
        // value comes later.
        if let Op::Case { .. } = def.op {
            out.push_str(&format!(
                "  reg {}{};\n",
                signed_and_range(&def.ty),
                names.get(def.id)
            ));
            continue;
        }
        let name = names.get(def.id);
        let rhs = render_op(module, names, &fold, &def.op, &def.ty);
        out.push_str(&format!(
            "  wire {}{} = {};\n",
            signed_and_range(&def.ty),
            name,
            rhs
        ));
    }

    emit_case_blocks(out, module, names, &fold, &live);

    let emitted_declarations = !out.is_empty();
    if !module.drivers.is_empty() {
        if emitted_declarations {
            out.push('\n');
        }
        for (port_id, value_id) in &module.drivers {
            let port: &Port = module.port(*port_id);
            out.push_str(&format!(
                "  assign {} = {};\n",
                sanitize(&port.name),
                operand(module, names, &fold, *value_id)
            ));
        }
    }

    if !module.regs.is_empty() {
        emit_clocked_block(out, module, names, &fold);
    }
    for mem in &module.mems {
        emit_memory_block(out, module, names, &fold, mem);
    }
    emit_instances(out, module);
    emit_assertions(out, module, names, &fold);
}

/// A graph's submodules, connected by name.
///
/// By name and never by position: the port order of a lowered process is three
/// ports per pipe in an order ir.rs chose, and a positional connection would
/// turn a change there into a silently miswired design rather than a compile
/// error.
fn emit_instances(out: &mut String, module: &Module) {
    for inst in &module.instances {
        // One blank line between instances, and none doubled up against the
        // one the net declarations already left behind.
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&format!(
            "  {} {} (\n",
            sanitize(&inst.module),
            sanitize(&inst.name)
        ));
        let width = inst
            .conns
            .iter()
            .map(|(formal, _)| formal.len())
            .max()
            .unwrap_or(0);
        for (ix, (formal, actual)) in inst.conns.iter().enumerate() {
            let comma = if ix + 1 == inst.conns.len() { "" } else { "," };
            out.push_str(&format!(
                "    .{:<width$} ({}){}\n",
                sanitize(formal),
                sanitize(actual),
                comma,
                width = width
            ));
        }
        out.push_str("  );\n");
    }
}

/// Immediate assertions, and nothing of them in synthesis.
///
/// Guarded on SIMULATION rather than on the absence of SYNTHESIS, because
/// GowinSynthesis does not define SYNTHESIS -- docs/gowin-sv-support.md, and
/// the hand-written RTL guards all fifteen of its assertion sites the same
/// way. `$error` and `$fatal` are SystemVerilog system tasks, which is fine
/// precisely because nothing outside a simulator ever reads this block.
///
/// In a clocked module the checks run on the edge and are held off during
/// reset: registers hold their reset value then, and an assertion about what
/// the design computes has nothing to say about a design that is being held.
fn emit_assertions(out: &mut String, module: &Module, names: &NameTable, fold: &[bool]) {
    if module.asserts.is_empty() {
        return;
    }
    let clocked = module.is_clocked();
    out.push('\n');
    out.push_str("`ifdef SIMULATION\n");
    if clocked {
        out.push_str("  always @(posedge clk) begin\n");
        out.push_str("    if (rst_n) begin\n");
    } else {
        out.push_str("  always @* begin\n");
    }
    // Two levels inside a clocked block, one inside a combinational one.
    let indent = if clocked { "      " } else { "    " };
    for a in &module.asserts {
        let task = if a.is_fatal { "$fatal(1, " } else { "$error(" };
        out.push_str(&format!(
            "{}if (!({})) {}\"%m: {}\");\n",
            indent,
            operand(module, names, fold, a.cond),
            task,
            escape_message(&a.message)
        ));
    }
    if clocked {
        out.push_str("    end\n");
    }
    out.push_str("  end\n");
    out.push_str("`endif\n");
}

/// Makes a message safe to sit inside a Verilog string.
///
/// A `%` in the text would be read as a format specifier by `$error` and would
/// consume an argument that is not there.
fn escape_message(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    for ch in msg.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("%%"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// One memory, in the shape GowinSynthesis infers a RAM from.
///
/// Measured constraints from docs/gowin-sv-support.md: SSRAM inference needs
/// ONE synchronous write port and asynchronous reads. Two write ports infer no
/// RAM at all -- the K2G value array collapsed to ~3700 LUTs when a second was
/// added -- so the IR carries exactly one write port and several source writes
/// mux onto it.
///
/// The reset loop is emitted only when the source asked for one. It is not
/// free: k2g_regfile.sv:76 measures it at ~85 LUTs plus some extra RAM
/// primitives. It is usually worth paying, because without it the array powers
/// up undefined and cosimulation cannot compare a register until something
/// writes it -- but that is the source's call, not this backend's.
/// The name of a memory's read-port register.
///
/// Derived from the SANITIZED array name, so the two always agree and the
/// register visibly belongs to the array: a memory called `table` escapes to
/// `table_`, and its read register is `table__q` rather than `table_q`, which
/// would read as a different memory's.
pub fn read_reg_name(module: &Module, mem: u32) -> String {
    format!("{}_q", sanitize(&module.mems[mem as usize].name))
}

fn emit_memory_block(
    out: &mut String,
    module: &Module,
    names: &NameTable,
    fold: &[bool],
    mem: &Memory,
) {
    let name = sanitize(&mem.name);
    // A memory nothing writes is a lookup table. Emitting `if (1'b0)` around a
    // dead assignment would only give synthesis something to warn about -- but
    // a synchronous read still needs its block, because that is where its
    // register is driven from.
    let never_written = matches!(module.value(mem.we).op, Op::Const(0));
    if never_written && mem.reset.is_none() && mem.read.is_none() {
        return;
    }
    let we = operand(module, names, fold, mem.we);
    let addr = operand(module, names, fold, mem.addr);
    let data = operand(module, names, fold, mem.data);

    out.push('\n');
    out.push_str(&format!("  // {} [0:{}] -- {}\n", name, mem.len - 1, mem_note(mem.kind)));
    out.push_str("  always @(posedge clk) begin\n");

    let mut write_open = false;
    match mem.reset {
        Some(k) => {
            out.push_str("    if (!rst_n) begin\n");
            out.push_str(&format!(
                "      for ({ix} = 0; {ix} < {len}; {ix} = {ix} + 1) {nm}[{ix}] <= {val};\n",
                ix = format!("{}_ix", name),
                len = mem.len,
                nm = name,
                val = render_const(k, &mem.elem)
            ));
            if never_written {
                out.push_str("    end\n");
            } else {
                out.push_str(&format!("    end else if ({}) begin\n", we));
                write_open = true;
            }
        }
        None => {
            if !never_written {
                out.push_str(&format!("    if ({}) begin\n", we));
                write_open = true;
            }
        }
    }
    if write_open {
        out.push_str(&format!("      {}[{}] <= {};\n", name, addr, data));
        out.push_str("    end\n");
    }

    // THE READ, INSIDE THE SAME BLOCK. This is what makes it a block RAM
    // rather than a distributed one with a flop bolted on: the array is read
    // on the clock edge, in the block that owns the array, and the value lands
    // in a register nothing outside can see unregistered.
    //
    // A `wire q = mem[addr];` with the flop in some other always block is a
    // combinational array read plus a register, and infers exactly what
    // `lutram` already gives you, plus the flop.
    //
    // The enable holds the value rather than letting the read free-run,
    // because the state that consumes it may wait any number of cycles on a
    // handshake. A read enable is part of the template synthesizers recognise.
    if let Some(r) = &mem.read {
        out.push_str(&format!(
            "    if ({}) {}_q <= {}[{}];\n",
            operand(module, names, fold, r.en),
            name,
            name,
            operand(module, names, fold, r.addr)
        ));
    }
    out.push_str("  end\n");
}

/// The resource each memory asked for, as a comment above its always block.
///
/// Nothing in Verilog-2005 says "put this in block RAM", so the request
/// survives as inference shape plus this note. A reader diffing the output
/// against the DDL source needs to see that the request was heard.
fn mem_note(kind: MemKind) -> &'static str {
    match kind {
        MemKind::LutRam => "distributed RAM: one sync write port, async reads",
        MemKind::BlockRam => "block RAM: one sync write port, sync reads",
        MemKind::BankedRam => "banked RAM",
    }
}

/// The one sequential block: synchronous, active-low reset, `posedge clk`.
///
/// House style per docs/development.md -- `always_ff` and `always_comb` only,
/// active-low synchronous reset tested first. This emits `always` rather than
/// `always_ff` because the output is Verilog-2005; the shape is the same.
fn emit_clocked_block(out: &mut String, module: &Module, names: &NameTable, fold: &[bool]) {
    out.push('\n');
    out.push_str("  always @(posedge clk) begin\n");
    out.push_str("    if (!rst_n) begin\n");
    for reg in &module.regs {
        out.push_str(&format!(
            "      {} <= {};\n",
            sanitize(&reg.name),
            render_const(reg.reset, &reg.ty)
        ));
    }
    out.push_str("    end else begin\n");
    for reg in &module.regs {
        out.push_str(&format!(
            "      {} <= {};\n",
            sanitize(&reg.name),
            operand(module, names, fold, reg.next)
        ));
    }
    out.push_str("    end\n");
    out.push_str("  end\n");
}

/// `[31:0]` for a multi-bit type, empty for a single bit.
fn range_of(ty: &Ty) -> String {
    let w = ty.bit_width();
    if w == 1 {
        String::new()
    } else {
        format!("[{}:0]", w - 1)
    }
}

/// Signedness lives on the declaration rather than in `$signed(...)` casts,
/// matching the house style of `wire logic signed [31:0] value_s = value;`
/// in k2g_shift.sv:31. Includes a trailing space when non-empty so callers can
/// splice it straight before an identifier.
fn signed_and_range(ty: &Ty) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if ty.is_signed() {
        parts.push("signed");
    }
    let range = range_of(ty);
    if !range.is_empty() {
        parts.push(&range);
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("{} ", parts.join(" "))
}

fn render_op(module: &Module, names: &NameTable, fold: &[bool], op: &Op, ty: &Ty) -> String {
    match op {
        Op::MemReadReg { mem } => read_reg_name(module, *mem),
        Op::MemRead { mem, addr } => {
            let m = &module.mems[*mem as usize];
            format!("{}[{}]", sanitize(&m.name), operand(module, names, fold, *addr))
        }
        // Emitted as its own always block by emit_case_blocks, never inline.
        Op::Case { .. } => unreachable!("a case is emitted as a procedural block"),
        Op::Port(id) => sanitize(&module.port(*id).name),
        Op::RegRead(ix) => sanitize(&module.regs[*ix as usize].name),

        Op::Const(value) => render_const(*value, ty),

        Op::Bin { op, lhs, rhs } => {
            let sym = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Mod => "%",
                BinOp::Shl => "<<",
                // A signed left operand means an arithmetic shift. The wire
                // carrying it was declared `signed`, but `>>>` is spelled
                // explicitly so the intent survives a reader's glance.
                BinOp::Shr => {
                    if module.value(*lhs).ty.is_signed() { ">>>" } else { ">>" }
                }
                BinOp::And => "&",
                BinOp::Or => "|",
                BinOp::Xor => "^",
            };
            format!("{} {} {}", operand(module, names, fold, *lhs), sym, operand(module, names, fold, *rhs))
        }

        Op::Cmp { op, lhs, rhs } => {
            let sym = match op {
                CmpOp::Eq => "==",
                CmpOp::Ne => "!=",
                CmpOp::Lt => "<",
                CmpOp::Gt => ">",
                CmpOp::Le => "<=",
                CmpOp::Ge => ">=",
            };
            format!("{} {} {}", operand(module, names, fold, *lhs), sym, operand(module, names, fold, *rhs))
        }

        Op::Un { op, arg } => match op {
            UnOp::BitNot => format!("~{}", operand(module, names, fold, *arg)),
            UnOp::Neg => format!("-{}", operand(module, names, fold, *arg)),
            UnOp::LogNot => format!("!{}", operand(module, names, fold, *arg)),
        },

        Op::Slice { arg, hi, lo } => {
            if hi == lo {
                format!("{}[{}]", operand(module, names, fold, *arg), hi)
            } else {
                format!("{}[{}:{}]", operand(module, names, fold, *arg), hi, lo)
            }
        }

        // The computed-base part-select. The base is a signal, never an
        // expression containing `$clog2` -- that form is one of the silent
        // synthesis failures.
        Op::DynSlice { arg, base, width } => {
            format!("{}[{} +: {}]", operand(module, names, fold, *arg), operand(module, names, fold, *base), width)
        }

        Op::Concat(parts) => {
            let rendered: Vec<String> = parts.iter().map(|p| operand(module, names, fold, *p)).collect();
            format!("{{{}}}", rendered.join(", "))
        }

        Op::Repeat { arg, times } => format!("{{{}{{{}}}}}", times, operand(module, names, fold, *arg)),

        // Written as a concatenation rather than a cast. `(AW+1)'(x)` is
        // recorded at k2g_cdc_fifo.sv:87 as triggering the empty-log failure,
        // and the fix there was `{{AW{1'b0}}, x}` -- the same shape as this.
        Op::ZExt { arg, to } => {
            let from = module.value(*arg).ty.bit_width();
            if from >= *to {
                operand(module, names, fold, *arg)
            } else {
                format!("{{{{{}{{1'b0}}}}, {}}}", to - from, operand(module, names, fold, *arg))
            }
        }

        Op::SExt { arg, to } => {
            let from = module.value(*arg).ty.bit_width();
            if from >= *to {
                operand(module, names, fold, *arg)
            } else {
                // Replicate the sign bit. Also a concatenation, for the same
                // reason as ZExt.
                format!(
                    "{{{{{}{{{}[{}]}}}}, {}}}",
                    to - from,
                    operand(module, names, fold, *arg),
                    from - 1,
                    operand(module, names, fold, *arg)
                )
            }
        }

        Op::Trunc { arg, to } => {
            if *to == 1 {
                format!("{}[0]", operand(module, names, fold, *arg))
            } else {
                format!("{}[{}:0]", operand(module, names, fold, *arg), to - 1)
            }
        }

        // Reinterpretation only: the destination wire's declaration carries
        // the signedness, so nothing is needed in the expression.
        Op::Cast { arg } => operand(module, names, fold, *arg),

        Op::Mux { cond, then_val, else_val } => format!(
            "{} ? {} : {}",
            operand(module, names, fold, *cond),
            operand(module, names, fold, *then_val),
            operand(module, names, fold, *else_val)
        ),
    }
}

fn render_const(value: u128, ty: &Ty) -> String {
    let w = ty.bit_width();
    if w == 1 {
        return format!("1'b{}", value & 1);
    }
    // Masked to the declared width so a negative-looking constant prints as
    // the bit pattern it actually is.
    let masked = if w >= 128 { value } else { value & ((1u128 << w) - 1) };
    if masked < 10 {
        format!("{}'d{}", w, masked)
    } else {
        format!("{}'h{:X}", w, masked)
    }
}

// ---- naming --------------------------------------------------------------

struct NameTable {
    names: Vec<String>,
}

/// A `Cast` only changes how the bits are interpreted. When the signedness is
/// unchanged too -- reading an enum field out of a struct, say -- the Verilog
/// is identical to its operand, so the value is an alias rather than a wire.
fn is_pure_rename(module: &Module, def: &ValueDef) -> bool {
    match &def.op {
        Op::Cast { arg } => {
            let src = module.value(*arg);
            src.ty.is_signed() == def.ty.is_signed()
                && src.ty.bit_width() == def.ty.bit_width()
        }
        _ => false,
    }
}

impl NameTable {
    fn build(module: &Module) -> NameTable {
        let mut used: Vec<String> = module.ports.iter().map(|p| sanitize(&p.name)).collect();
        let mut names: Vec<String> = Vec::with_capacity(module.values.len());

        for def in &module.values {
            // An input port's value IS the port; do not invent a second name.
            if let Op::Port(id) = def.op {
                names.push(sanitize(&module.port(id).name));
                continue;
            }
            // Likewise a register: the held value is the register signal.
            if let Op::RegRead(ix) = def.op {
                names.push(sanitize(&module.regs[ix as usize].name));
                continue;
            }
            let base = match &def.name {
                Some(n) => sanitize(n),
                None => format!("n{}", def.id.0),
            };
            let mut candidate = base.clone();
            let mut suffix = 1;
            while used.contains(&candidate) {
                candidate = format!("{}_{}", base, suffix);
                suffix += 1;
            }
            used.push(candidate.clone());
            names.push(candidate);
        }
        NameTable { names }
    }

    fn get(&self, id: ValueId) -> String {
        self.names[id.0 as usize].clone()
    }
}

/// Verilog-2005 reserved words. A DDL name that collides gets a trailing
/// underscore rather than producing a file that will not compile.
const RESERVED: &[&str] = &[
    "always", "and", "assign", "automatic", "begin", "buf", "case", "casex", "casez", "cell",
    "cmos", "config", "deassign", "default", "defparam", "design", "disable", "edge", "else",
    "end", "endcase", "endconfig", "endfunction", "endgenerate", "endmodule", "endprimitive",
    "endspecify", "endtable", "endtask", "event", "for", "force", "forever", "fork", "function",
    "generate", "genvar", "highz0", "highz1", "if", "ifnone", "initial", "inout", "input",
    "integer", "join", "large", "localparam", "macromodule", "medium", "module", "nand",
    "negedge", "nmos", "nor", "noshowcancelled", "not", "notif0", "notif1", "or", "output",
    "parameter", "pmos", "posedge", "primitive", "pull0", "pull1", "pulldown", "pullup", "real",
    "realtime", "reg", "release", "repeat", "rnmos", "rpmos", "rtran", "rtranif0", "rtranif1",
    "scalared", "signed", "small", "specify", "specparam", "strong0", "strong1", "supply0",
    "supply1", "table", "task", "time", "tran", "tranif0", "tranif1", "tri", "tri0", "tri1",
    "triand", "trior", "trireg", "unsigned", "vectored", "wait", "wand", "weak0", "weak1",
    "while", "wire", "wor", "xnor", "xor",
];

fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (ix, ch) in name.chars().enumerate() {
        let ok = ch.is_ascii_alphanumeric() || ch == '_';
        let ok = ok && !(ix == 0 && ch.is_ascii_digit());
        out.push(if ok { ch } else { '_' });
    }
    if out.is_empty() {
        out.push('_');
    }
    if RESERVED.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_words_are_escaped() {
        assert_eq!(sanitize("output"), "output_");
        assert_eq!(sanitize("value"), "value");
        assert_eq!(sanitize("2bad"), "_bad");
    }

    #[test]
    fn constants_render_with_their_width() {
        assert_eq!(render_const(1, &Ty::UInt(1)), "1'b1");
        assert_eq!(render_const(5, &Ty::UInt(32)), "32'd5");
        assert_eq!(render_const(255, &Ty::UInt(32)), "32'hFF");
        // Masked to the declared width rather than printed as a huge number.
        assert_eq!(render_const(u128::MAX, &Ty::UInt(8)), "8'hFF");
    }

    #[test]
    fn single_bit_types_have_no_range() {
        assert_eq!(range_of(&Ty::UInt(1)), "");
        assert_eq!(range_of(&Ty::UInt(32)), "[31:0]");
        assert_eq!(signed_and_range(&Ty::SInt(32)), "signed [31:0] ");
        assert_eq!(signed_and_range(&Ty::SInt(1)), "signed ");
        assert_eq!(signed_and_range(&Ty::UInt(1)), "");
    }
}
