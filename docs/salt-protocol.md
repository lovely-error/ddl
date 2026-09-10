# The DDL Gray-Code Salt Protocol

This document provides an in-depth explanation of DDL's point-to-point channel interconnect: the **Gray-Code Salt Protocol**.

> **This protocol is internal.** It runs between two modules the compiler wrote, where both ends are generated together and the properties below are worth their cost. It is not what you wire up by hand.
>
> Every boundary a person writes Verilog against — an `extern`, or the module you asked the compiler to export — presents a **Show-Ahead FIFO (zero read latency)** instead: `p_can_receive` / `p_receive_en` / `p_data_write_in` going in, and `p_has_data` / `p_drop_item` / `p_data_read_out` coming out. The compiler puts an adapter on its own side to translate. See [FIFO Boundaries & Export](fifo-boundaries-and-export.md) and [Guide 3](guides/3-interfacing-and-integration.md) for that interface; read on for what sits behind it.

---

## Table of Contents

- [Motivation: The Pitfalls of `valid`/`ready`](#motivation-the-pitfalls-of-validready)
  - [1. Combinational Timing Loops](#1-combinational-timing-loops)
  - [2. Mutual-Wait Protocol Deadlocks](#2-mutual-wait-protocol-deadlocks)
  - [3. Backwards Combinational Paths and $F_{\max}$](#3-backwards-combinational-paths-and-f_max)
- [Protocol Specification](#protocol-specification)
  - [Port Interface](#port-interface)
  - [2-Bit Gray-Code Pointer Progression](#2-bit-gray-code-pointer-progression)
  - [Slot Index Mapping](#slot-index-mapping)
  - [Buffer Status (Empty and Full)](#buffer-status-empty-and-full)
- [The 2-Entry Skid Buffer Mechanics](#the-2-entry-skid-buffer-mechanics)
  - [Why Depth = 2 is Required](#why-depth--2-is-required)
  - [Absorbing Registered Round-Trip Delay](#absorbing-registered-round-trip-delay)
- [Cycle-by-Cycle Timing Traces](#cycle-by-cycle-timing-traces)
  - [Trace 1: Sustained 1 Transfer/Cycle Streaming](#trace-1-sustained-1-transfercycle-streaming)
  - [Trace 2: Downstream Backpressure and Skid Stall](#trace-2-downstream-backpressure-and-skid-stall)
  - [Trace 3: Recovery from Stall](#trace-3-recovery-from-stall)
- [Hardware Synthesis Properties](#hardware-synthesis-properties)
- [Interfacing with AXI4-Stream (`valid`/`ready`)](#interfacing-with-axi4-stream-validready)
  - [DDL Buffer to AXI-Stream Producer Adapter](#ddl-buffer-to-axi-stream-producer-adapter)

---

## Motivation: The Pitfalls of `valid`/`ready`

Most RTL streaming interfaces (such as AMBA AXI4-Stream, Avalon-ST, or custom FIFOs) rely on a pair of forward `valid` and backward `ready` wires evaluated in the same clock cycle. A transfer occurs when `valid && ready == 1` on the rising clock edge.

While conceptually simple, unbuffered or naively buffered `valid`/`ready` handshakes introduce severe hardware vulnerabilities.

### 1. Combinational Timing Loops

In standard `valid`/`ready` handshakes, downstream readiness often depends combinationally on upstream validity, or vice-versa:
- A producer may wait to assert `valid` until it knows downstream can accept data.
- A consumer may assert `ready` only when a valid item arrives with a specific command or header.

The AMBA AXI specification mandates: *`VALID` must not depend combinationally on `READY`*. However, when multiple modules are connected across structural netlists—particularly in designs containing feedback paths, arbiters (`merge`), or splitters (`split`)—combinational paths easily loop back:

```
[Module A: valid] ----> [Module B: comb logic] ----> [Module C: ready]
       ^                                                     |
       |-----------------------------------------------------|
                     (Combinational loop closed!)
```

These zero-delay loops may compile without warning in some RTL simulators, but they produce unroutable netlists, synthesis warnings, inferred latches, or intermittent silicon lockups.

### 2. Mutual-Wait Protocol Deadlocks

Handshake bugs occur when two communicating blocks make opposing assumptions about who asserts first:
- **Module A (Producer)**: Waits for `ready == 1` before asserting `valid`.
- **Module B (Consumer)**: Waits for `valid == 1` before asserting `ready`.

Because neither signal asserts independently, the interface enters a permanent **mutual-wait deadlock**. Finding these protocol hangs requires complex assertion checkers and formal verification.

### 3. Backwards Combinational Paths and $F_{\max}$

Even when combinational loops are avoided, backward `ready` signals traverse backwards through every unbuffered block in a pipeline. A stall at the final consumer ripples combinationally back through every upstream stage to disable register clock enables. This long backward path frequently becomes the critical timing path of the entire chip, capping achievable clock frequency ($F_{\max}$).

---

## Protocol Specification

DDL solves these challenges by replacing `valid`/`ready` handshakes with a **Gray-Code Salt Protocol**.

### Port Interface

Every DDL `buffer` channel between two compiled modules flattens into exactly three Verilog ports:

| Port | Direction (Producer $\rightarrow$ Consumer) | Width | Description |
|---|---|---|---|
| `<p>_wsalt` | Output $\rightarrow$ Input | 2 bits | Registered write pointer published by the producer. |
| `<p>_rsalt` | Input $\leftarrow$ Output | 2 bits | Registered read pointer published by the consumer. |
| `<p>_data` | Output $\rightarrow$ Input | $2 \times W$ bits | Packed data bus carrying both FIFO slots: `{slot1, slot0}`. |

Notice that **both pointers originate from flip-flops**. There are zero combinational paths crossing the module boundary in either direction.

### 2-Bit Gray-Code Pointer Progression

The salt pointers cycle through a 2-bit Gray-code sequence:

$$\text{2'b00} \longrightarrow \text{2'b01} \longrightarrow \text{2'b11} \longrightarrow \text{2'b10} \longrightarrow \text{2'b00}$$

On each transfer:
- Exactly **one bit** toggles.
- No multi-bit transition glitches can occur.
- Advancing the pointer *is* the commitment of the transfer.

The next salt state is computed by XORing with either `2'd1` (bit 0) or `2'd2` (bit 1), determined by the current slot index:

```verilog
wire [1:0] salt_toggle = idx ? 2'd2 : 2'd1;
wire [1:0] salt_next   = salt_q ^ salt_toggle;
```

### Slot Index Mapping

Each buffer contains two storage slots (slot 0 and slot 1). The target slot index is derived by XORing the two salt bits:

$$\text{idx} = \text{salt}[0] \oplus \text{salt}[1]$$

| Salt State | $\text{salt}[0] \oplus \text{salt}[1]$ | Active Slot |
|:---:|:---:|:---:|
| `2'b00` | $0 \oplus 0 = 0$ | Slot 0 |
| `2'b01` | $1 \oplus 0 = 1$ | Slot 1 |
| `2'b11` | $1 \oplus 1 = 0$ | Slot 0 |
| `2'b10` | $0 \oplus 1 = 1$ | Slot 1 |

As the salt pointer advances, the active slot cleanly alternates: `0 -> 1 -> 0 -> 1`.

### Buffer Status (Empty and Full)

Because the buffer has depth 2, the relative distance between `wsalt` and `rsalt` indicates buffer fullness:

#### 1. Empty Condition (Consumer Side)

The buffer is empty when the producer's write salt equals the consumer's registered read salt:

```verilog
wire src_empty = (src_wsalt == src_rsalt_q);
```

- When `src_empty == 1`, no valid data is available to consume.
- When `src_empty == 0`, at least one valid item is in the buffer. The consumer extracts its item from the packed bus:

```verilog
wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
wire [W-1:0] item = src_ridx ? src_data[2*W-1:W] : src_data[W-1:0];
```

#### 2. Full Condition (Producer Side)

The buffer is full when the producer's registered write salt is two steps ahead of the consumer's read salt. In a 2-bit Gray-code cycle of length 4, two steps ahead corresponds to bitwise inversion:

```verilog
wire dst_full = (dst_wsalt_q == (~dst_rsalt));
```

- When `dst_full == 0`, the producer has space to write.
- When `dst_full == 1`, both slots are occupied; the producer must stall.

---

## The 2-Entry Skid Buffer Mechanics

### Why Depth = 2 is Required

When handshake pointers are registered at both ends:
1. The consumer's decision to read takes 1 cycle to update `rsalt`.
2. The producer observes the updated `rsalt` on the following cycle.

If a buffer had only **1 entry** (a simple register), the producer would have to wait an extra cycle after every transfer to observe that the consumer emptied the slot. This would throttle maximum throughput to **1 transfer every 2 cycles (50% utilization)**.

With a **2-entry buffer** (primary slot + skid slot):
- The producer can stream continuously at **1 transfer per cycle (100% throughput)**.
- If the consumer stops reading, the producer learns about the stall one cycle later. During that in-flight cycle, the item already committed by the producer safely lands in the second slot (the skid slot).
- Once both slots are filled, `dst_full` evaluates to true, and the producer halts before any data is overwritten.

---

## Cycle-by-Cycle Timing Traces

### Trace 1: Sustained 1 Transfer/Cycle Streaming

Both producer and consumer transfer on every clock edge. Pointers advance in lockstep:

```
Cycle          :    0      1      2      3      4      5
clk            :  __/~~\__/~~\__/~~\__/~~\__/~~\__/~~\
wsalt_q        :    00     01     11     10     00     01
rsalt_q        :    00     01     11     10     00     01
Item Written   :    D0     D1     D2     D3     D4     D5
Slot Written   :     0      1      0      1      0      1
Empty? (w==r)  :    No*    No*    No*    No*    No*    No*  (active stream)
Full?  (w==~r) :    No     No     No     No     No     No
```

*Note: In continuous streaming, writes and reads advance in the same cycle, maintaining an equilibrium occupancy of 1 item with zero pipeline stalls.*

### Trace 2: Downstream Backpressure and Skid Stall

The consumer stalls (stops reading). The producer commits one final in-flight transfer into the skid slot before pausing:

```
Cycle          :    0      1      2      3      4
clk            :  __/~~\__/~~\__/~~\__/~~\__/~~\
Consumer Read  :   Active  STALL  STALL  STALL  STALL
rsalt_q        :    00     00     00     00     00   (held at 00)
-------------------------------------------------------
Producer State :
wsalt_q        :    00     01     11     11     11   (stalls at 11!)
dst_full       :     0      0      1      1      1   (asserts at cycle 2)
Slot Written   :     -    Slot 0 Slot 1   -      -
Occupancy      :     0      1      2      2      2   (Buffer FULL)
```

- **Cycle 1**: Producer writes item $A$ to Slot 0 and advances `wsalt_q` to `01`. `dst_full` is still `0` because `~rsalt` is `11`.
- **Cycle 2**: Producer writes item $B$ to Slot 1 (the skid slot) and advances `wsalt_q` to `11`.
- **Cycle 3**: Now `wsalt_q (11) == ~rsalt (11)`. `dst_full` evaluates to `1`. The producer pauses. Both items $A$ and $B$ are safely preserved in slots 0 and 1.

### Trace 3: Recovery from Stall

The consumer resumes reading. As soon as `rsalt_q` advances, `dst_full` deasserts on the very next cycle, allowing the producer to resume:

```
Cycle          :    0      1      2      3
clk            :  __/~~\__/~~\__/~~\__/~~\
Consumer Read  :   READ   READ   READ   Active
rsalt_q        :    00     01     11     10
-------------------------------------------------------
wsalt_q        :    11     11     10     00
dst_full       :     1      0      0      0   (deasserts at cycle 1!)
Producer Push  :  STALLED RESUMED Active Active
```

---

## The Protocol Is Single-Clock

Everything above assumes **one clock**. The guarantee that makes the protocol
work — a slot is stable for as long as it is offered — rests on the producer and
the consumer agreeing about when a cycle is, and two clocks do not agree about
that.

Gray coding protects the *pointer*: one bit changes per step, so a salt sampled
mid-transition reads as the old value or the new one and never a mixture.
Nothing protects the decision made from it. The `empty` flag is a combinational
function of the asynchronous incoming pointer `wsalt`. Because that flag fans out
to multiple registers in the receiving domain, subtle routing skew causes registers
to sample differing values within the exact same clock edge — one register advances
the read pointer while another fails to capture the item.

This is not theoretical. Driven across two PLL domains on a Tang Nano 9K, the
compiler's own emitted link corrupted **100% of items at 54 million errors per
second**, while the same design on one clock ran clean. The experiment is
documented in [`cdc-demo/`](../cdc-demo/README.md). The remedy is to isolate the
boundary with an asynchronous FIFO: wire [`lib/ddl_cdc_fifo.v`](../lib/ddl_cdc_fifo.v)
by hand, or let the compiler automate it via `--async-export`. See [**Clock
Domains**](clock-domains.md) and [**Guide 6: Clock Domain
Crossings**](guides/6-clock-domain-crossings.md).

## Hardware Synthesis Properties

1. **Pure Register-to-Register Paths**: Every inter-module wire (`wsalt`, `rsalt`, `data`) connects a flip-flop directly to downstream logic. Combinational paths never cross module boundaries.
2. **Minimal Silicon Footprint**:
   - Pointers: Exactly two 2-bit registers per buffer (`wsalt_q`, `rsalt_q`).
   - Storage: Two data registers (`e0`, `e1`) of width $W$.
   - Logic: A 2-bit XOR gate, a 2-bit equality comparator, and a 2-to-1 data multiplexer.
3. **No Glitches**: Because Gray-code pointers advance by toggling exactly one bit per transfer, race conditions and intermediate invalid pointer states are physically eliminated.

---

## Interfacing with AXI4-Stream (`valid`/`ready`)

You do not write this adapter. The compiler writes it.

An `extern`, or a module chosen as an export target, presents a FIFO — so bridging from there to AXI4-Stream is a renaming, with no state and no arithmetic:

```verilog
// producing side
assign m_axis_tvalid = dst_has_data;
assign m_axis_tdata  = dst_data_read_out;
assign dst_drop_item = m_axis_tvalid && m_axis_tready;

// consuming side
assign s_axis_tready     = src_can_receive;
assign src_data_write_in = s_axis_tdata;
assign src_receive_en    = s_axis_tvalid && s_axis_tready;
```

The property this document exists to establish survives the trip: `dst_has_data` is a function of the module's registered pointers and never of `m_axis_tready`, so there is no combinational path from `ready` back to `valid` — the loop AXI forbids, and the one [Motivation](#motivation-the-pitfalls-of-validready) opens with.

The translation between the pointers and that FIFO lives in modules the compiler emits — `ddl_salt_to_rport_<W>` and `ddl_wport_to_salt_<W>`, plus the two mirrors of them that drive an `extern` — written in `src/ir_adapt.rs`. Each is one gray-code pointer and a handful of gates, built on the same helpers `@merge` and `@split` use, and simulated against the FIFO contract in `tests/adapters.rs`.

For a full specification of the Show-Ahead boundary contract and adapter circuits, see [**FIFO Boundary Adapters & Export Architecture**](fifo-boundaries-and-export.md).
