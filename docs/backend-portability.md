# Verilog-2005 Backend & Synthesizer Portability

This document describes the design principles of DDL's code generator (`src/verilog.rs`), specifically how it achieves robust portability across restrictive FPGA synthesis engines like **GowinSynthesis**.

---

## Table of Contents

- [The Fragility of FPGA Synthesis Engines](#the-fragility-of-fpga-synthesis-engines)
- [Target: Strict Verilog-2005](#target-strict-verilog-2005)
- [Forbidden Constructs & Workarounds](#forbidden-constructs--workarounds)
  - [1. No `$clog2`](#1-no-clog2)
  - [2. No Width Casts in Expressions](#2-no-width-casts-in-expressions)
  - [3. No Verilog Function Calls](#3-no-verilog-function-calls)
- [Code Generation Conventions](#code-generation-conventions)
  - [Explicit Bitwidth Typing on All Literals](#explicit-bitwidth-typing-on-all-literals)
  - [Reproducibility Banners & `--check`](#reproducibility-banners---check)
- [Automated Verification with Verilator](#automated-verification-with-verilator)
  - [Verilator as an AST/Netlist Oracle](#verilator-as-an-astnetlist-oracle)
  - [Checks Enforced at `-Wall`](#checks-enforced-at--wall)

---

## The Fragility of FPGA Synthesis Engines

While commercial ASIC synthesis tools (e.g. Synopsys Design Compiler) and high-end FPGA suites (Vivado, Quartus Prime Pro) support modern SystemVerilog standards, tools for low-cost and open-source FPGAs often have incomplete, bug-ridden parsers.

The primary hardware target for DDL's reference hardware is the **Gowin GW1NR-9C** (Tang Nano 9K). During development, the compiler authors discovered that **GowinSynthesis routinely crashes or exits with a completely empty log file** when encountering standard Verilog and SystemVerilog constructs.

To guarantee that any valid DDL program compiles and synthesizes on all vendor engines without modification, DDL generates a strictly constrained subset of **Verilog-2005**.

---

## Target: Strict Verilog-2005

DDL's backend produces clean, standard-compliant Verilog-2005:
- All module declarations use standard ANSI C-style port lists.
- Control logic is emitted as standard `always @(posedge clk)` synchronous blocks and `always @(*)` combinational blocks.
- No SystemVerilog-specific keywords (`always_ff`, `always_comb`, `logic`, `interface`) appear in generated `.v` files.
- Structures and arrays are flattened into standard packed 1D vectors (`reg [W-1:0]`).

---

## Forbidden Constructs & Workarounds

DDL enforces three strict prohibitions in its backend to prevent synthesis tool crashes:

### 1. No `$clog2`

- **The Problem**: In GowinSynthesis, passing constant expressions or parameter calculations to the standard system function `$clog2(...)` frequently causes the parser to fault and terminate compilation without an error message.
- **DDL Workaround**: The DDL compiler performs all log2, bitwidth, and address calculations **ahead of time in Rust during AST lowering**. Emitted Verilog contains exclusively pre-evaluated integer constants:
  ```verilog
  // FORBIDDEN:
  reg [$clog2(256)-1:0] addr;

  // EMITTED BY DDL:
  reg [7:0] addr;
  ```

### 2. No Width Casts in Expressions

- **The Problem**: Verilog-2001 and SystemVerilog width casts (e.g., `16'(x + y)`) and complex replicated concatenations inside expressions trigger internal AST assertion failures in GowinSynthesis.
- **DDL Workaround**:
  - All type conversions (`@zext`, `@sext`, `@trunc`) are expanded by the compiler into explicit Verilog bit-slices and zero/sign replications.
  - Zero-extension is emitted using explicit zero padding:
    ```verilog
    wire [31:0] wide = {{16{1'b0}}, doubled_s1};
    ```
  - Truncation is emitted using explicit bit slicing:
    ```verilog
    wire [7:0] narrow = full_val[7:0];
    ```

### 3. No Verilog Function Calls

- **The Problem**: Inlined Verilog functions (`function ... endfunction`) often produce synthesis warnings regarding incomplete sensitivity lists or fail to infer registers properly.
- **DDL Workaround**:
  - DDL user functions (`fun`) are completely **inlined at call sites** during SSA lowering.
  - No Verilog functions are ever emitted in the output netlist; all combinational logic is represented directly as continuous `assign` statements or `always @(*)` blocks.

---

## Code Generation Conventions

### Explicit Bitwidth Typing on All Literals

Unsized integer literals in Verilog (e.g. `0` or `42`) default to a 32-bit signed integer. If used inside comparisons or arithmetic, they can silently widen expressions, leading to unintended sign extension or extra logic gates.

DDL emits **every single literal with an explicit width and base**:
- `1'b0` or `1'b1` for booleans and enables.
- `2'd0`, `2'd1`, `2'd2` for salt pointers.
- `32'd0`, `16'hFFFF` for typed integers.

### Reproducibility Banners & `--check`

Every emitted Verilog file begins with a standardized banner:

```verilog
// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/mul3.ddl -o examples/mul3.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.
```

The `--check` flag allows CI pipelines to verify that checked-in Verilog files remain identical to what the compiler produces, preventing drift between compiler updates and generated hardware:

```bash
ddl build examples/mul3.ddl -o examples/mul3.v --check
```

---

## Automated Verification with Verilator

### Verilator as an AST/Netlist Oracle

Text-based tests (e.g. checking whether a string appears in output) cannot detect subtle electrical bugs like floating nets or latch inference.

In DDL's test suite ([`tests/lint.rs`](file:///e:/Code/ddl/tests/lint.rs)), every generated Verilog example is compiled through [Verilator](https://www.veripool.org/verilator/) with strict linting enabled (`-Wall`).

### Checks Enforced at `-Wall`

Verilator parses the output into a complete circuit netlist and validates:

1. **No Inferred Latches (`COMBDLY` / `LATCH`)**: Verifies that every `if` and `case` statement assigns all output signals across all execution branches.
2. **No Multi-Driven Nets (`MULTIDRIVE`)**: Verifies that no wire or register has more than one concurrent driver.
3. **No Combinational Loops (`UNOPTFLAT` / `CIRCULAR`)**: Verifies that no signal loops back on itself within the same clock cycle.
4. **No Width Mismatches (`WIDTH`)**: Verifies that all port connections, assignments, and operands match bit-for-bit without truncation.
5. **Clean Clocking**: Verifies that registers are clocked on single edges without mixed-edge anomalies.

A generated `.v` file is considered valid only when Verilator compiles it with zero warnings.
