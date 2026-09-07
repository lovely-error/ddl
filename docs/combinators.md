# Zero-Latency Hardware Combinators: `@merge` and `@split`

This document details the architecture and synthesis of DDL's built-in channel combinators: **`@merge`** and **`@split`**.

---

## Table of Contents

- [Overview](#overview)
- [Why Not Processes? (The Zero-Latency Requirement)](#why-not-processes-the-zero-latency-requirement)
- [The `@merge` Combinator (Round-Robin Arbitration)](#the-merge-combinator-round-robin-arbitration)
  - [Starvation-Free Rotating Priority](#starvation-free-rotating-priority)
  - [Hardware Datapath](#hardware-datapath)
- [The `@split` Combinator (Lossless Channel Broadcast)](#the-split-combinator-lossless-channel-broadcast)
  - [Why ANDing Ready Signals is Forbidden](#why-anding-ready-signals-is-forbidden)
  - [Independent Sink Buffering & All-Accept Retirement](#independent-sink-buffering--all-accept-retirement)
  - [Hardware Datapath](#hardware-datapath-1)
- [Generated Verilog Module Naming](#generated-verilog-module-naming)

---

## Overview

In complex dataflow architectures, channels must frequently be multiplexed or distributed:
- **Multiplexing (`@merge`)**: Several producers send messages over a shared interconnect to a single consumer.
- **Distribution (`@split`)**: A single producer broadcasts identical copies of each message to multiple independent consumers.

In DDL, combinators are instantiated structurally inside a `graph`:

```ddl
graph router (
    p: buffer in u32,
    q: buffer in u32,
    o1: buffer out u32,
    o2: buffer out u32
)
  let m: buffer u32
  let d: buffer u32

  @merge(p, q, m)      -- Arbitrates p and q onto m
  transform(m, d)
  @split(d, o1, o2)    -- Broadcasts d to o1 and o2
```

---

## Why Not Processes? (The Zero-Latency Requirement)

It is syntactically possible to implement merge and split operations as a DDL `process`:

```ddl
// Naive merge as a process:
process naive_merge (a: buffer in u32, b: buffer in u32, out: buffer out u32)
  loop
    let (v_a, ok_a) = @try_rcv(a)
    if ok_a then
      @send(out, v_a)
    else
      let (v_b, ok_b) = @try_rcv(b)
      if ok_b then
        @send(out, v_b)
```

However, writing them as a `process` has a severe architectural drawback:
- A `process` is a state machine, and state transitions require clock cycles.
- Every hop through a process-based merge or split adds **1 or more clock cycles of latency**.
- For pure routing nodes whose sole purpose is to pass items along, adding cycle latency wastes bandwidth and introduces pipeline bubbles.

Instead, DDL's compiler (`src/ir_comb.rs`) generates `@merge` and `@split` as **pure, dedicated datapath hardware modules**:
- Items pass through in **0 extra cycles** beyond standard buffer registration.
- Peak throughput is maintained at **1 transfer per cycle**.
- Zero FSM state registers are consumed.

---

## The `@merge` Combinator (Round-Robin Arbitration)

`@merge(in0, in1, ..., out)` multiplexes multiple input buffers onto a single output channel.

### Starvation-Free Rotating Priority

A naive arbiter always favors `in0` over `in1`. If `in0` streams continuously, `in1` experiences indefinite starvation.

DDL's `@merge` implements a **rotating priority bit (`rot`)**:
1. When only one input has an available item, that input is granted immediately.
2. When both `in0` and `in1` have items available in the same cycle, the grant decision is determined by `rot`.
3. Whenever a contested transfer completes, `rot` toggles, ensuring the unselected input receives top priority on the subsequent cycle.

### Hardware Datapath

```
Input 0 (wsalt, data) ----\
                           +---> [ Priority Arbiter ] ---> Output (wsalt, data)
Input 1 (wsalt, data) ----/             ^
                                        |
Output (rsalt) -------------------> [ rot reg ]
```

- **Salt Preservation**: The unselected input's read salt (`rsalt`) is held constant; its item remains safely queued in its buffer without loss.
- **Cycle-Decoupled**: The output write salt (`out_wsalt_q`) is driven from a register, ensuring that downstream backpressure does not form combinational paths back into upstream producers.

---

## The `@split` Combinator (Lossless Channel Broadcast)

`@split(in, out0, out1, ...)` broadcasts an input stream to multiple independent downstream sinks.

### Why ANDing Ready Signals is Forbidden

In conventional SystemVerilog designs, designers frequently implement a broadcast by wiring the input valid to all outputs and ANDing downstream ready signals together:

$$\text{in\_ready} = \text{out0\_ready} \ \ \& \ \ \text{out1\_ready}$$

In a decoupled Gray-code architecture, **this is strictly forbidden**:
- ANDing consumer readiness signals across multiple consumers creates a long combinational fan-in path that couples all downstream modules together.
- If one downstream consumer has complex readiness logic, that logic ripples combinationally into every other consumer on the split, completely destroying the isolation guarantee of the salt protocol.

### Independent Sink Buffering & All-Accept Retirement

DDL avoids combinational coupling by giving **each sink its own independent pair of buffer entries**:
1. **Independent Holding Slots**: Output 0 and Output 1 each possess their own 2-entry skid buffer and write salt (`wsalt`).
2. **All-Accept Condition**: An item is consumed from the input buffer (`@rcv(in)`) if and only if **all output buffers have space**:
   ```verilog
   wire can_broadcast = !out0_full && !out1_full;
   wire take_input    = !in_empty && can_broadcast;
   ```
3. **Synchronized Forwarding**: On the clock edge where `take_input` asserts, the data is pushed into both output buffers simultaneously, and their respective write salts advance in lockstep.

If Sink 0 is ready but Sink 1 is stalled, no transfer occurs: the input item remains at the head of the upstream channel until Sink 1 clears space. Data is never duplicated or lost.

---

## Generated Verilog Module Naming

To minimize code duplication, the compiler analyzes all instantiated combinators across the design and emits deduplicated helper modules parametrized by port width:

- `@merge` instances emit modules named: `ddl_merge_<N>x<WIDTH>` (e.g. `ddl_merge_2x32` for a 2-to-1 32-bit merge).
- `@split` instances emit modules named: `ddl_split_<N>x<WIDTH>` (e.g. `ddl_split_2x32` for a 1-to-2 32-bit split).

If a design contains five 32-bit 2-way merges, `ddl_merge_2x32` is emitted exactly once and instantiated five times.
