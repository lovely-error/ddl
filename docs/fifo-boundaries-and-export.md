# FIFO Boundary Adapters & Export Architecture

> **Every port on these faces is synchronous to that module's `clk`.** Wiring one
> to logic on another clock corrupts data silently — measured at 100% of items on
> hardware. If logic across a boundary runs on another clock, either wire
> [`lib/ddl_cdc_fifo.v`](../lib/ddl_cdc_fifo.v) by hand, or let the compiler wire
> one with `--async-export` / `--async-extern`. See [Guide 6](guides/6-clock-domain-crossings.md)
> for the recipe and [Clock Domains](clock-domains.md) for the underlying measurements.

This document provides a comprehensive hardware architecture specification of DDL's external boundary interfaces: how the compiler presents **Show-Ahead FIFOs (zero read latency)** at module boundaries, how the four adapter modules in [`src/ir_adapt.rs`](../src/ir_adapt.rs) translate between internal Gray-code salt and external FIFO protocols, and how export targets are resolved and wrapped in [`src/ir_export.rs`](../src/ir_export.rs).

---

## Table of Contents

- [Motivation: Keeping Salt Internal](#motivation-keeping-salt-internal)
- [The Show-Ahead FIFO Contract](#the-show-ahead-fifo-contract)
  - [Zero Read Latency & Head Visibility](#zero-read-latency--head-visibility)
  - [The Write Face (`buffer in`)](#the-write-face-buffer-in)
  - [The Read Face (`buffer out`)](#the-read-face-buffer-out)
  - [Safety Gating & Invariants](#safety-gating--invariants)
- [The Four Adapter Modules (`src/ir_adapt.rs`)](#the-four-adapter-modules-srcir_adaptrs)
  - [Adapter Taxonomy](#adapter-taxonomy)
  - [The READ Logic Core](#the-read-logic-core)
  - [The WRITE Logic Core](#the-write-logic-core)
  - [Synthesis Footprint & Timing Isolation](#synthesis-footprint--timing-isolation)
- [Export Target Discovery (`src/ir_export.rs`)](#export-target-discovery-srcir_exportrs)
  - [Root Analysis on the Complete Use Graph](#root-analysis-on-the-complete-use-graph)
  - [Distinguishing Deliverables from Imports](#distinguishing-deliverables-from-imports)
  - [Accounting for Inlined Calls](#accounting-for-inlined-calls)
  - [Disambiguation: Single Root vs. Multiple Roots](#disambiguation-single-root-vs-multiple-roots)
- [Transitive Closure & Dead-Code Pruning](#transitive-closure--dead-code-pruning)
- [Top-Level Wrapper Generation](#top-level-wrapper-generation)
  - [Core Renaming (`<name>_core`) and Wrapper (`<name>`)](#core-renaming-name_core-and-wrapper-name)
  - [Transparent `wire` Passthrough](#transparent-wire-passthrough)
  - [Bare Export Mode (`--bare-export`)](#bare-export-mode---bare-export)
- [Interfacing & System Verilog Integration](#interfacing--system-verilog-integration)

---

## Motivation: Keeping Salt Internal

DDL's internal channel interconnect is the **Gray-Code Salt Protocol** (see [The Gray-Code Salt Protocol](salt-protocol.md)). Internally, salt achieves vital hardware properties:
- `wsalt` and `rsalt` are direct flip-flop outputs; no combinational path crosses module boundaries.
- The 2-entry capacity absorbs the 1-cycle latency of registered pointer exchange, eliminating combinational `ready` loops and backward timing degradation.

However, exposing raw salt across an external interface places an unacceptable burden on human RTL designers and standard verification tools:
1. **Packed Dual-Slot Bus**: A 32-bit channel flattens into a 64-bit bus carrying both FIFO slots `{slot1, slot0}` at once. A hand-written Verilog consumer had to know which half was active by computing `ridx = rsalt[0] ^ rsalt[1]`.
2. **Stateful Pointer Toggling**: To pop an item, the consumer had to maintain its own registered 2-bit Gray pointer and toggle the correct bit (`rsalt <= rsalt ^ (ridx ? 2'd2 : 2'd1)`).
3. **Inverted Full Detection**: Detecting room on a write port required comparing inverted Gray codes (`wsalt == ~rsalt`).
4. **Third-Party IP Incompatibility**: Standard EDA verification IP (VIP), bus functional models (BFMs), and SoC interconnects expect standard FIFO or `valid`/`ready` handshakes, not proprietary Gray-code pointer mathematics.

To resolve this, DDL **confines the salt protocol strictly to internal modules**. Every boundary a person writes Verilog against—an export target or an `extern` declaration—presents a standard **Show-Ahead FIFO** instead.

---

## The Show-Ahead FIFO Contract

A Show-Ahead FIFO (also known in industry IP catalogs as zero-read-latency FIFO) eliminates the traditional 1-cycle read latency penalty.

### Zero Read Latency & Head Visibility

In a **classic FIFO**, asserting read enable (`rd_en`) requests an item, which only appears on `rd_data` on the *subsequent* clock cycle (read latency = 1). Downstream logic cannot inspect data before deciding to take it.

In DDL's **Show-Ahead FIFO**:
- When `has_data` (`!empty`) is high, the data word is **already valid and stable on `data_read_out` in the exact same cycle**.
- Downstream logic can inspect the payload (e.g. packet headers, opcodes, routing tags) combinationally.
- The item remains stable on `data_read_out` for as many cycles as needed until the consumer explicitly acknowledges receipt by raising `drop_item`.

```
Clock            :  __/~~\__/~~\__/~~\__/~~\__/~~\__/~~\
has_data         :  _______/~~~~~~~~~~~~~~~~~~~~~~~~\___
data_read_out    :  -------<     Item A     >< Item B >-
drop_item        :  ___________________/~~~~~~\___/~~~~~
Handshake (Pop)  :                         ^        ^
                                       Item A   Item B
```

### The Write Face (`buffer in`)

When a DDL module consumes items from the outside world (`src: buffer in T`):

| Port | Direction | Type / Width | Meaning |
|---|---|---|---|
| `<p>_can_receive` | Output | `1 bit` | Buffer has room for at least one entry (`!full`). |
| `<p>_receive_en` | Input | `1 bit` | Write strobe: push `data_write_in` this cycle (`wr_en`). |
| `<p>_data_write_in` | Input | `W bits` | The single data item to push (`wr_data`). |

- The external producer asserts `receive_en = 1` while `can_receive == 1` to push an item.
- Width is strictly the payload width $W$, never a packed pair.

### The Read Face (`buffer out`)

When a DDL module emits items to the outside world (`dst: buffer out T`):

| Port | Direction | Type / Width | Meaning |
|---|---|---|---|
| `<p>_has_data` | Output | `1 bit` | Item is available at the head (`!empty`). |
| `<p>_drop_item` | Input | `1 bit` | Read strobe / pop acknowledgment: consumer accepted item (`rd_en`). |
| `<p>_data_read_out` | Output | `W bits` | The current head item (`rd_data`). |

- `data_read_out` continuously reflects the head item whenever `has_data == 1`.
- Raising `drop_item = 1` acknowledges receipt. On the next clock edge, the current item is retired and the next item (if any) is presented.

### Safety Gating & Invariants

All DDL adapters enforce strict protocol immunity (verified by simulation in [`tests/adapters.rs`](../tests/adapters.rs)):
1. **Conservation of Items**: Zero items are dropped, duplicated, or reordered under any combination of steady-state transfers or irregular backpressure stalls.
2. **Defensive Gating**: If an external master raises `drop_item` when `has_data == 0`, or raises `receive_en` when `can_receive == 0`, the adapter internally suppresses the signal (`has_item & drop_item` and `room & receive_en`). An illegal external pulse cannot corrupt the internal Gray pointers or desynchronize state.
3. **Registered Readiness**: `can_receive` is derived strictly from registered write and read pointers; it never combinationally loops backward through downstream readiness.

---

## The Four Adapter Modules (`src/ir_adapt.rs`)

To bridge between internal salt channels and external Show-Ahead FIFOs, the compiler automatically synthesizes up to four adapter modules:

```
                  Internal Side                       External Boundary
                  (Salt Protocol)                     (Show-Ahead FIFO)

Export Target     [ Core Output ] ===(salt)===> [ ddl_salt_to_rport ] ---> has_data / drop_item / data_read_out
`buffer out`

Export Target     [ Core Input  ] <===(salt)=== [ ddl_wport_to_salt ] <--- can_receive / receive_en / data_write_in
`buffer in`

`extern` Block    [ DDL Graph   ] ===(salt)===> [ ddl_salt_to_wport ] ---> can_receive / receive_en / data_write_in
`buffer in`

`extern` Block    [ DDL Graph   ] <===(salt)=== [ ddl_rport_to_salt ] <--- has_data / drop_item / data_read_out
`buffer out`
```

### Adapter Taxonomy

The four adapters are distinguished by direction and whether the adapter acts as a **slave** or a **master**:

| Adapter Kind | Location | Role | External Face | Internal Salt |
|---|---|---|---|---|
| `SaltToRport` | Export Target `buffer out` | **Slave**: External consumer tells adapter when to drop. | Read Face (Output `has_data`, Input `drop_item`, Output `data_read_out`) | Drains `i_wsalt`/`i_data`, emits `i_rsalt` |
| `WportToSalt` | Export Target `buffer in` | **Slave**: External producer tells adapter when to write. | Write Face (Output `can_receive`, Input `receive_en`, Input `data_write_in`) | Drives `o_wsalt`/`o_data`, reads `o_rsalt` |
| `SaltToWport` | `extern` module `buffer in` | **Master**: Adapter drives external `receive_en` whenever data is ready and extern says `can_receive`. | Write Face (Input `can_receive`, Output `receive_en`, Output `data_write_in`) | Drains `i_wsalt`/`i_data`, emits `i_rsalt` |
| `RportToSalt` | `extern` module `buffer out` | **Master**: Adapter drives external `drop_item` whenever room exists and extern says `has_data`. | Read Face (Input `has_data`, Output `drop_item`, Input `data_read_out`) | Drives `o_wsalt`/`o_data`, reads `o_rsalt` |

Adapters are keyed by shape and width (`ddl_<kind>_<width>`), so multiple channels sharing the same width and direction reuse the same synthesized module.

### The READ Logic Core

Used by `SaltToRport` and `SaltToWport`. It holds **one 2-bit register** (`rsalt_q`):
- Connects to incoming `i_wsalt` and packed `i_data`.
- Computes emptiness: `wire i_empty = (i_wsalt == rsalt_q); wire has_item = !i_empty;`
- Derives slot index: `wire i_ridx = rsalt_q[0] ^ rsalt_q[1];`
- Multiplexes the current item to the output: `wire [W-1:0] i_item = i_ridx ? i_data[2*W-1:W] : i_data[W-1:0];`
- Evaluates `take`:
  - In `SaltToRport` (slave): `wire take = has_item & drop_item;`
  - In `SaltToWport` (master): `wire take = has_item & can_receive;`
- On `take == 1`, advances `rsalt_q <= rsalt_q ^ (i_ridx ? 2'd2 : 2'd1);`.

Because `i_data` comes from registered storage in the upstream core, `i_item` is valid whenever `has_item` is high: **zero read latency**.

### The WRITE Logic Core

Used by `WportToSalt` and `RportToSalt`. It holds **three registers**:
- Two data registers `e0` and `e1` (each of width $W$).
- One 2-bit Gray pointer `wsalt_q`.

Behavior:
- Computes fullness: `wire full = (wsalt_q == ~o_rsalt); wire room = !full;`
- Derives write slot index: `wire widx = wsalt_q[0] ^ wsalt_q[1];`
- Evaluates `push`:
  - In `WportToSalt` (slave): `wire push = room & receive_en;`
  - In `RportToSalt` (master): `wire push = room & has_data;`
- When `push == 1`:
  - Latches incoming data into `e0` (if `!widx`) or `e1` (if `widx`).
  - Advances `wsalt_q <= wsalt_q ^ (widx ? 2'd2 : 2'd1);`.
- Drives internal salt bus: `assign o_data = {e1, e0}; assign o_wsalt = wsalt_q;`.

### Synthesis Footprint & Timing Isolation

- **Storage Cost**: An output adapter (`SaltToRport`) requires only **2 flip-flops** (`rsalt_q`). An input adapter (`WportToSalt`) requires **$2 \times W + 2$ flip-flops** (`e0`, `e1`, `wsalt_q`).
- **Timing Isolation**: Every salt pointer originates from a register. Combinational loops across modules or between external buses and internal state machines are broken by construction.

---

## Export Target Discovery (`src/ir_export.rs`)

When compiling a DDL project with multiple files or declarations, the compiler must decide **which module the build is for**.

### Root Analysis on the Complete Use Graph

An export candidate is a **root** in the use graph: a module that is not used by any other declaration.

Crucially, the use graph tracks **both forms of usage**:
1. **Structural Instantiations**: Modules instantiated inside a `graph`.
2. **Inlined Function Calls**: Combinational functions (`fun`) called within a `sequence`, `process`, or another `fun`.

Because inlined calls are flattened during SSA lowering and leave no structural `Instance` in the IR, call edges are recorded during lowering (`Module.calls`). Without this, inlined helper functions would mistakenly be classified as unreferenced roots.

### Distinguishing Deliverables from Imports

Only declarations from source files **explicitly named on the compiler command line** are eligible to be export roots:

```bash
ddl build app.ddl -o app.v
```

If `app.ddl` contains `import "lib/helpers.ddl"`, unused utility modules inside `helpers.ddl` are **never** treated as candidate deliverables. An import provides dependencies, not top-level deliverables.

### Accounting for Inlined Calls

Consider a file defining a pipeline `alu` and a helper function `is_signed`:
- `alu` calls `is_signed(...)`.
- The call is inlined into `alu`.
- The use graph records `alu -> is_signed`.
- Root detection evaluates:
  - `alu`: unreferenced $\rightarrow$ **Root Candidate**.
  - `is_signed`: referenced by `alu` $\rightarrow$ **Internal Dependency**.
- `alu` is selected as the sole deliverable without requiring any CLI flags.

### Disambiguation: Single Root vs. Multiple Roots

- **Single Root Detected**: The compiler automatically designates it as the export target. No command-line flags are required.
- **Multiple Roots Detected**: If a source file defines multiple independent top-level modules (e.g. two separate pipelines), the compiler **refuses to guess** which one the designer intended to build. It issues a clear diagnostic error:
  ```
  error: several modules could be exported: `block_a`, `block_b`
    = note: name the ones you want with `--export <a>,<b>`, or keep the salt ports with `--bare-export <a>`
  ```

---

## Transitive Closure & Dead-Code Pruning

Once export targets are determined (either by root inference or via `--export`), the compiler computes the **transitive dependency closure** of those targets:
- Starting from the export targets, the compiler walks all instantiation edges (`instances`) and call edges (`calls`).
- Any declaration not in the transitive closure is **pruned completely**.

Unused helpers, uninstantiated pipelines, or dead library imports are omitted from the generated Verilog. Downstream synthesis tools (such as GowinSynthesis or Yosys) and linters never spend cycles elaborating unneeded modules.

---

## Top-Level Wrapper Generation

When a module is selected as an export target, its logic is packaged inside a Show-Ahead FIFO wrapper.

### Core Renaming (`<name>_core`) and Wrapper (`<name>`)

1. The lowered module containing the actual computation keeps its logic and is renamed to `<name>_core`.
2. A new top-level module under the original name `<name>` is created as the wrapper.
3. For each pipe parameter:
   - The wrapper declares external Show-Ahead FIFO ports (`can_receive`/`receive_en`/`data_write_in` or `has_data`/`drop_item`/`data_read_out`).
   - The wrapper declares internal salt nets (`<p>_wsalt`, `<p>_rsalt`, `<p>_data`).
   - The wrapper instantiates the appropriate adapter (`ddl_wport_to_salt_<W>` or `ddl_salt_to_rport_<W>`).
4. The wrapper instantiates `<name>_core`, binding its salt ports to the adapters.

```verilog
// Top-Level Show-Ahead FIFO Wrapper:
module mul3 (
    input         clk,
    input         rst_n,
    output        src_can_receive,
    input         src_receive_en,
    input  [15:0] src_data_write_in,
    output        dst_has_data,
    input         dst_drop_item,
    output [31:0] dst_data_read_out
);

  wire [1:0] src_wsalt;
  wire [1:0] src_rsalt;
  wire [31:0] src_data;
  wire [1:0] dst_wsalt;
  wire [1:0] dst_rsalt;
  wire [63:0] dst_data;

  ddl_wport_to_salt_16 u_src_adapt ( ... );
  ddl_salt_to_rport_32 u_dst_adapt ( ... );
  mul3_core u_mul3_core ( ... );

endmodule
```

### Transparent `wire` Passthrough

Bare physical signals (`wire in T` and `wire out T`) carry no handshake and represent physical pins or sidebands:
- A `wire` passes directly through the wrapper to `<name>_core` without any adapter.
- No storage or logic gates are inserted on bare wires.

### Bare Export Mode (`--bare-export`)

If you want the compiler to emit a module with its raw internal Gray-code salt ports exposed—for protocol verification, custom skid testing, or unit benchmarking—pass `--bare-export`:

```bash
ddl build examples/mul3.ddl -o examples/mul3.v --bare-export mul3
```

In bare mode:
- No wrapper or boundary adapters are generated.
- The module retains its original name (not `<name>_core`).
- The emitted Verilog speaks the Gray-code salt protocol directly.

---

## Interfacing & System Verilog Integration

Because exported modules present Show-Ahead FIFOs, interfacing them with external buses or testbenches requires zero glue logic:

### AXI4-Stream Master Bridge
```verilog
assign m_axis_tvalid = dst_has_data;
assign m_axis_tdata  = dst_data_read_out;
assign dst_drop_item = m_axis_tvalid && m_axis_tready;
```

### AXI4-Stream Slave Bridge
```verilog
assign s_axis_tready    = src_can_receive;
assign src_data_write_in = s_axis_tdata;
assign src_receive_en   = s_axis_tvalid && s_axis_tready;
```

### SystemVerilog Testbench Task
```systemverilog
task send_word(input logic [15:0] val);
  begin
    src_data_write_in <= val;
    src_receive_en    <= 1'b1;
    do @(posedge clk); while (!src_can_receive);
    src_receive_en <= 1'b0;
  end
endtask

task receive_word(output logic [31:0] val);
  begin
    while (!dst_has_data) @(posedge clk);
    val           = dst_data_read_out; // Head item is visible with zero latency
    dst_drop_item <= 1'b1;
    @(posedge clk);
    dst_drop_item <= 1'b0;
  end
endtask
```
