# DDL Pipeline Lowering & Memory Forwarding

This document explains how the DDL compiler lowers high-level `sequence` declarations into cycle-accurate, backpressured hardware pipelines.

---

## Table of Contents

- [Overview](#overview)
- [Stage Cuts (`|||`) and Pipeline Partitioning](#stage-cuts--and-pipeline-partitioning)
- [Shift-Register Insertion for Value Spanning](#shift-register-insertion-for-value-spanning)
- [Backpressure and Per-Stage Shifts](#backpressure-and-per-stage-shifts)
  - [Why Not One Shift for the Whole Pipeline](#why-not-one-shift-for-the-whole-pipeline)
- [Head Gather and Per-Stage Sends](#head-gather-and-per-stage-sends)
  - [A Join Is Not a `@merge`](#a-join-is-not-a-merge)
  - [Non-blocking Inputs in Any Stage](#non-blocking-inputs-in-any-stage)
  - [Loops of Pipes](#loops-of-pipes)
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
- Downstream backpressure stalls each stage only as far as it has to: a stage waits for its own sinks and for the stage below it.
- Cross-stage values, validity signals, and hazard forwarding are managed automatically.

---

## Stage Cuts (`|||`) and Pipeline Partitioning

During lowering, `split_stages` partitions the statements of a `sequence` at every `|||` marker:
- **Stage 0 (Head)**: Receives the item from the input buffers.
- **Every Stage**: Executes purely combinational datapath operations on the values available at that stage, and may `@send` to output buffers.
- **Stage $N-1$ (Tail)**: The last stage; nothing is registered below it.

Each output is sent to exactly once per item, from whichever stage its `@send` is written in. An output sent from stage $k$ leaves $k$ cycles after the item entered; two outputs sent from different stages carry the same items in the same order, at different latencies.

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
                     (clocked by shift0)    (clocked by shift1)
```

Each register at a cut is loaded by the shift of the stage above it (`shift0` for the first cut, `shift1` for the second), so a value moves exactly when the item it belongs to does.

---

## Backpressure and Per-Stage Shifts

Every stage has its own advance signal, `shift{k}`. A stage moves its item on when every sink it sends to has a slot **and** the cut below it is free -- empty, or emptying because the stage below is moving on the same edge:

```verilog
wire dst_full = dst_wsalt_q == (~dst_rsalt);
wire shift2   = !dst_full;         // the last stage: only its sink
wire shift1   = (!v1) | shift2;    // cut below empty, or the stage below moves
wire shift0   = (!v0) | shift1;
```

In general, $shift_k = room_k \land (\lnot v_k \lor shift_{k+1})$, where $room_k$ is the AND over the slots stage $k$ sends to (true when it sends nothing) and $v_k$ is the validity bit of the cut below it.

- When every sink has room, every `shift{k}` is high and the whole pipeline advances on the edge.
- When a sink is full, the stage sending to it holds, and so does every occupied stage above it. Stages above a **bubble** keep moving into it, so a stall closes up the gaps instead of freezing them in place.
- Upstream items are safely held in the input buffer until the pipeline's own capacity is exhausted.

A one-stage sequence has a single enable, still named `shift`.

The cost is a chain: `shift0` depends on every stage below it within the cycle. Every term in it is a register on this side of the module -- a validity bit, or a slot's own `wsalt` compared with the consumer's `rsalt` -- so the path stays inside the module and never runs through a pipe into another block.

### Why Not One Shift for the Whole Pipeline

A single `shift` over every sink is enough while every send sits in the last stage. It stops being enough once sends sit in different stages. Take a sequence that sends `x` from stage 0 and `y` from stage 2 into a consumer that joins them:

```ddl
sequence s (src: buffer in u8, x: buffer out u8, y: buffer out u8)
  let a = @rcv(src)
  @send(x, a)
  |||
  let b: u8 = a + 8'd1
  |||
  @send(y, b + 8'd1)
```

The consumer cannot drain `x` until the matching `y` arrives, so `x` fills with two items while the first one's `y` is still two stages up. With one global shift, a full `x` holds every stage, the late half never reaches its send, and the design deadlocks after two items. With per-stage shifts, the stages below stage 0 keep moving, `y` arrives, the join drains `x`, and the pipeline runs.

Per-stage shifts make that shape **live**, not **fast**. The early pipe's two entries have to hold every item whose late half has not arrived yet; with a gap of $d$ stages, full throughput needs room for $d + 1$, so any gap runs below one item per cycle when the two outputs rejoin. A deeper buffer on the early path is the fix for rate.

---

## Head Gather and Per-Stage Sends

A `sequence` may declare any number of `buffer in` and `buffer out`
parameters. Every input is received exactly once in stage 0, and every output
sent to exactly once, from whichever stage its `@send` is written in. Sending
the same output twice -- in one stage or in two -- is an error: a consumer is
owed one item per item.

The head reduces to one predicate, and so do the sends of each stage:

```verilog
  wire offered = a_present & b_present;   // every blocking input has an item
  wire shift1  = x_room & y_room;         // every sink of stage 1 has a slot

  wire take = offered & shift0;           // one take, for all inputs
  wire push = shift1 & v0;                // one push, for all of stage 1's outputs
```

The head is a **rendezvous**: the inputs of a single item arrive together, so
either every blocking input gives up an entry on this edge or none does. The
sends of one stage are a **broadcast**: every sink that stage writes is written
from the same item on the same edge. With sends in several stages there is one
push per sending stage, named `push{k}`:

```verilog
  wire open0  = (!v0) | shift1;
  wire shift0 = x_room & open0;           // stage 0 sends x, and waits on the cut below
  wire push0  = shift0 & src_present;
  wire push2  = shift2 & v1;
```

Both reductions are over **slot occupancy**, which is a register on this side of
the wire, and never over the far side's handshake. That is the same argument
[`@split`](combinators.md) makes: ANDing the consumers' readiness would put each
one's logic into every other one's timing path, where asking whether each of
*our* two-entry slots has room reads only local flops. Each sink gets its own
slot, and the stage sending to it waits for the slowest of them.

With one input and one output the reductions have a single term each, so
nothing costs anything until it is used.

### A Join Is Not a `@merge`

They look alike in a graph and are not interchangeable:

| | `@merge` | a sequence head |
|---|---|---|
| takes | one input per cycle | one item from *every* input |
| when | any input is offering | all blocking inputs are offering |
| picks | rotating priority | nothing to pick |
| is | an arbiter, interleaving streams | a rendezvous, zipping them |

Use `@merge` to funnel several producers of the same stream into one. Use a
multi-input sequence when one item is a function of one item from each source.

### Non-blocking Inputs in Any Stage

`@try_rcv`, `@peek` and `@drop` never wait, and may be written in any stage,
inside an `if` or a `match` -- the same operations, lowered by the same code, as
in a `process`. What makes them a pipeline's is *when they take*. An operation
in stage $k$ acts for the item stage $k$ holds: it looks on every cycle, and
takes an entry only on a cycle that stage is live and moving:

```verilog
  wire take   = a_present & shift0;     // the blocking head, as before
  wire b_take = (v0 & shift1) & b_present;  // `@try_rcv(b)` in stage 1
```

So a bubble in stage $k$ takes nothing, and a stage held by a full sink takes
nothing again on each cycle it waits. Under a condition, the arm's guard joins
the `b_present` half. For a stage-0 input, `v0 & shift1` is `take`, which is
exactly the `take & b_present` an optional head input always had.

The `ok` half of `@try_rcv`, and `present` from `@peek`, are `b_present` narrowed
to the operation's path. They do not also say "and this stage moved": nothing a
stage computes on a cycle it does not move is committed -- its crossing
registers, writes, assertions and pushes all wait for the same shift. They cross
the stage cuts like any other value, so a later stage sees the `ok` that belongs
to its item. When `ok` is low, `y` holds whatever the buffer still had -- the
same bargain `@try_rcv` makes in a `process`.

**One stage per input.** Every operation on an `in` pipe must sit in one stage,
for the reason a written memory must (see
[Single-Stage Memory Ownership](#single-stage-memory-ownership)): at any cycle
two stages hold two different items, so a `@peek` in one and a `@drop` in
another would look at one item and throw away another's. Within its stage a
pipe is taken from at most once per item, and two takes in disjoint arms of an
`if` are one. A pipe must be taken from somewhere: one that is only peeked at
never drains, and is refused.

**What paces the head.** A non-blocking input never holds the pipeline up, so
it is absent from `offered` -- a later stage's input has no say in whether stage
0 holds an item at all. With *no* blocking input, `offered` becomes the OR over
the inputs stage 0 touches. It is deliberately not a constant: a head that fired
with nothing on any input would be a free-running source, emitting an item per
cycle out of nothing. A cycle in which nothing arrived is a **bubble** -- stage
0 shifts, the validity bit it shifts in is low, and no `push` commits anything.
Shifting is not committing. A head built from `@peek` is live while its pipe
offers, so an item it declines to `@drop` is emitted again on the next cycle,
as a `process` written the same way would do. A sequence whose first stage
touches no input at all is refused.

### Loops of Pipes

A sequence produces an item only after receiving one on every pipe it blocks on,
and every pipe starts empty. So around a loop of sequences, each one is waiting
for an item only its predecessor can make, and none of them ever fires. The
compiler reports that loop rather than emitting it:

```
error: `acc` waits on itself through acc -> hold -> acc
```

Routing the feedback through a `@try_rcv` makes the same loop live -- an
accumulator that reads last cycle's result on the cycles there is one. The check
stops at sequences: a `process` may send before it ever receives, and an `extern`
is opaque, so a loop through either is left alone. A clean compile is not a proof
of liveness; only the diagnostic is a proof of the opposite.

Note that *reconvergence* is not a loop and is never reported. A `@split` into
two paths of unequal depth rejoining at a two-input sequence makes progress: the
fast path fills and back-pressures, the slow path keeps draining into the join,
and the design runs at the slow path's rate.

---

## Validity Bits and Stage Gating

Bubbles (empty pipeline slots) are tracked using single-bit validity registers, one per cut: `v0`, `v1`, $\dots$, `v(N-2)`. There is no cut below the last stage; what leaves it goes into the slots it sends to, whose `wsalt`s already say they hold it.

A validity bit changes when its cut **opens** -- the stage below is empty or moving -- which is not quite the same as the stage above it moving:

```verilog
always @(posedge clk) begin
  ...
  v0 <= (open0 ? (src_present & x_room) : v0);   // stage 0 sends to x
  v1 <= (shift1 ? v0 : v1);                      // stage 1 sends nothing
end
```

When stage 0's sink is full it holds its item, but the stage below may still leave. The cut then has to take a **bubble**: keeping its old `1` would send the item below a second time. So what enters is the item only if its stage was live and its sends went through. For a stage that sends nothing, the cut opens exactly when the stage moves, and the two signals are one.

- **Output Gating**: A stage pushes to its output buffers only when it holds a valid item and it shifts:
  ```verilog
  wire push = shift1 & v0;
  ```
- **Assertion and Write Gating**: Assertions and memory writes inside stage $k$ take effect only when stage $k$ holds a valid item and `shift{k}` is high, under any enclosing branch conditions. Spurious assertion failures on bubbles are impossible, and a stalled stage never writes twice.

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
4. The BRAM read enable is the asking stage's liveness ANDed with its `shift` signal, so `mem_q` holds while that stage is stalled.

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
  wire shift1   = !dst_full;
  wire shift0   = (!v0) | shift1;
  wire take = src_present & shift0;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
    end else begin
      v0 <= (shift0 ? src_present : v0);
      squared_s1 <= (shift0 ? x * x : squared_s1);
      x_s1       <= (shift0 ? x : x_s1);  // Spans stage cut alongside squared
    end
  end

  // Tail stage computes res using x_s1
  wire [31:0] res = squared_s1 + {{16{1'b0}}, x_s1};
  ...
endmodule
```
