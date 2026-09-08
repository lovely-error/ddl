# DDL (Dataflow Description Language)

DDL is a hardware description language that compiles high-level dataflow specifications into synthesizable Verilog-2005.

While [`desc.md`](desc.md) outlines the long-term design goals of the language, this document describes the language and features implemented in the compiler today.

---

## Table of Contents

- [Quick Start](#quick-start)
- [Practical Usage Guides](#practical-usage-guides)
- [Architecture Deep Dives](#architecture-deep-dives)
- [Design Philosophy](#design-philosophy)
- [Core Declarations](#core-declarations)
  - [Pipelines (`sequence`)](#pipelines-sequence)
  - [State Machines (`process`)](#state-machines-process)
  - [Combinational Functions (`fun`)](#combinational-functions-fun)
  - [Structural Composition (`graph`)](#structural-composition-graph)
  - [External Modules (`extern`)](#external-modules-extern)
- [Channels and Interconnect](#channels-and-interconnect)
  - [Backpressured Buffers (`buffer`)](#backpressured-buffers-buffer)
  - [The Gray-Code Salt Protocol](#the-gray-code-salt-protocol)
  - [Bare Signals (`wire`)](#bare-signals-wire)
  - [Boundaries and the FIFO interface](#boundaries-and-the-fifo-interface)
  - [Channel Operations](#channel-operations)
  - [Combinators (`@merge` and `@split`)](#combinators-merge-and-split)
- [Type System and Storage](#type-system-and-storage)
  - [Integers and Bit Operations](#integers-and-bit-operations)
  - [Structs and Tagged Unions](#structs-and-tagged-unions)
  - [Packed Arrays](#packed-arrays)
  - [Memories (`lutram` and `bram`)](#memories-lutram-and-bram)
  - [Multi-Port Synthesis and LVT BRAM](#multi-port-synthesis-and-lvt-bram)
- [Statements and Scoping](#statements-and-scoping)
- [Assertions](#assertions)
- [Compile-Time Diagnostics](#compile-time-diagnostics)
- [Verilog Backend and Tooling](#verilog-backend-and-tooling)
  - [Synthesizer Compatibility](#synthesizer-compatibility)
  - [Multi-File Projects](#multi-file-projects)
  - [Formatter](#formatter)
  - [Graph Visualization](#graph-visualization)
  - [Editor Support](#editor-support)
- [Testing and Verification](#testing-and-verification)
- [Repository Layout](#repository-layout)

---

## Practical Usage Guides

For step-by-step tutorials, design patterns, and hardware integration guides, see:

- [**Guide 1: Writing Your First Pipelined Accelerator**](docs/guides/1-getting-started-pipelines.md): Dataflow stage cuts (`|||`), shift-register spanning, compiling, and visualizing.
- [**Guide 2: Control State Machines, Packet Parsers, and Register Interfaces**](docs/guides/2-fsm-and-command-processors.md): Memory-mapped CSRs, tagged union dispatchers, zero-cycle branch dispatch, and lossless relays.
- [**Guide 3: Interfacing DDL with Existing Verilog, AXI-Stream, and FPGA Pins**](docs/guides/3-interfacing-and-integration.md): Connecting to physical chip pins via `wire`, `extern` IP integration over the FIFO boundary, and AXI4-Stream master/slave wrappers.
- [**Guide 4: Memory Patterns: ROMs, Block RAMs, and Multi-Port Register Files**](docs/guides/4-memory-and-register-files.md): `lutram` vs. `bram`, zero-cost BRAM stage alignment, and multi-write register files with `--lvt-bram`.
- [**Guide 5: Simulation, Verification, and Build Workflows**](docs/guides/5-verification-and-simulation.md): Edge-triggered simulation assertions, SystemVerilog testbench templates, Verilator `-Wall` linting, and Makefiles.

---

## Architecture Deep Dives

For detailed hardware design documents on DDL's compilation passes, interconnect protocols, and FPGA synthesis targets, see:

- [**The Gray-Code Salt Protocol**](docs/salt-protocol.md): 2-bit Gray-code pointer mathematics, cycle-by-cycle waveform traces, skid buffer proofs, and AXI-Stream adapters.
- [**Pipeline Lowering & Memory Forwarding**](docs/pipeline-lowering.md): `sequence` stage cuts (`|||`), shift-register spanning, backpressure, and zero-cost BRAM alignment.
- [**Finite State Machine Synthesis & Scoping**](docs/fsm-synthesis.md): `process` control-flow graph construction, zero-cycle branch dispatch, lexical register allocation, and resource sharing.
- [**Multi-Write BRAM via Live Value Tables**](docs/lvt-bram-architecture.md): Synthesizing multi-write memories on FPGAs using single-write BRAM banks and distributed LVTs (`--lvt-bram`).
- [**Zero-Latency Hardware Combinators**](docs/combinators.md): Pure datapath implementation of `@merge` (rotating priority) and `@split` (lossless broadcast).
- [**Verilog Backend & Synthesizer Portability**](docs/backend-portability.md): Restrictive Verilog-2005 subset, avoiding GowinSynthesis toolchain crashes, and Verilator `-Wall` linting.
- [**GitHub CI/CD & Multi-Platform Release Pipeline**](docs/ci-release-pipeline.md): Automated multi-platform builds (Linux, Windows, macOS), nightly cron releases, and publishing runbook.

---

## Quick Start

The DDL compiler requires a nightly Rust toolchain, pinned via `rust-toolchain.toml`.

### Build the Compiler

```bash
cargo build
```

### Compile DDL to Verilog

```bash
./target/debug/ddl build examples/mul3.ddl -o examples/mul3.v
```

Use the `--check` flag to verify that an existing output file matches what the compiler would emit without modifying it:

```bash
./target/debug/ddl build examples/mul3.ddl -o examples/mul3.v --check
```

---

## Design Philosophy

In conventional Verilog and SystemVerilog, designers must manually implement handshakes, FIFOs, `valid`/`ready` signals, and intermediate pipeline registers. Every manual handshake creates potential combinational paths through `ready`, risking deadlocks and timing failures that are difficult to catch prior to physical synthesis or silicon bring-up.

DDL automates protocol and register management:
- **Focus on data transformation**: The designer describes operations on data streams; the compiler generates the control logic, pipeline stages, validity signals, and skid registers.
- **Isolated state**: There is no global or shared memory. Declarations encapsulate their own state, and all inter-block communication takes place over point-to-point pipes.
- **Cycle-decoupled handshakes**: Channels eliminate combinational `ready` loops by construct through registered, gray-coded pointer exchange.

---

## Core Declarations

DDL provides five primary top-level declarations:

| Declaration | Hardware Equivalent | Description |
|---|---|---|
| `sequence` | Pipeline | Pipelined datapath partitioned into clock stages by cut operators (`\|\|\|`). |
| `process` | Finite State Machine | Sequential control flow with blocking channel waits. |
| `fun` | Combinational Module | Pure combinational logic; inlined at call sites or emitted as a standalone module. |
| `graph` | Structural Netlist | Instantiates blocks and wires their communication channels together. |
| `extern` | Module Interface | Declares third-party or hand-written Verilog modules for integration in a `graph`. |
| `wire` | Bare Signal | An unhandshaked signal on an `extern` or a `graph`: a pin, or a sideband. |

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

The compiler translates this into a three-stage pipeline (see [`examples/mul3.v`](examples/mul3.v)):
- Each stage includes an automatically managed validity bit and downstream backpressure handling.
- A `sequence` receives once from its input buffer in the first stage and sends once to its output buffer in the final stage.
- Non-blocking buffer operations (`@peek`, `@try_rcv`, `@drop`, `@try_send`) are not permitted in pipelines; use a `process` when non-blocking buffer access is required.

For details on shift-register generation, BRAM stage alignment, and same-cycle hazard forwarding, see [**Pipeline Lowering & Memory Forwarding**](docs/pipeline-lowering.md).

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

The compiler synthesizes this into an FSM (see [`examples/reg_port.v`](examples/reg_port.v)) where branching decisions take effect in the cycle the triggering command is received.

- A `loop` executes repeatedly across cycles.
- A process body without an enclosing `loop` executes its sequential statements once and halts.

For details on CFG state construction, zero-cycle branch dispatch, and lexical register allocation across waits, see [**Finite State Machine Synthesis & Scoping**](docs/fsm-synthesis.md).

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

Each pipe reaches the hand-written module as an ordinary FIFO interface, never as the internal salt protocol. For `psram` above:

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

The compiler puts an adapter on its own side of the boundary to translate, so nothing you write has to reconstruct the salt protocol. See [Boundaries and the FIFO interface](#boundaries-and-the-fifo-interface).

An `extern` may also declare `wire in` / `wire out` parameters, for pins and sidebands that carry no handshake at all.

---

## Channels and Interconnect

Everything a DDL module computes with travels through a **`buffer`**: a backpressured, lossless, point-to-point channel. A **`wire`** carries a bare signal with no handshake, and is allowed only on an `extern` or a `graph` -- the two declarations that sit at the edge of the program and compute nothing.

### Backpressured Buffers (`buffer`)

A `buffer` provides flow-controlled point-to-point communication:
- **Depth**: Every buffer has a fixed depth of 2 entries (a primary register and a skid register).
- **Backpressure**: Producers stall automatically when the buffer is full; consumers stall when it is empty.
- **Lossless Guarantee**: Data is never silently dropped or overwritten. A transfer occurs only when both producer and consumer agree.

### The Gray-Code Salt Protocol

Rather than using traditional `valid`/`ready` handshakes, DDL flattens each `buffer` into three Verilog ports. This is the *internal* interconnect, between two modules the compiler wrote; a boundary you wire up by hand gets a [FIFO interface](#boundaries-and-the-fifo-interface) instead.

- `<p>_wsalt`: 2-bit write pointer emitted by the producer.
- `<p>_rsalt`: 2-bit read pointer emitted by the consumer.
- `<p>_data`: Data bus carrying both FIFO slots.

Both pointers are registered and gray-coded:

```verilog
wire src_empty = (src_wsalt == src_rsalt_q);     // Pointers match: buffer is empty
wire dst_full  = (dst_wsalt_q == (~dst_rsalt));  // Differ in both bits: buffer is full
```

Advancing a pointer *is* the transfer. Because both pointers originate from registers, there are no combinational paths linking consumer readiness to producer validity. This eliminates two classic hardware handshake pitfalls by construction:
- **Combinational timing loops**: In traditional `valid`/`ready` interfaces, downstream readiness is often combinationally coupled to upstream validity. Chaining blocks or introducing feedback paths can inadvertently close a zero-delay combinational loop through `ready`—a hazard that synthesis tools may mishandle or that causes silicon to lock up.
- **Mutual-wait protocol deadlocks**: In standard handshakes, protocol bugs can arise where a producer waits for `ready` before asserting `valid`, while the consumer waits for `valid` before asserting `ready`, causing both to wait indefinitely. With DDL's salt protocol, neither side waits for a same-cycle response from the other; transfer decisions are determined strictly from registered pointers, and the 2-entry buffer capacity (head + skid) absorbs the 1-cycle latency of registered pointer updates.

For a complete architectural breakdown, including cycle-by-cycle waveform diagrams and skid buffer analysis, see [**The Gray-Code Salt Protocol**](docs/salt-protocol.md).

### Bare Signals (`wire`)

A `wire` is a signal of the declared width with no handshake at all: a pin, a clock-domain sideband, a status flag from a piece of vendor IP. Because there is nothing on it to wait for, it is legal only where nothing waits -- an `extern` or a `graph`:

```ddl
extern pll (locked: wire out u1, cfg: buffer in u32)

graph board (cfg: buffer in u32, lock_led: wire out u1)
  pll(lock_led, cfg)
```

- `x: wire in T` emits one input `x` of `T`'s width; `y: wire out T` emits one output.
- A graph routes a wire straight through to its own boundary, and checks that exactly one instance drives each output.
- A `process` or `sequence` cannot take one. Their parameters are pipes and constant parameters, because a body that could present a raw wire could hand-roll a protocol over it, and the protocol is the compiler's job.

### Boundaries and the FIFO interface

The salt protocol above is how two DDL-compiled modules talk to each other. It never crosses a boundary a person writes Verilog against. An `extern`, and any module chosen as an **export target**, presents an ordinary FIFO instead:

| `p: buffer in T` (the module consumes) | dir | meaning |
| --- | --- | --- |
| `p_can_receive` | output | there is room |
| `p_receive_en` | input | write `p_data_write_in` this cycle |
| `p_data_write_in [W-1:0]` | input | the item |

| `p: buffer out T` (the module produces) | dir | meaning |
| --- | --- | --- |
| `p_has_data` | output | an item is available |
| `p_drop_item` | input | I took it; advance |
| `p_data_read_out [W-1:0]` | output | the item |

This is `!full`/`wr_en`/`wr_data` and `!empty`/`rd_en`/`rd_data` with first-word-fall-through. `receive_en` and `drop_item` are honoured only while the matching flag is high; the far side gates them, as it would on any FIFO. The translation is a module the compiler writes -- `ddl_wport_to_salt_32` and friends -- so no hand-written file contains a gray-code pointer.

#### Choosing the export target

An **export target** is the module the invocation is for. It is found from the use graph: a **root** is a module that nothing else instantiates or calls, among the declarations in the files named on the command line. An `import` supplies a place to look for dependencies, not a list of things to ship, so its declarations are never candidates.

The emitted file is **the targets and everything they use**, and nothing else. Asking for one module does not ship an unrelated one that happened to be in the same source.

- One root: it is the target, and no flag is needed.
- Several roots: the compiler names them and asks, rather than picking which module your file is for.

```bash
ddl build src.ddl -o src.v --export top          # this one presents a FIFO
ddl build src.ddl -o src.v --bare-export top     # keep the raw salt ports
ddl build src.ddl -o src.v --export a,b,c        # several, comma-separated
```

A target's logic keeps its shape and takes the name `<name>_core`; the wrapper under the original name holds the FIFO ports, one adapter per pipe, and one instance of the core. `--bare-export` emits the module exactly as it lowers, salt ports and all, for anyone who wants to speak the protocol directly.

### Channel Operations

A `buffer` is reached through the built-in channel intrinsics:

| Operation | Behavior |
|---|---|
| `@rcv(ch)` | Blocks until an item is available, then consumes it.  |
| `@send(ch, val)` | Blocks until room is available, then pushes the value.  |
| `@peek(ch)` | Returns `(data, present)`. Inspects the head item without consuming it.  |
| `@try_rcv(ch)` | Non-blocking receive: returns `(data, ok)`. Consumes the item if available.  |
| `@try_send(ch, val)` | Non-blocking send: returns a boolean indicating whether the push succeeded.  |
| `@drop(ch)` | Consumes and discards the head item without reading its payload.  |

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

### Combinators (`@merge` and `@split`)

DDL includes built-in structural combinators for channel multiplexing and distribution:

- **`@merge(in0, in1, out)`**: Arbitrates two input buffers onto a single output channel using rotating round-robin priority. Prevents starvation without introducing multi-cycle FSM latency.
- **`@split(in, out0, out1)`**: Broadcasts a single input channel to multiple consumers. Each consumer receives its own independent 2-entry skid buffer. An item is retired from the input only when *all* consumers have accepted it.

Combinators are generated as dedicated datapath modules (e.g., `ddl_merge_2x32`) without internal state machine overhead.

For complete datapath diagrams and starvation-free arbitration details, see [**Zero-Latency Hardware Combinators**](docs/combinators.md).

---

## Type System and Storage

### Integers and Bit Operations

- **Arbitrary-Width Types**: Unsigned `uN` and signed `iN` integers (e.g., `u1`, `u8`, `u32`, `i16`).
- **Strict Width Safety**: Mixing bitwidths or mixing signed and unsigned values in an operation causes a compile error. Automatic truncation or extension is prohibited; use `@zext(val, width)`, `@sext(val, width)`, or `@trunc(val, width)`.
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
- Out-of-bounds indexing is rejected at compile time when constant, and bounds-checked during synthesis.

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

### Multi-Port Synthesis and LVT BRAM

The compiler infers one read/write port per concurrent access:
- Two sequential writes in the same cycle require two physical write ports.
- Mutually exclusive writes within `if`/`else` branches share a single write port.

When targeting FPGAs that lack native dual-write block RAM cells (such as the Gowin GW1NR-9C), pass the `--lvt-bram` flag:

```bash
ddl build src/top.ddl -o build/top.v --lvt-bram
```

This synthesizes a multi-write RAM using single-write BRAM blocks coordinated by a Live Value Table (LVT) implemented in distributed logic. See [`examples/rf_lvt.ddl`](examples/rf_lvt.ddl) for benchmarks.

For an in-depth breakdown of LVT mechanics, memory banking, and FPGA synthesis benchmarks, see [**Multi-Write BRAM via Live Value Tables**](docs/lvt-bram-architecture.md).

---

## Statements and Scoping

DDL supports structured imperative control flow:

- **Bindings**:
  - `let name: T = expr`: Immutable local binding.
  - `var name: T = expr`: Mutable binding.
- **Variable Scoping**:
  - Process-level `var` declarations declared at the start of a `process` persist as registers across cycles and are initialized on reset.
  - Local `var` declarations inside loops and conditional blocks have lexical scope. They re-initialize whenever execution enters their scope and retain state across wait cycles within that scope.
- **Assignments**: Standard (`=`) and compound (`+=`, `-=`, `<<=`, `>>=`, `&=`, `|=`, `^=`).
- **Conditionals (`if` / `else`)**: In FSMs, conditions can fork execution across states. In combinational blocks, all branches must assign outputs to avoid latches.
- **Pattern Matching (`match`)**: Matches on enum variants. Exhaustiveness checking is strictly enforced.
- **Loops**:
  - `loop { ... }`: Infinite loop in a `process`, modeling recurring state cycles.
  - `break`: Exits the innermost loop. At the top level of a process, `break` permanently halts execution.
  - `for i in 0..n { ... }`: Compile-time unrolled loop. Trip bounds must be statically determinable. Blocking channel operations and synchronous memory reads are disallowed within `for` loops.

---

## Assertions

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
- **Invalid blocking operations**: Placing a blocking `@rcv` or `@send` inside an `if` condition, `match` scrutinee, or unrolled `for` loop.
- **Unsupported memory usage**: Invoking a synchronous `bram` read within a stateless `fun`, or attaching an asynchronous reset to a BRAM primitive.
- **Dynamic indexing restrictions**: Placing computed indices at any position other than the terminal step of an assignment target.
- **Tagged union misuse**: Performing direct comparisons (`==`) on tagged unions, or accessing variant payloads without matching.

---

## Verilog Backend and Tooling

### Synthesizer Compatibility

DDL targets standard Verilog-2005 with strict adherence to constructs accepted by GowinSynthesis and other FPGA toolchains:
- No use of `$clog2` in emitted code.
- No dynamic width casting inside expressions.
- No Verilog function calls in output netlists.

All generated files carry a banner comment recording the compiler version and exact command used to build them.

For complete details on toolchain failure modes, forbidden Verilog constructs, and automated Verilator `-Wall` linting, see [**Verilog Backend & Synthesizer Portability**](docs/backend-portability.md).

### Multi-File Projects

Projects can be split across multiple files using `import`:

```ddl
import "types.ddl"
import "subsystem/core.ddl"
```

- Files are resolved relative to the importing file's directory, followed by paths supplied via `-I` compiler flags.
- Declarations share a unified global namespace.
- Circular and diamond imports are automatically deduplicated.

### Formatter

DDL includes an AST-preserving code formatter:

```bash
ddl fmt examples/*.ddl          # Format files in place
ddl fmt --check examples/*.ddl  # Check formatting (exits with code 1 on mismatch)
```

The formatter adjusts whitespace, blank lines, and trailing spaces. It does not alter block indentation, ensuring syntax trees remain identical before and after formatting.

### Graph Visualization

To inspect structural connectivity and feedback loops in a `graph`, export a Graphviz DOT representation:

```bash
ddl build examples/pipeline_graph.ddl --emit=dot | dot -Tsvg > pipeline.svg
```

### Editor Support

Syntax highlighting for Visual Studio Code is available in `editors/vscode/`. To install:

```bash
cp -r editors/vscode ~/.vscode/extensions/ddl
```

The VS Code grammar is automatically verified against compiler keywords and builtins by `tests/editor.rs`.

---

## Testing and Verification

The repository includes a comprehensive test and validation suite:

### Unit Tests and Clippy

```bash
cargo test
cargo clippy --all-targets
```

### Undefined Behavior Checks (Miri)

The AST parser's pointer operations are validated under Miri:

```bash
MIRIFLAGS=-Zmiri-disable-isolation cargo miri test --test fuzz
```

### Fuzz Testing

The compiler includes an integrated fuzzer for parser and type checker robustness:

```bash
# Run 1,000,000 iterations of fuzzing
DDL_FUZZ_ITERS=1000000 cargo test --release --test fuzz
```

### Verilator Linting

If [Verilator](https://www.veripool.org/verilator/) is installed, test runs automatically lint generated Verilog files at `-Wall` to check for latches, multi-driven nets, combinational loops, and unclocked registers:
- On Linux/macOS, standard package manager installations work out of the box.
- On Windows under MSYS2:
  ```bash
  pacman -S mingw-w64-x86_64-verilator
  export VERILATOR=/c/msys64/mingw64/bin/verilator_bin.exe
  export VERILATOR_ROOT=/c/msys64/mingw64/share/verilator
  ```

### Synthesis and Equivalence Verification

The verification script [`examples/verify.sh`](examples/verify.sh) compares generated Verilog modules against reference SystemVerilog implementations using QuestaSim and the Gowin synthesis toolchain, verifying both functional cycle-equivalence and post-synthesis resource utilization.

---

## Repository Layout

```
src/
├── main.rs         CLI entry point and command-line parsing
├── lib.rs          Compiler library interface
├── driver.rs       Pipeline driver coordinating compiler passes
├── source.rs       Source file manager and import resolution
├── lex.rs          Lexer and token definitions
├── parse.rs        Recursive-descent parser and AST representation
├── symbols.rs      Symbol table and lexical scope resolution
├── ty.rs           Type inference, validation, and width checking
├── ir.rs           Typed Static Single Assignment (SSA) intermediate representation
├── ir_pipe.rs      Lowers sequence declarations into pipeline stages
├── ir_fsm.rs       Lowers process declarations into finite state machines
├── ir_graph.rs     Lowers graph declarations into structural instance netlists
├── ir_comb.rs      Generates hardware for @merge and @split combinators
├── ir_match.rs     Lowers match expressions into case statements
├── verilog.rs      Synthesizable Verilog-2005 code generator
└── diag.rs         Source diagnostics and caret rendering
```
