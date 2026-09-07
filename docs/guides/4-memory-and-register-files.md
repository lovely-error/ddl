# Guide 4: Memory Patterns: ROMs, Block RAMs, and Multi-Port Register Files

This guide explains how memory arrays work in DDL, how to choose between distributed RAM (`lutram`) and block memory (`bram`), and how to synthesize multi-write register files on FPGAs without logic explosion.

---

## What You Will Learn

- The timing differences between `lutram` (asynchronous read) and `bram` (synchronous read).
- How to declare memory arrays using implementation attributes (`#[impl(...)]`).
- How synchronous Block RAM reads align with pipeline stage cuts (`|||`) at zero register overhead.
- How to synthesize dual-write CPU register files on FPGAs using `--lvt-bram`.
- Memory consistency and same-cycle hazard forwarding rules.

---

## 1. `lutram` vs. `bram`: Choosing the Right Memory

DDL provides two primary hardware memory primitives:

| Memory Attribute | Hardware Type | Read Latency | Synthesis Target | Typical Use Case |
|---|---|---|---|---|
| `#[impl(lutram)]` | Distributed RAM | **0 cycles** (Asynchronous) | FPGA LUTs (e.g. RAM16X1D) | Small buffers (<64 entries), register files requiring same-cycle read & forward. |
| `#[impl(bram)]` | Dedicated Block RAM | **1 cycle** (Synchronous) | Hard BRAM tiles (e.g. SDPB) | Dense storage (tables, caches, buffers >64 entries). |

### Where Does the Read Cycle Go?
A Block RAM read requires one clock cycle. In DDL, that cycle must correspond to a physical boundary:
- In a **`process`**, a `bram` read occupies a dedicated FSM state.
- In a **`sequence`**, a `bram` read must be separated by a stage cut (`|||`).

---

## 2. Pipelined Lookup Table with Block RAM

When implementing a lookup table or coefficient ROM inside a `sequence`, DDL aligns the 1-cycle BRAM read latency with the stage cut:

```ddl
sequence lut_pipeline (src: buffer in u8, dst: buffer out u16)
  var table: #[impl(bram)] [u16; 256]

  let idx = @rcv(src)
  |||
  let val = table[idx]
  |||
  @send(dst, val)
```

### Hardware Lowering Optimization:
1. In Stage 0, the address `idx` is presented to the BRAM address inputs.
2. The stage cut `|||` coincides with the memory's clocked output register.
3. In Stage 1, `val` maps directly to `table_q` (the internal output register of the Block RAM tile).
4. **Zero extra flip-flops**: The hardware Block RAM's built-in output register acts as the pipeline register.

---

## 3. Multi-Write CPU Register Files (`--lvt-bram`)

Modern processors frequently require register files with multiple write ports (e.g. 2 write ports for superscalar writeback or simultaneous ALU and load operations).

Declaring two writes in one state creates a multi-write RAM:

```ddl
struct rf_proc_req
  wa0: u8
  wd0: u32
  wa1: u8
  wd1: u32
  ra0: u8
  ra1: u8

process rf_lvt_proc (cmd: buffer in rf_proc_req, rd: buffer out u64)
  var vals: #[impl(bram)] [u32; 256]

  loop
    let q = @rcv(cmd)

    -- Two writes in one cycle = two write ports
    vals[q.wa0] = q.wd0
    vals[q.wa1] = q.wd1

    -- Two reads, multiplexed onto the memory read port
    let x = vals[q.ra0]
    let y = vals[q.ra1]

    @send(rd, {x, y})
```

### Compiling with `--lvt-bram`

Physical FPGA block RAM tiles (such as on the Gowin GW1NR-9C) only have one write port. If you compile this without `--lvt-bram`, synthesis tools will fall back to discrete flip-flops, consuming over 8,000 flip-flops and failing place-and-route.

To synthesize multi-write arrays into true Block RAMs, pass `--lvt-bram`:

```bash
ddl build rf_example.ddl -o rf_example.v --lvt-bram
```

### How It Synthesizes:
- DDL instantiates **two independent single-write BRAM banks** (one for write port 0, one for write port 1).
- It instantiates a **Live Value Table (LVT)** in distributed LUTRAM that records which bank holds the newest write for each address.
- Reads access both banks in parallel, using the LVT pointer to select the live value.
- On a Gowin Tang Nano 9K, this reduces resource usage from >8,000 flip-flops to just **2 SDPB block RAMs** and minimal LUTs.

---

## 4. Memory Consistency & Forwarding Rules

1. **Single-Stage Ownership in Pipelines**: A memory written within a `sequence` must be read and written in a **single stage**. Dividing reads and writes across stages is rejected at compile time because different pipeline stages process items from different time steps.
2. **Automatic Same-Cycle Forwarding**: If a pipeline stage reads and writes to the same memory, DDL synthesizes collision-detection logic to forward write data directly to the read result when addresses match, guaranteeing that reads observe earlier writes in source order.
3. **No Resets on BRAM**: A Block RAM primitive cannot be cleared with an asynchronous or synchronous reset loop (clearing 1024 words in one cycle requires 1024 write ports). DDL rejects attempts to attach resets to `bram` arrays.
