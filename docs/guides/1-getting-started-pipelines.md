# Guide 1: Writing Your First Pipelined Accelerator

This guide walks you through building, compiling, and analyzing a pipelined dataflow accelerator in DDL.

---

## What You Will Learn

- How to think in dataflow stages rather than clock-by-clock register assignments.
- How to partition computations using the stage-cut operator (`|||`).
- How DDL automatically inserts shift registers for cross-stage values.
- How to compile DDL to Verilog and inspect the generated control logic.
- How to visualize pipeline structure with Graphviz.

---

## 1. The Design Problem

Suppose you need to design a high-throughput 3-stage arithmetic accelerator:
- **Input**: A stream of 16-bit integers (`u16`).
- **Operation**:
  - Stage 1: Compute `doubled = x + x`.
  - Stage 2: Compute `quad = doubled + doubled` (widened to 32 bits).
  - Stage 3: Add original input `x` to `quad` to produce `result = quad + x` (`u32`).
- **Output**: A stream of 32-bit integers (`u32`).

In conventional Verilog, you would need to:
1. Declare flip-flops for `doubled_r` and `quad_r`.
2. Notice that `x` is needed in Stage 3, and manually create a 2-stage shift register (`x_pipe1`, `x_pipe2`).
3. Wire a validity pipeline (`valid_s1`, `valid_s2`, `valid_s3`).
4. Implement downstream backpressure (`ready`) gating all registers.

In DDL, you write only the arithmetic datapath and stage cuts.

---

## 2. Writing the DDL Source

Create a file named `fir3.ddl`:

```ddl
sequence fir3 (src: buffer in u16, dst: buffer out u32)
  let x = @rcv(src)
  let doubled: u16 = x + x
  |||
  let quad: u32 = @zext(doubled + doubled, 32)
  |||
  let result: u32 = quad + @zext(x, 32)
  @send(dst, result)
```

### Key Syntax Elements:
- `sequence fir3(...)`: Declares a pipelined block.
- `src: buffer in u16`: Input backpressured channel carrying 16-bit words.
- `dst: buffer out u32`: Output backpressured channel carrying 32-bit words.
- `@rcv(src)`: Consumes one item at the head of the pipeline.
- `|||`: Stage cut boundary (maps to one clock cycle latency).
- `@zext(val, 32)`: Explicit zero-extension to 32 bits (DDL prohibits implicit width casting).
- `@send(dst, result)`: Emits the result from the stage it is written in -- here the last one, three cycles after the item arrived.

---

## 3. Compiling to Verilog

Run the DDL compiler:

```bash
ddl build fir3.ddl -o fir3.v
```

By default, the DDL compiler packages your design into two modules:
1. **`fir3` (The Top-Level Wrapper)**: Presents standard **Show-Ahead FIFOs (zero read latency)** to the outside world (`src_can_receive`, `src_receive_en`, `src_data_write_in`, `dst_has_data`, `dst_drop_item`, `dst_data_read_out`). It instantiates compiler-generated boundary adapters that translate to and from internal salt channels.
2. **`fir3_core` (The Pipeline Datapath)**: Contains the clock-by-clock pipeline registers, arithmetic logic, and shift registers, communicating internally over the Gray-code salt protocol.

*(To emit the core directly with raw salt ports without the FIFO wrapper, pass `--bare-export fir3`.)*

Let's examine key parts of the generated Verilog:

### 1. The Top-Level Show-Ahead FIFO Wrapper (`fir3`)

The exported module presents clean, standard FIFO ports that any testbench, AXI-Stream bridge, or Verilog module can drive directly:

```verilog
module fir3 (
    input         clk,
    input         rst_n,
    output        src_can_receive,
    input         src_receive_en,
    input  [15:0] src_data_write_in,
    output        dst_has_data,
    input         dst_drop_item,
    output [31:0] dst_data_read_out
);
```

Because the output face is **Show-Ahead**, `dst_data_read_out` already presents the current valid 32-bit word whenever `dst_has_data` is high. Pulsing `dst_drop_item = 1` acknowledges receipt and advances the pipeline.

### 2. Automatic Shift Registers in `fir3_core`

Inside `fir3_core`, notice how the compiler automatically generated `x_s1` and `x_s2` to carry `x` across two clock boundaries:

```verilog
reg [15:0] x_s1;
reg [15:0] x_s2;
...
always @(posedge clk) begin
  if (!rst_n) begin
    ...
  end else begin
    x_s1 <= (shift0 ? x : x_s1);    // Captured in Stage 1
    x_s2 <= (shift1 ? x_s1 : x_s2); // Shifted into Stage 2
  end
end
```

In Stage 3, `result` uses `x_s2`, guaranteeing that `x` remains synchronized with the exact item that entered the pipeline two cycles prior:

```verilog
wire [31:0] result = quad_s2 + ({{16{1'b0}}, x_s2});
```

### 3. Per-Stage Backpressure Gating

Each stage has its own enable. The last stage waits for its sink; every stage above it waits only until the stage below it is empty or moving:

```verilog
wire dst_full = dst_wsalt_q == (~dst_rsalt);
wire shift2   = !dst_full;
wire shift1   = (!v1) | shift2;
wire shift0   = (!v0) | shift1;
```

When downstream stalls (`dst_full == 1`), `shift2` goes low and the last stage holds. Stages above it keep moving into any empty stage below them, then hold too once they reach an occupied one -- without dropping or corrupting any in-flight data.

---

## 4. Visualizing the Pipeline

To visualize the structural dataflow graph of your design, emit a Graphviz DOT file:

```bash
ddl build fir3.ddl --emit=dot | dot -Tsvg > fir3.svg
```

This draws each stage, channel buffer, and data connection as a visual flowchart.

---

## 5. Rules to Keep in Mind

1. **Blocking Receives at the Head, Everything Else Anywhere**: a `sequence` may declare any number of `buffer in` and `buffer out` parameters. A blocking `@rcv` belongs in Stage 0. Each output is sent to from one stage, at most once per item, and the send may sit inside an `if` or a `match`. An output sent from an earlier stage simply leaves earlier.
2. **The Head Waits for All of Them; a Stage Waits Only for Its Sends**: an item enters the pipeline only when every blocking input is offering one. A stage shifts only when every output this item `@send`s to has room -- an item that skips a full output is not held by it, and a stage that is held never holds the stages after it. `@try_send` never waits: it pushes if there is room and returns whether it did.
3. **Non-blocking Inputs Work in Any Stage**: `let (v, ok) = @try_rcv(p)`, `let (v, here) = @peek(p)` and `@drop(p)` never wait, and may sit in any stage, inside an `if` or a `match`. Each acts for the item its stage holds: it takes an entry only on a cycle that stage is holding an item and moving it on. In Stage 0 with no blocking `@rcv`, an item enters whenever any input the stage touches is offering.
4. **One Stage per Input**: every stage holds a different item at once, so all the operations on one `in` pipe must be in the same stage, and something must take from it -- a pipe that is only peeked at never drains. The same goes for outputs: all the sends to one `out` pipe sit in one stage.
5. **Width Safety**: Adding a 16-bit number to a 32-bit number is a compile-time error. Always explicitly extend using `@zext(val, 32)` or `@sext(val, 32)`.
