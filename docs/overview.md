# DDL Language Overview & Reference Manual

This document provides a comprehensive technical reference for the Dataflow Description Language (DDL): its design principles, core declarations, communication channels, type system, storage primitives, scoping rules, and compile-time guarantees.

---

## Table of Contents

- [Design Philosophy](#design-philosophy)
- [Core Declarations](#core-declarations)
  - [Pipelines (`sequence`)](#pipelines-sequence)
  - [State Machines (`process`)](#state-machines-process)
  - [Combinational Functions (`fun`)](#combinational-functions-fun)
  - [Structural Composition (`graph`)](#structural-composition-graph)
  - [External Modules (`extern`)](#external-modules-extern)
  - [Bare Signals (`wire`)](#bare-signals-wire)
- [Channels and Interconnect](#channels-and-interconnect)
  - [Backpressured Buffers (`buffer`)](#backpressured-buffers-buffer)
  - [The Internal Salt Protocol](#the-internal-salt-protocol)
  - [Show-Ahead FIFO Boundaries](#show-ahead-fifo-boundaries)
  - [Clock Domain Crossings (CDC)](#clock-domain-crossings-cdc)
  - [Channel Operations & Intrinsics](#channel-operations--intrinsics)
  - [Hardware Combinators (`@merge` and `@split`)](#hardware-combinators-merge-and-split)
- [Type System and Storage](#type-system-and-storage)
  - [Integers and Bit Operations](#integers-and-bit-operations)
  - [Structs and Tagged Unions](#structs-and-tagged-unions)
  - [Packed Arrays](#packed-arrays)
  - [Memories (`lutram` and `bram`)](#memories-lutram-and-bram)
  - [Multi-Port Synthesis and LVT BRAM](#multi-port-synthesis-and-lvt-bram)
- [Statements and Scoping](#statements-and-scoping)
  - [Bindings and Variable Scoping](#bindings-and-variable-scoping)
  - [Control Flow and Pattern Matching](#control-flow-and-pattern-matching)
- [Simulation Assertions](#simulation-assertions)
- [Compile-Time Diagnostics](#compile-time-diagnostics)
- [Verilog Backend & Tooling](#verilog-backend--tooling)
  - [Synthesizer Portability](#synthesizer-portability)
  - [Multi-File Projects (`import`)](#multi-file-projects-import)
  - [Formatter and Tooling](#formatter-and-tooling)

---

## Design Philosophy

In conventional Verilog and SystemVerilog, designers must manually implement handshakes, FIFOs, `valid`/`ready` signals, and intermediate pipeline registers. Every manual handshake creates potential combinational paths through `ready`, risking deadlocks and timing failures that are difficult to catch prior to physical synthesis or silicon bring-up.

DDL automates protocol and register management:
- **Focus on data transformation**: The designer describes operations on data streams; the compiler generates the control logic, pipeline stages, validity signals, and skid registers.
- **Isolated state**: There is no global or shared memory. Declarations encapsulate their own state, and all inter-block communication takes place over point-to-point pipes.
- **Cycle-decoupled handshakes**: Channels eliminate combinational `ready` loops by construction through registered, gray-coded pointer exchange.
- **Strict, Portable Verilog-2005**: All output avoids brittle constructs like `$clog2`, dynamic width casts, and Verilog functions that crash low-cost FPGA toolchains like GowinSynthesis.

---

## Core Declarations

DDL provides six primary top-level declarations:

| Declaration | Hardware Equivalent | Description |
|---|---|---|
| `sequence` | Pipeline | Pipelined datapath partitioned into clock stages by cut operators (`\|\|\|`). |
| `process` | Finite State Machine | Sequential control flow with blocking channel waits. |
| `fun` | Combinational Module | Pure combinational logic; inlined at call sites or emitted as a standalone module. |
| `graph` | Structural Netlist | Instantiates blocks and wires their communication channels together. |
| `extern` | Module Interface | Declares third-party or hand-written Verilog modules for integration in a `graph`. |
| `wire` | Bare Signal | An unhandshaked signal on an `extern` or a `graph`: a pin or a sideband. |

### Pipelines (`sequence`)

A `sequence` represents a feed-forward pipeline. Cut markers (`|||`) separate stages executed across clock cycles:

```ddl
sequence mul3 (src: buffer in u16, dst: buffer out u32)
  let a = @rcv(src)
  let doubled: u16 = a + a
  |||
  let wide: u32 = @zext(doubled, 32)
  |||
  let scaled: u32 = wide + wide
  @send(dst, scaled)
```

The compiler translates this into a three-stage pipeline (see [`examples/mul3.v`](../examples/mul3.v)):
- Each stage includes an automatically managed validity bit and its own shift enable: a stage advances when its sinks have room and the stage below it is empty or advancing, so a stall closes up bubbles instead of freezing the whole pipeline.
- A `sequence` receives once from each blocking input in the first stage. It sends to each output from one stage, at most once per item, under any `if` or `match` -- an output sent from stage $k$ leaves $k$ cycles after its item entered. With several inputs the head is a join: an item enters only when every blocking input is offering. A stage shifts only when every output it `@send`s this item to has room; a `@try_send` never holds it, pushes only into room, and answers whether it did. A stage held by a full sink never holds the stages after it.
- `@try_rcv`, `@peek` and `@drop` may be used in any stage, under any condition. None of them waits: an operation in stage $k$ acts for the item stage $k$ holds, looks on every cycle, and takes an entry only on a cycle that stage is live and moving -- never for a bubble, and never twice for a held item. Every operation on one `in` pipe must sit in one stage, and a pipe that is only ever peeked at is refused, since it would never drain. `@try_send` is not permitted in pipelines; use `@send`, or a `process`.
- A loop of pipes whose sequences can never fire can never carry its first item, and the compiler reports it. A head with blocking `@rcv`s needs all of them; a head with none needs any pipe its first stage reads. An accumulator that blocks on an outside input and samples its feedback with `@try_rcv` is live; one whose head reads only the feedback is not.

For details on shift-register generation, BRAM stage alignment, and same-cycle hazard forwarding, see [**Pipeline Lowering & Memory Forwarding**](pipeline-lowering.md).

### State Machines (`process`)

A `process` models a sequential state machine. Blocking operations on channels delineate state transitions:

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

The compiler synthesizes this into an FSM (see [`examples/reg_port.v`](../examples/reg_port.v)) where branching decisions take effect in the cycle the triggering command is received:
- A `loop` executes repeatedly across cycles.
- A process body without an enclosing `loop` executes its sequential statements once and halts.

For details on CFG state construction, zero-cycle branch dispatch, and lexical register allocation across waits, see [**Finite State Machine Synthesis & Scoping**](fsm-synthesis.md).

### Combinational Functions (`fun`)

A `fun` computes pure combinational logic:
- Function calls within other declarations are inlined during lowering.
- When compiled as an entry point, a `fun` becomes a Verilog module with return values and `out` parameters mapped to output ports.

### Structural Composition (`graph`)

A `graph` instantiates functions, sequences, processes, and externs, binding their input and output channels:

```ddl
graph top (p: buffer in u32, q: buffer in u32, o1: buffer out u32, o2: buffer out u32)
  let m: buffer u32
  let d: buffer u32
  @merge(p, q, m)      -- Two producers arbitrated onto one channel
  dbl(m, d)
  @split(d, o1, o2)    -- One producer broadcast to two independent consumers
```

### External Modules (`extern`)

The `extern` keyword declares the interface of an external or hand-written Verilog module so that it can be wired inside a `graph`:

```ddl
extern psram (req: buffer in mem_req_t, rsp: buffer out mem_rsp_t)
```

Connections to an `extern` are type-checked during compilation, but no module body is generated: the Verilog is yours, and the compiler links against it by name.

Each pipe reaches the hand-written module as an ordinary Show-Ahead FIFO interface, never as the internal salt protocol. For `psram` above:

```verilog
module psram (
  input         clk,
  input         rst_n,
  output        req_can_receive,     // I have room
  input         req_receive_en,      // write req_data_write_in now
  input  [31:0] req_data_write_in,
  output        rsp_has_data,        // I have an item
  input         rsp_drop_item,       // I took it; advance
  output [31:0] rsp_data_read_out
);
```

The compiler puts an adapter on its own side of the boundary to translate, so nothing you write has to reconstruct the salt protocol. See [Show-Ahead FIFO Boundaries](#show-ahead-fifo-boundaries).

### Bare Signals (`wire`)

A `wire` is a signal of the declared width with no handshake at all: a pin, a clock-domain sideband, or a status flag from a piece of vendor IP. Because there is nothing on it to wait for, it is legal only where nothing waits -- an `extern` or a `graph`:

```ddl
extern pll (locked: wire out u1, cfg: buffer in u32)

graph board (cfg: buffer in u32, lock_led: wire out u1)
  pll(lock_led, cfg)
```

- `x: wire in T` emits one input `x` of `T`'s width; `y: wire out T` emits one output.
- A graph routes a wire straight through to its own boundary, and checks that exactly one instance drives each output.
- A `process` or `sequence` cannot take one. Their parameters are pipes and constant parameters, because a body that could present a raw wire could hand-roll an uncontrolled protocol over it, and the protocol is the compiler's job.

---

## Channels and Interconnect

Everything a DDL module computes with travels through a **`buffer`**: a backpressured, lossless, point-to-point channel. A **`wire`** carries a bare signal with no handshake, and is allowed only on an `extern` or a `graph` -- the two declarations that sit at the edge of the program and compute nothing.

### Backpressured Buffers (`buffer`)

A `buffer` provides flow-controlled point-to-point communication:
- **Depth**: Every buffer has a fixed depth of 2 entries (a primary register and a skid register).
- **Backpressure**: Producers stall automatically when the buffer is full; consumers stall when it is empty.
- **Lossless Guarantee**: Data is never silently dropped or overwritten. A transfer occurs only when both producer and consumer agree.

### The Internal Salt Protocol

Between two modules the compiler generates, each `buffer` flattens into three Verilog ports:
- `<p>_wsalt`: 2-bit write pointer emitted by the producer.
- `<p>_rsalt`: 2-bit read pointer emitted by the consumer.
- `<p>_data`: Data bus carrying both FIFO slots (`{slot1, slot0}`).

Both pointers are registered and gray-coded:

```verilog
wire src_empty = (src_wsalt == src_rsalt_q);     // Pointers match: buffer is empty
wire dst_full  = (dst_wsalt_q == (~dst_rsalt));  // Differ in both bits: buffer is full
```

Advancing a pointer *is* the transfer. Because both pointers originate from registers, there are no combinational paths linking consumer readiness to producer validity. This eliminates two classic hardware handshake pitfalls by construction:
1. **Combinational timing loops**: In traditional `valid`/`ready` interfaces, downstream readiness is often combinationally coupled to upstream validity. Chaining blocks can inadvertently close a zero-delay combinational loop through `ready`.
2. **Mutual-wait protocol deadlocks**: In standard handshakes, protocol bugs can arise where a producer waits for `ready` before asserting `valid`, while the consumer waits for `valid` before asserting `ready`. With DDL's salt protocol, transfer decisions are determined strictly from registered pointers, and the 2-entry buffer capacity absorbs the 1-cycle latency of registered pointer updates.

For a complete architectural breakdown, including cycle-by-cycle waveform diagrams and skid buffer analysis, see [**The Gray-Code Salt Protocol**](salt-protocol.md).

### Show-Ahead FIFO Boundaries

The salt protocol above is strictly internal. It never crosses a boundary a person writes Verilog against. An `extern`, and any module chosen as an **export target**, presents a **Show-Ahead FIFO (zero read latency)** instead:

| `p: buffer in T` (the module consumes) | dir | meaning |
| --- | --- | --- |
| `p_can_receive` | output | there is room (`!full`) |
| `p_receive_en` | input | write `p_data_write_in` this cycle (`wr_en`) |
| `p_data_write_in [W-1:0]` | input | the item (`wr_data`) |

| `p: buffer out T` (the module produces) | dir | meaning |
| --- | --- | --- |
| `p_has_data` | output | an item is available at head (`!empty`) |
| `p_drop_item` | input | acknowledge receipt and advance (`rd_en`) |
| `p_data_read_out [W-1:0]` | output | the item (`rd_data`) |

In this interface:
- While `p_has_data` is high, `p_data_read_out` is **already valid and stable** on that exact cycle without requiring an advance read-enable pulse (zero read latency).
- Pulsing `p_drop_item = 1` acknowledges and retires the current item; on the next clock edge, the subsequent item is presented.
- `receive_en` and `drop_item` are honoured only while their respective flag is high; adapters internally gate them to prevent pointer corruption.
- The translation is performed by compiler-synthesized adapter modules (`ddl_wport_to_salt_<W>`, `ddl_salt_to_rport_<W>`, etc.) from `src/ir_adapt.rs`.

```
+-----------------------------------------------------------------------------------+
| Top-Level Wrapper: mul3 (src/ir_export.rs)                                        |
|                                                                                   |
|                   [ddl_wport_to_salt_16] (Adapter)                                |
|                   +-------------------------------+                               |
| src_can_receive <-| can_receive                   |                               |
| src_receive_en  ->| receive_en            o_wsalt |---(src_wsalt)---+             |
| src_data_write  ->| data_write_in         o_rsalt |<--(src_rsalt)---|             |
|                   |                        o_data |===(src_data)====|             |
|                   +-------------------------------+                 |             |
|                                                                     v             |
|                                                           +-------------------+   |
|                                                           | mul3_core         |   |
|                                                           | (Internal Salt)   |   |
|                                                           +-------------------+   |
|                                                                     |             |
|                   [ddl_salt_to_rport_32] (Adapter)                  |             |
|                   +-------------------------------+                 |             |
|                   |                       i_wsalt |<--(dst_wsalt)---|             |
| dst_has_data    <-| has_data              i_rsalt |---(dst_rsalt)---+             |
| dst_drop_item   ->| drop_item              i_data |===(dst_data)====+             |
| dst_data_read   <-| data_read_out                 |                               |
|                   +-------------------------------+                               |
+-----------------------------------------------------------------------------------+
```

#### Choosing the Export Target

An **export target** is the module the build is for. It is discovered from the use graph: a **root** is a module that nothing else instantiates or calls, among the declarations in files named on the command line. An `import` supplies a place to look for dependencies, not deliverables, so its declarations are never root candidates.

- **One root**: automatically chosen as the target; no flags required.
- **Multiple roots**: the compiler issues an error and asks you to disambiguate:
  ```bash
  ddl build src.ddl -o src.v --export top          # wraps with Show-Ahead FIFOs
  ddl build src.ddl -o src.v --bare-export top     # keeps raw internal salt ports
  ddl build src.ddl -o src.v --export a,b,c        # several, comma-separated
  ```

For full details, see [**FIFO Boundary Adapters & Export Architecture**](fifo-boundaries-and-export.md).

### Clock Domain Crossings (CDC)

Every port on every module DDL generates is synchronous to `clk`. If an external module or IP core runs on a different clock, wiring it directly will cause silent data corruption (measured at 100% item corruption on hardware).

To cross clock domains safely:
- **Compiler-Automated**: Pass `--async-export <pipe>[=<domain>]` or `--async-extern <instance>.<pipe>[=<domain>]`. The compiler automatically synthesizes an asynchronous Gray-pointer FIFO (`ddl_cdc_fifo`) and an active-handshake reset synchronizer (`ddl_rst_cross`).
- **Hand-Wired**: Instantiate [`lib/ddl_cdc_fifo.v`](../lib/ddl_cdc_fifo.v) or vendor CDC primitives (`XPM_CDC_FIFO`, `DCFIFO`) at the top-level boundary.

For architecture details, empirical failure measurements, and the hand-wired route, see [**Clock Domains**](clock-domains.md). For practical recipes, port listings, and flags, see [**Guide 6: Clock Domain Crossings**](guides/6-clock-domain-crossings.md).

### Channel Operations & Intrinsics

A `buffer` is reached through built-in channel intrinsics:

| Operation | Behavior |
|---|---|
| `@rcv(ch)` | Blocks until an item is available, then consumes it. |
| `@send(ch, val)` | Blocks until room is available, then pushes the value. |
| `@peek(ch)` | Returns `(data, present)`. Inspects the head item without consuming it. |
| `@try_rcv(ch)` | Non-blocking receive: returns `(data, ok)`. Consumes the item if available. |
| `@try_send(ch, val)` | Non-blocking send: returns a boolean indicating whether the push succeeded. |
| `@drop(ch)` | Consumes and discards the head item without reading its payload. |

#### Same-Cycle Relay Pattern

Because transfers occur on clock edges, non-blocking operations allow conditional inspection and forwarding within a single cycle. To forward from one buffer to another without risking data loss on backpressure, use `@peek` before `@drop`:

```ddl
process relay (src: buffer in u8, dst: buffer out u8)
  loop
    let (x, present) = @peek(src)
    if present then
      let sent = @try_send(dst, x)
      if sent then
        let took = @drop(src)
        @assert(took)
```

### Hardware Combinators (`@merge` and `@split`)

DDL provides built-in structural combinators for channel multiplexing and distribution:
- **`@merge(in0, in1, out)`**: Arbitrates two input buffers onto a single output channel using rotating round-robin priority. Prevents starvation without introducing multi-cycle FSM latency.
- **`@split(in, out0, out1)`**: Broadcasts a single input channel to multiple consumers. Each consumer receives its own independent 2-entry skid buffer. An item is retired from the input only when *all* consumers have accepted it.

Combinators are generated as dedicated datapath modules (e.g. `ddl_merge_2x32`) without internal state machine overhead. For details, see [**Zero-Latency Hardware Combinators**](combinators.md).

---

## Type System and Storage

### Integers and Bit Operations

- **Arbitrary-Width Types**: Unsigned `uN` and signed `iN` integers (e.g., `u1`, `u8`, `u32`, `i16`).
- **Strict Width Safety**: Mixing bitwidths or mixing signed and unsigned values in an operation causes a compile error. Automatic truncation or extension is prohibited; use `@zext(val, width)`, `@sext(val, width)`, or `@trunc(val, width)`.
- **Compile-time arithmetic has two domains**, and a literal's spelling picks which one:
  - *Unsized* literals (`4`, `0xFF`) are mathematical integers. `4 * 64` is `256`, and an expression that runs past 128 bits is an error rather than a wrap. Array lengths, loop bounds and widths are written in this domain.
  - *Sized* literals (`8'd255`) carry their width, and arithmetic on them wraps exactly as the hardware would: `8'd255 + 8'd1` is `0`, whether it is folded at compile time or computed at runtime.

  A constant expression means the same thing in every position that accepts one, so moving a subexpression into a `let` never changes what a program does.
- **Bit Slicing**: `@slice(val, base_index, width)` extracts a dynamic bit window of size `width` starting at `base_index`.
- **Concatenation**: `{a, b, c}` packs values from most-significant to least-significant bits into an unsigned integer whose width equals the sum of operand widths.

### Structs and Tagged Unions

- **Structs**: Product types with named fields. Memory layout matches SystemVerilog packed structure conventions, enabling direct interoperation with `.svh` headers.
- **Simple Enums**: Defined with an explicit backing width (e.g., `enum State: u2 { Idle, Run, Stop }`).
- **Tagged Unions**: Enums with variant payloads:
  ```ddl
  enum req_e
    Nop
    Read(addr_t)
    Write(u8)
    Halt
  ```
  Tagged unions pack into `{tag, payload}` format with the tag occupying the highest bits. The overall width is derived from the tag width plus the widest variant payload.
  - Payload fields can *only* be accessed via `match` expressions.
  - Direct equality checks (`==`) on tagged unions are disallowed to prevent comparing uninitialized payload padding.

### Packed Arrays

A type of the form `[T; n]` without a memory attribute is a packed value:
- Supported as struct fields, channel payloads, and parameters.
- Elements are accessed via `a[idx]` and sliced via `a[hi..lo]`.
- Array assignments support both constant and dynamic indexing: `a[idx] = val`.
- Out-of-bounds indexing is rejected at compile time when the index is constant.
- A computed index is bounds-checked in the generated hardware: an index outside the array reads as zero, and a write to one changes nothing. The check is emitted only when the index is wide enough to express an out-of-range value, so a `u2` index into `[T; 4]` costs no extra logic.

### Memories (`lutram` and `bram`)

Array storage can be backed by physical hardware memory primitives using implementation attributes:

```ddl
#[impl(lutram)]
var lut_table: [u16; 64]

#[impl(bram)]
var block_mem: [u32; 1024]
```

- **`lutram` (Asynchronous Read)**: Distributed RAM. Reads complete within the same clock cycle.
- **`bram` (Synchronous Read)**: Block RAM. Reads require one clock cycle latency and are emitted inside the memory's clocked block to ensure inference by synthesis tools.
  - In a `process`, a `bram` read occupies a dedicated FSM state.
  - In a `sequence`, a `bram` read must precede a stage cut (`|||`). The block RAM's built-in output register acts as the pipeline stage register.
  - **Forwarding**: Same-cycle writes occurring earlier in source order are forwarded around the memory array, which updates on the clock edge.

#### Writing part of an element

An element may be an aggregate — `#[impl(lutram)] [[u8; 4]; 16]`, or a memory of structs — and part of one can be assigned directly:

```ddl
t[addr][lane] = byte
t[addr].field = value
```

The write port always carries a whole element, so the parts not named come from a read of the same address: a read-modify-write, with the same forwarding as any other read, so two lanes of one row written in the same cycle both land.

This needs the read to be free, so it is available on `lutram` only. On `bram` and `bkram` the read costs a cycle and a single statement has nowhere to spend it — inside a `sequence` the value would not arrive until after the stage cut — so the compiler asks for the two steps to be written out, where the cycle is visible:

```ddl
var row = t[addr]
row[lane] = byte
t[addr] = row
```

Each such write claims a write port, exactly as a whole-element write does. Compound assignment (`+=`) to a memory is not supported, nested or not.

### Multi-Port Synthesis and LVT BRAM

The compiler infers one read/write port per concurrent access:
- Two sequential writes in the same cycle require two physical write ports.
- Mutually exclusive writes within `if`/`else` branches share a single write port.

When targeting FPGAs that lack native dual-write block RAM cells (such as the Gowin GW1NR-9C), pass the `--lvt-bram` flag:

```bash
ddl build src/top.ddl -o build/top.v --lvt-bram
```

This synthesizes a multi-write RAM using single-write BRAM blocks coordinated by a Live Value Table (LVT) implemented in distributed logic. See [`examples/rf_lvt.ddl`](../examples/rf_lvt.ddl) and [**Multi-Write BRAM via Live Value Tables**](lvt-bram-architecture.md).

---

## Statements and Scoping

### Bindings and Variable Scoping

- **Bindings**:
  - `let name: T = expr`: Immutable local binding.
  - `var name: T = expr`: Mutable binding.
- **Variable Scoping**:
  - Process-level `var` declarations declared at the root of a `process` persist as registers across cycles and are initialized on reset.
  - Local `var` declarations inside loops and conditional blocks have lexical scope. They re-initialize whenever execution enters their scope and retain state across wait cycles within that scope.
- **Assignments**: Standard (`=`) and compound (`+=`, `-=`, `<<=`, `>>=`, `&=`, `|=`, `^=`).

### Control Flow and Pattern Matching

- **Conditionals (`if` / `else`)**: In FSMs, conditions can fork execution across states. In combinational blocks, all branches must assign outputs to avoid latches.
- **Pattern Matching (`match`)**: Matches on enum variants. Exhaustiveness checking is strictly enforced. One arm may name several variants by joining them with `|`, and that list may wrap onto the next line, indented at the arm's own depth or deeper.
- **Loops**:
  - `loop { ... }`: Infinite loop in a `process`, modeling recurring state cycles.
  - `break`: Exits the innermost loop. At the top level of a process, `break` permanently halts execution.
  - `for i in 0..n { ... }`: Compile-time unrolled loop. Trip bounds must be statically determinable. Blocking channel operations and synchronous memory reads are disallowed within `for` loops.

---

## Simulation Assertions

DDL provides built-in simulation assertions:

```ddl
@assert(cond)
@assert(cond, "Condition violated")
@fatal("Unrecoverable state reached")
```

- **Simulation Guarded**: Assertions are emitted within `` `ifdef SIMULATION `` blocks and stripped during physical synthesis.
- **Execution Scoped**:
  - In a `process`, assertions evaluate only when their specific state and branch are active; idle cycles do not trigger spurious failures.
  - In a `sequence`, assertions evaluate only when the containing stage holds a valid token and the pipeline advances.
  - Assertions are automatically held in reset during hardware reset cycles.

---

## Compile-Time Diagnostics

The DDL compiler emphasizes strict validation, failing fast with descriptive diagnostics:
- **Incomplete assignments**: Omitting an output assignment on any branch of an `if` or `match` in combinational code (prevents unintended latches).
- **Non-exhaustive matches**: Failing to match every variant of an enum.
- **Width or signedness mismatches**: Mixing incompatible numeric types without explicit casts.
- **Graph topology errors**: Binding a pipe to multiple producers, or leaving a pipe unattached.
- **Invalid blocking operations**: Placing a blocking `@rcv` or `@send` anywhere but statement position — inside an `if` condition, a `match` scrutinee, an unrolled `for` loop, or another operation's operand list, as in `@send(dst, @rcv(src))`. A blocking transfer costs a cycle, and only a statement can be given one; bind it first with `let v = @rcv(src)` and send the name.
- **Unsupported memory usage**: Invoking a synchronous `bram` read within a stateless `fun`, or attaching an asynchronous reset to a BRAM primitive.
- **Dynamic indexing restrictions**: Placing computed indices at any position other than the terminal step of an assignment target.
- **Tagged union misuse**: Performing direct comparisons (`==`) on tagged unions, or accessing variant payloads without matching.

---

## Verilog Backend & Tooling

### Synthesizer Portability

DDL targets standard Verilog-2005 with strict adherence to constructs accepted by GowinSynthesis and other FPGA toolchains:
- No use of `$clog2` in emitted code (all log2 widths pre-calculated at compile time).
- No dynamic width casting inside expressions (expanded into bit-slices and zero/sign replications).
- No Verilog function calls in output netlists (pure inlined continuous and clocked logic).

All generated files carry a banner comment recording the compiler version and exact command used to build them. See [**Verilog Backend & Synthesizer Portability**](backend-portability.md).

### Multi-File Projects (`import`)

Projects can be split across multiple files using `import`:

```ddl
import "types.ddl"
import "subsystem/core.ddl"
```

- Files are resolved relative to the importing file's directory, followed by paths supplied via `-I` compiler flags.
- Declarations share a unified global namespace.
- Circular and diamond imports are automatically deduplicated.

### Formatter and Tooling

- **Formatter**: `ddl fmt <files>` formats files in place; `ddl fmt --check <files>` verifies formatting in CI.
- **Graphviz DOT**: `ddl build <file> --emit=dot | dot -Tsvg > graph.svg` visualizes module netlists.
- **VS Code Extension**: Syntax highlighting is provided under `editors/vscode/`.
