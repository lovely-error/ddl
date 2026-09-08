# DDL Finite State Machine Synthesis & Scoping

This document details how the DDL compiler lowers imperative `process` declarations into synthesizable, cycle-accurate Finite State Machines (FSMs).

---

## Table of Contents

- [Overview](#overview)
- [Barrier-Driven State Partitioning](#barrier-driven-state-partitioning)
- [Control-Flow Graph (CFG) vs. Linear FSMs](#control-flow-graph-cfg-vs-linear-fsms)
- [Zero-Cycle Branch Dispatch](#zero-cycle-branch-dispatch)
  - [Pre-Barrier (`stmts`) vs. Post-Barrier (`post`) Execution](#pre-barrier-stmts-vs-post-barrier-post-execution)
- [Lexical Variable Lifetimes & Register Allocation](#lexical-variable-lifetimes--register-allocation)
  - [Top-Level Process Registers](#top-level-process-registers)
  - [Scoped Locals Across Wait States](#scoped-locals-across-wait-states)
  - [Shadowing and Re-Entry](#shadowing-and-re-entry)
- [Resource & Port Sharing Across States](#resource--port-sharing-across-states)
  - [Memory Port Sharing](#memory-port-sharing)
  - [Mutually Exclusive Channel Operations](#mutually-exclusive-channel-operations)
- [Verilog Emission Architecture](#verilog-emission-architecture)

---

## Overview

In DDL, a `process` expresses sequential, imperative algorithms that interact with channels and memories over multiple clock cycles:

```ddl
process reg_port (cmd: buffer in u8, din: buffer in u32, dout: buffer out u32)
  var cell: u32 = @zeroed()
  loop
    let c = @rcv(cmd)
    if c[0] then
      let d = @rcv(din)
      cell = d
    else
      @send(dout, cell)
```

The compiler (`src/ir_fsm.rs`) converts this sequential code into a hardware state machine where:
- State transitions occur only at designated **wait points** (blocking channel operations or synchronous memory reads).
- Combinational operations between wait points are grouped into the executing state.
- Control flow (`if`/`else`, `match`, `loop`, `break`) maps directly to state graph edges.
- Register allocation preserves variable state across multi-cycle waits while respecting lexical lifetimes.

---

## Barrier-Driven State Partitioning

An ordinary combinational block executes in 0 cycles, while a clocked pipeline executes across fixed stages. A `process`, by contrast, contains operations that may stall for an unpredictable duration.

The compiler identifies **barriers** that require hardware wait states:
1. **Blocking Channel Operations**: `@rcv` and `@send` on `buffer` interfaces.
2. **Synchronous Memory Reads**: Reads from `#[impl(bram)]` arrays that take 1 clock cycle to return data.
3. **Loop Boundaries and Explicit Continuations**: Points where control flow loops back or branches to a shared continuation.

Everything between two barriers executes as single-cycle combinational logic within a single state. The barrier itself forms the condition that advances the FSM to its next state.

---

## Control-Flow Graph (CFG) vs. Linear FSMs

Unlike simple compilers that assign sequential integer states ($S_0 \rightarrow S_1 \rightarrow S_2$), DDL constructs an explicit **Control-Flow Graph (CFG)**:

- **Conditional Branches (`if`/`else`)**: Fork the current state into distinct target states.
- **Pattern Matching (`match`)**: Generates multi-way branch edges conditioned on the enum tag.
- **Loops (`loop`)**: Create backward edges in the CFG.
- **Loop Halting (`break`)**: Jumps to the post-loop continuation state, or halts the process entirely if at the top level.

```
                  [ State 0: Wait for cmd ]
                             |
                   (cmd[0] == 1 ? / else)
                    /                  \
   [ State 1: Wait for din ]     [ State 2: Send dout ]
          |                               |
          \-------------------------------/
                             |
                    (Loop back to State 0)
```

Because the state transition is a graph edge rather than an incrementing counter (`state <= state + 1`), dead states are impossible and transitions execute with zero bubble cycles.

---

## Zero-Cycle Branch Dispatch

A major performance problem in traditional high-level synthesis (HLS) tools is the **dispatch bubble**:
> *Cycle 0: Receive command $\rightarrow$ Cycle 1: Decode command $\rightarrow$ Cycle 2: Branch to target handler.*

This wastes clock cycles on control overhead.

### Pre-Barrier (`stmts`) vs. Post-Barrier (`post`) Execution

DDL eliminates dispatch bubbles by dividing each barrier state into two statement lists:
- **`stmts` (Pre-Barrier)**: Executed before the wait condition is satisfied (e.g. preparing address wires or evaluating branch guards).
- **`post` (Post-Barrier)**: Executed in the **same cycle** that the barrier fires, with the received value immediately in scope.

Consider this command loop:

```ddl
let c = @rcv(cmd)
if c[0] then
  ...
```

When `@rcv(cmd)` completes on the rising clock edge, the received byte `c` is already available on the input bus. DDL schedules the condition `c[0]` in the `post` block of State 0. The next state register `state_q` is loaded directly with State 1 or State 2 on that same edge:

```verilog
// Next-state logic evaluated in the cycle the command arrives:
always @(*) begin
  case (state_q)
    STATE_0: begin
      if (cmd_ready_to_take) begin
        state_d = cmd_item[0] ? STATE_1 : STATE_2;
      end else begin
        state_d = STATE_0; // Wait
      end
    end
    ...
  endcase
end
```

**Result**: Zero idle cycles between command arrival and branch target execution.

---

## Lexical Variable Lifetimes & Register Allocation

DDL distinguishes between persistent top-level registers and scoped local variables.

### Top-Level Process Registers

Variables declared at the root level of a `process` body before any loop or wait statements are **persistent registers**:

```ddl
process counter (...)
  var count: u32 = 32'd0   -- Persistent register
  loop
    ...
```

- Initialized to their declared initial value on global reset (`rst_n == 0`).
- Retain their value across all states and loop iterations.

### Scoped Locals Across Wait States

Variables declared inside loops, branches, or match arms have **lexical scope**:

```ddl
process count (src: buffer in u8, dst: buffer out u8)
  loop
    var remaining: u8 = @rcv(src)  -- Re-initialized on every loop iteration
    loop
      @send(dst, remaining)
      remaining -= 8'd1
      if remaining == 8'd0 then
        break
```

1. **Re-Initialization**: Every time execution enters the declaring scope, the variable is re-initialized (in this case, taking the newly received payload).
2. **State Spanning**: If a local variable is live across a wait state (such as `@send(dst, remaining)`), the compiler automatically allocates a holding register (`LocalReg`).
3. **Dead-Variable Reclamation**: Once execution leaves the variable's lexical block, its holding register cannot be read, and the compiler may reuse storage.

### Shadowing and Re-Entry

If an inner scope shadows an outer variable name:
- The inner binding is isolated to its containing CFG states.
- Exiting the inner scope automatically restores the outer binding.
- Sibling match arms can declare variables with identical names and different types without storage or symbol collision.

---

## Resource & Port Sharing Across States

### Memory Port Sharing

Block RAMs have a fixed number of physical write ports (typically 1 or 2). In naive Verilog, writing to the same memory from two different places in code generates a multi-driven net error:

```
Error: Net 'mem' has multiple drivers!
```

DDL automatically multiplexes access across states:
- If State 1 writes `mem[a] = d1` and State 3 writes `mem[b] = d2`, the compiler infers a single physical write port.
- Address and data lines are multiplexed using the current state register:
  ```verilog
  wire mem_we = (state_q == STATE_1) | (state_q == STATE_3);
  wire [ADDR_W-1:0] mem_addr = (state_q == STATE_1) ? a_reg : b_reg;
  wire [DATA_W-1:0] mem_din  = (state_q == STATE_1) ? d1_reg : d2_reg;
  ```
- Mutually exclusive branches of an `if` or arms of a `match` share write ports in the same manner.

### Mutually Exclusive Channel Operations

When multiple arms of a `match` each send a different message to the same output channel, the compiler verifies mutual exclusivity and routes the selected arm's payload to the output bus, asserting `@send` only for the executing state.

Attempts to execute multiple non-blocking sends to the same channel in the same cycle are caught and diagnosed as compile errors.

---

## Verilog Emission Architecture

Generated FSMs follow standard two-process synchronous Verilog architecture:

1. **Sequential State Register (`always @(posedge clk)`)**:
   - Updates `state_q <= state_d` under reset and clock control.
   - Updates local variable registers and channel salt pointers.
2. **Combinational Transition Logic (`always @(*)`)**:
   - Decodes `state_q`.
   - Computes barrier readiness (e.g. `!src_empty`, `!dst_full`).
   - Evaluates branch conditions.
   - Determines next state `state_d` and control enables.
