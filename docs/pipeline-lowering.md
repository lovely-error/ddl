# DDL Pipeline Lowering & Memory Forwarding

This document explains how the DDL compiler lowers high-level `sequence` declarations into cycle-accurate, backpressured hardware pipelines.

---

## Table of Contents

- [Overview](#overview)
- [Stage Cuts (`|||`) and Pipeline Partitioning](#stage-cuts--and-pipeline-partitioning)
- [Shift-Register Insertion for Value Spanning](#shift-register-insertion-for-value-spanning)
- [Backpressure and the Unified Pipeline Shift](#backpressure-and-the-unified-pipeline-shift)
- [Validity Bits and Stage Gating](#validity-bits-and-stage-gating)
- [Zero-Cost BRAM Stage Alignment](#zero-cost-bram-stage-alignment)
- [Memory Consistency in Pipelines](#memory-consistency-in-pipelines)
  - [Single-Stage Memory Ownership](#single-stage-memory-ownership)
  - [Same-Stage Forwarding ("Plus Its Own")](#same-stage-forwarding-plus-its-own)
  - [Multi-Read Replication](#multi-read-replication)
- [Hardware Example & Verilog Emission](#hardware-example--verilog-emission)

---

## Overview

In DDL, a `sequence` represents a synchronous dataflow pipeline. Designers write sequential code describing how an item is transformed, separating pipeline stages with the stage-cut operator (`|||`):

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

The compiler (`src/ir_pipe.rs`) synthesizes this into a hardware pipeline where:
- Latency is exactly one clock cycle per stage cut.
- Sustained throughput is **1 item per clock cycle** while downstream keeps pace.
- Downstream backpressure automatically stalls the entire pipeline in unison.
- Cross-stage values, validity signals, and hazard forwarding are managed automatically.

---

## Stage Cuts (`|||`) and Pipeline Partitioning

During lowering, `split_stages` partitions the statements of a `sequence` at every `|||` marker:
- **Stage 0 (Head)**: Receives the item from the input buffer.
- **Intermediate Stages**: Execute purely combinational datapath operations on the values available at that stage.
- **Stage $N-1$ (Tail)**: Drives the result into the output buffer.

Every stage boundary maps to a physical bank of registers clocked by `clk`.

---

## Shift-Register Insertion for Value Spanning

A fundamental challenge in pipeline generation is value lifetime:
> *If a value is computed in stage $j$ and consumed in stage $k$ (where $k > j$), how does it arrive at stage $k$ aligned with the correct streaming item?*

If a value were registered only once, stage $k$ would observe a value computed $(k - j - 1)$ cycles too recently, pairing an old item's stage-$k$ computation with a newer item's stage-$j$ value.

DDL solves this by automatically inferring a **shift register of depth $(k - j)$**:
1. When lowering stage $s$, the compiler tracks all live bindings.
2. For every variable referenced in a downstream stage, the compiler inserts a stage register at each intermediate cut boundary.
3. At stage $k$, the variable's identifier resolves to the final stage register output.

```
Stage 0 (j)           Stage 1               Stage 2 (k)
 [ Compute x ] ----> [ Reg x_s1 ] --------> [ Reg x_s2 ] ----> [ Use x ]
                     (clocked by shift)     (clocked by shift)
```

All inserted pipeline registers share the global pipeline shift enable (`shift`), ensuring perfect synchronization across stalls.

---

## Backpressure and the Unified Pipeline Shift

A DDL pipeline moves forward as a single coordinated unit. The global advance signal is named `shift`:

```verilog
wire dst_full = (out_wsalt_q == (~dst_rsalt));
wire shift    = !dst_full;
```

- When the downstream consumer has room (`dst_full == 0`), `shift` is high, and every pipeline stage advances simultaneously on the rising clock edge.
- When downstream asserts backpressure (`dst_full == 1`), `shift` falls low. Every stage register and validity bit holds its current state.
- Upstream items are safely held in the input buffer without dropping or stalling the producer until the pipeline's own skid capacity is exhausted.

---

## Validity Bits and Stage Gating

Bubbles (empty pipeline slots) are tracked using single-bit validity registers: `v0`, `v1`, $\dots$, `v(N-1)`.

On every clock edge:
```verilog
always @(posedge clk) begin
  if (!rst_n) begin
    v0 <= 1'b0;
    v1 <= 1'b0;
  end else if (shift) begin
    v0 <= !src_empty;  // Stage 0 valid if fresh input was consumed
    v1 <= v0;          // Validity propagates downstream
  end
end
```

- **Output Gating**: The tail stage pushes to the destination buffer only when both the pipeline advances and the final stage contains a valid item:
  ```verilog
  wire out_push = shift & v_last;
  ```
- **Assertion Gating**: Assertions written inside a pipeline stage execute only when `shift` is active, the stage validity bit is asserted, and any enclosing branch conditions are true. Spurious assertion failures on uninitialized bubbles are impossible.

---

## Zero-Cost BRAM Stage Alignment

Synchronous Block RAM (`#[impl(bram)]`) has an intrinsic 1-cycle read latency: the address is sampled on clock cycle $T$, and data appears on cycle $T+1$.

In naive hardware compilers, reading a BRAM inside a pipeline requires adding an extra pipeline register for the address plus a separate holding register for the data, wasting flip-flops.

DDL recognizes that **a Block RAM read latency is identical to a pipeline stage cut**:

```ddl
sequence lookup (src: buffer in addr_t, dst: buffer out data_t)
  let a = @rcv(src)
  let v = mem[a]
  |||
  let res = v + 1
  @send(dst, res)
```

Lowering optimization:
1. The address `a` is presented to the BRAM address port in Stage 0.
2. The stage cut `|||` aligns with the BRAM's internal clocked output register.
3. In Stage 1, the identifier `v` maps directly to `mem_q` (the output register of the BRAM).
4. The BRAM read enable is tied directly to the pipeline `shift` signal.

**Result**: Zero dedicated flip-flops are consumed for the boundary; the Block RAM primitive's internal silicon output register functions as the pipeline register.

---

## Memory Consistency in Pipelines

### Single-Stage Memory Ownership

In a hardware pipeline, multiple stages are active simultaneously on different data items:
- Stage $j$ processes item $X$
- Stage $j+1$ processes item $X-1$
- Stage $j+2$ processes item $X-2$

If stage $j$ writes to memory `M` while stage $j+2$ reads from memory `M`, the read would observe writes from an item 2 cycles in the future, destroying sequential causality.

**DDL Rule**: A memory that is **written** must be owned entirely by **one stage**.
- A written memory is confined to a single pipeline stage. Within that stage, every item observes all writes from prior items, plus its own, in source order.
- A **read-only** memory (e.g. a lookup table or ROM) has no write ordering constraints and may be read concurrently from any stage.

### Same-Stage Forwarding ("Plus Its Own")

When an item both reads and writes to the same memory within its stage, a write collision occurs on that clock edge: the BRAM array is updated on the edge, so the raw BRAM read output `_q` would deliver the *old* value prior to this item's write.

To preserve sequential source order ("every item sees its own writes"), DDL synthesizes **same-cycle forwarding hardware**:
1. Address comparison logic evaluates whether the read address matches the write address.
2. If addresses collide and the write enable is asserted, the collision bit and write data are forwarded across the cut.
3. The read result is muxed:
   ```verilog
   wire [W-1:0] read_data = fwd_hit ? fwd_data_q : mem_q;
   ```

### Multi-Read Replication

In a `process`, multiple reads occur in mutually exclusive states and can be time-multiplexed onto a single read port. In a `sequence`, however, every stage is active concurrently.

When multiple stages read from a read-only memory, DDL infers multiple independent read ports. On FPGAs lacking multi-read block RAM primitives, the compiler replicates the memory array across multiple BRAM instances under identical write streams, ensuring conflict-free simultaneous reads.

---

## Hardware Example & Verilog Emission

Consider a 2-stage pipeline with cross-stage value spanning:

```ddl
sequence mac (src: buffer in u16, dst: buffer out u32)
  let x = @rcv(src)
  let squared = @zext(x * x, 32)
  |||
  let res = squared + @zext(x, 32)  -- x is used across the cut!
  @send(dst, res)
```

Generated Verilog structure (simplified):

```verilog
module mac (
    input clk, input rst_n,
    input [1:0] src_wsalt, output [1:0] src_rsalt, input [31:0] src_data,
    output [1:0] dst_wsalt, input [1:0] dst_rsalt, output [63:0] dst_data
);
  reg v0;
  reg [31:0] squared_s1;
  reg [15:0] x_s1;          // Automatically inserted shift register for x

  wire dst_full = (dst_wsalt_q == ~dst_rsalt);
  wire shift    = !dst_full;
  wire src_take = !src_empty & shift;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
    end else if (shift) begin
      v0 <= src_take;
      squared_s1 <= x * x;
      x_s1       <= x;      // Spans stage cut alongside squared
    end
  end

  // Tail stage computes res using x_s1
  wire [31:0] res = squared_s1 + {{16{1'b0}}, x_s1};
  ...
endmodule
```
