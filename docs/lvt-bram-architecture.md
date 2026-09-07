# Multi-Write BRAM Synthesis & The Live Value Table (LVT)

This document describes how DDL synthesizes multi-write block memories on FPGAs using the **Live Value Table (`--lvt-bram`)** architecture.

---

## Table of Contents

- [The Multi-Write Memory Problem on FPGAs](#the-multi-write-memory-problem-on-fpgas)
  - [ASIC vs. FPGA Memory Primitives](#asic-vs-fpga-memory-primitives)
  - [The Vendor Synthesis Trap (Flip-Flop Explosion)](#the-vendor-synthesis-trap-flip-flop-explosion)
- [The Live Value Table (LVT) Architecture](#the-live-value-table-lvt-architecture)
  - [Bank Partitioning](#bank-partitioning)
  - [The Live Value Table](#the-live-value-table)
- [Hardware Implementation & Mechanics](#hardware-implementation--mechanics)
  - [Write Path](#write-path)
  - [Read Path & Registered Multiplexing](#read-path--registered-multiplexing)
  - [Reset and Initialization](#reset-and-initialization)
- [Benchmark Results: Gowin GW1NR-9C](#benchmark-results-gowin-gw1nr-9c)
- [When to Use `--lvt-bram`](#when-to-use---lvt-bram)

---

## The Multi-Write Memory Problem on FPGAs

Hardware designs frequently require memories with multiple concurrent write ports. Common examples include:
- Multi-threaded register files (e.g. 2 write ports for ALU writeback and memory load).
- Multi-channel DMA buffers.
- Parallel pipeline stages updating shared tracking tables.

In DDL, declaring multiple writes in a single cycle or within a single stage naturally asks for multiple write ports:

```ddl
#[impl(bram)]
var table: [u32; 256]

// Two sequential writes occurring in the same clock cycle:
table[addr_a] = data_a
table[addr_b] = data_b
```

### ASIC vs. FPGA Memory Primitives

- **ASICs**: Foundry memory compilers can synthesize custom multi-port SRAM cells (e.g. 2W/4R) with dedicated bitlines and wordlines for each port.
- **FPGAs**: Physical FPGA silicon contains hard Block RAM primitives (e.g. Xilinx Block RAM, Gowin Semi-Dual-Port Block RAM / SDPB). These primitives almost universally provide **at most one write port per physical block** (or two write ports under strict aspect-ratio and contention constraints).

### The Vendor Synthesis Trap (Flip-Flop Explosion)

When an FPGA synthesis tool (such as GowinSynthesis or Quartus) encounters HDL asking for two unconstrained write ports on a large array, it cannot infer a physical Block RAM cell.

Instead of issuing a clear diagnostic, the synthesizer silently falls back to **distributed logic**:
- Every memory bit becomes a discrete flip-flop.
- A 256×32-bit array requires $256 \times 32 = 8{,}192$ flip-flops, plus massive multiplexer trees for address decoding.
- On medium or low-cost FPGAs (such as the Gowin GW1NR-9C with 8,640 logic cells), this single array consumes nearly the entire chip's logic resources, causing physical place-and-route failure.

---

## The Live Value Table (LVT) Architecture

DDL solves this problem at the compiler level via the `--lvt-bram` flag:

```bash
ddl build src/top.ddl -o build/top.v --lvt-bram
```

When enabled, DDL replaces the impossible multi-write cell with an **LVT-coordinated multi-bank array**:

```
Write Port 0 ----> [ BRAM Bank 0 (1W/1R) ] ----\
Write Port 1 ----> [ BRAM Bank 1 (1W/1R) ] -----\
                                                 +---> [ Mux ] ---> Read Data
Write Port 0/1 ---> [ Live Value Table (LVT) ] --/
                    (Distributed LUTRAM)
```

### Bank Partitioning

- For $N$ concurrent write ports, DDL allocates **$N$ independent single-write Block RAM banks**.
- Each bank has the full depth ($D$) and width ($W$) of the declared memory.
- Write port $k$ writes *exclusively* to Bank $k$.

### The Live Value Table

Because data written to address $A$ on Port 1 is stored in Bank 1, while data written to address $B$ on Port 0 is stored in Bank 0, read operations need a mechanism to know which bank holds the newest value for any given address.

The **Live Value Table (LVT)** is a small tracking memory:
- **Depth**: Same as the data memory ($D$).
- **Width**: $\lceil \log_2 N \rceil$ bits (e.g., 1 bit for 2 banks, 2 bits for 3–4 banks).
- **Implementation**: Synthesized in distributed LUTRAM so that it can be cleared or updated with multi-port write access at negligible logic cost.

---

## Hardware Implementation & Mechanics

### Write Path

Whenever write port $k$ executes a write to address `addr`:
1. The payload `din` is written to `bank_k[addr]`.
2. The bank index $k$ is written into `lvt[addr]`.

```verilog
always @(posedge clk) begin
  if (we0) begin
    table_bank0[addr0] <= din0;
    table_lvt[addr0]   <= 1'd0; // Port 0 wrote this address
  end
  if (we1) begin
    table_bank1[addr1] <= din1;
    table_lvt[addr1]   <= 1'd1; // Port 1 wrote this address
  end
end
```

### Read Path & Registered Multiplexing

Block RAM reads have a 1-cycle latency. To avoid adding combinational delay after the BRAM output, the LVT lookup is registered in parallel with the BRAM read:

1. The read address `raddr` is applied simultaneously to all data banks and to `table_lvt`.
2. On the clock edge:
   - Bank 0 latches output `table_b0_q`.
   - Bank 1 latches output `table_b1_q`.
   - The LVT latches the bank pointer into `table_lvt_q`.
3. In the subsequent cycle, the active bank data is selected:
   ```verilog
   wire [31:0] read_data = (table_lvt_q == 1'd0) ? table_b0_q : table_b1_q;
   ```

Because selection occurs *after* the register edge on already-registered data, timing closure is preserved with zero cycle latency overhead.

### Reset and Initialization

During global reset, the data banks do not need clearing (Block RAM initialization is handled by bitstream loading). The LVT, being small, is initialized with a reset loop:

```verilog
always @(posedge clk) begin
  if (!rst_n) begin
    for (i = 0; i < 256; i = i + 1)
      table_lvt[i] <= 1'd0;
  end
  ...
end
```

---

## Benchmark Results: Gowin GW1NR-9C

The effectiveness of `--lvt-bram` is measured directly on a Gowin GW1NR-9C (Tang Nano 9K) in [`examples/rf_lvt.ddl`](examples/rf_lvt.ddl):

| Configuration | FPGA Synthesis Result (GowinSynthesis) | Resource Utilization |
|---|---|---|
| **Without `--lvt-bram`** | **Fails Place & Route** | **> 8,000 Flip-Flops** (LUTRAM exhausted) |
| **With `--lvt-bram`** | **Fits Comfortably** | **4 Semi-Dual-Port BRAMs (SDPB)** + minimal LUTs |

On target hardware, this is not merely an optimization—it is the difference between a design fitting on chip versus failing synthesis entirely.

---

## When to Use `--lvt-bram`

- **Use `--lvt-bram`** when targeting FPGAs (Gowin, Xilinx, Efinix, Lattice, Intel) and your design contains memories with multiple concurrent write ports.
- **Do not use `--lvt-bram`** when targeting ASICs with custom multi-port SRAM compilers, where the native multi-write macro cell is smaller and more power-efficient than multiple partitioned banks.
