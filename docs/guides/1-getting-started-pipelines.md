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
- `@send(dst, result)`: Emits the result at the tail of the pipeline.

---

## 3. Compiling to Verilog

Run the DDL compiler:

```bash
ddl build fir3.ddl -o fir3.v
```

Let's examine key parts of the generated `fir3.v`:

### Automatic Shift Register for `x`

Notice how the compiler automatically generated `x_s1` and `x_s2` to carry `x` across two clock boundaries:

```verilog
reg [15:0] x_s1;
reg [15:0] x_s2;
...
always @(posedge clk) begin
  if (!rst_n) begin
    ...
  end else if (shift) begin
    x_s1 <= x;    // Captured in Stage 1
    x_s2 <= x_s1; // Shifted into Stage 2
  end
end
```

In Stage 3, `result` uses `x_s2`, guaranteeing that `x` remains synchronized with the exact item that entered the pipeline two cycles prior:

```verilog
wire [31:0] result = quad_s2 + ({{16{1'b0}}, x_s2});
```

### Unified Backpressure Gating

All stage registers share a single enable wire: `shift`:

```verilog
wire dst_full = out_wsalt_q == (~dst_rsalt);
wire shift    = !dst_full;
```

When downstream stalls (`dst_full == 1`), `shift` goes low, freezing the pipeline without dropping any data.

---

## 4. Visualizing the Pipeline

To visualize the structural dataflow graph of your design, emit a Graphviz DOT file:

```bash
ddl build fir3.ddl --emit=dot | dot -Tsvg > fir3.svg
```

This draws each stage, channel buffer, and data connection as a visual flowchart.

---

## 5. Rules to Keep in Mind

1. **One Receive at Head, One Send at Tail**: A `sequence` must receive from its input buffer in Stage 0, and send to its output buffer in the final stage.
2. **No Non-blocking Buffer Ops in Pipelines**: Primitives like `@peek`, `@try_rcv`, and `@drop` are not allowed in a `sequence` because every pipeline stage is active concurrently. If you need dynamic inspection or packet dropping, use a `process`.
3. **Width Safety**: Adding a 16-bit number to a 32-bit number is a compile-time error. Always explicitly extend using `@zext(val, 32)` or `@sext(val, 32)`.
