# Guide 3: Interfacing DDL with Existing Verilog, AXI-Stream, and FPGA Pins

This guide explains how to integrate DDL-generated modules into existing FPGA or ASIC systems, connect directly to physical chip pins, and bridge between DDL channels and standard AXI4-Stream buses.

---

## What You Will Learn

- What a DDL module looks like from the outside, and how to drive it.
- How to instantiate external or vendor Verilog IP inside a DDL `graph` using `extern`.
- How to drive physical FPGA pins using `wire`.
- How to bridge to **AMBA AXI4-Stream** — which, at the boundary, is renaming.

---

## 1. The boundary is a FIFO

Internally, DDL connects two compiled modules with a gray-code pointer protocol (see [The Gray-Code Salt Protocol](../salt-protocol.md)). That protocol is between two modules the compiler wrote, and it never reaches you.

Every boundary you wire up by hand — an `extern`, or the module you asked the compiler to export — presents an ordinary FIFO instead. For a pipe `p` carrying `W` bits:

| the module **consumes** `p` (`buffer in`) | dir | meaning |
| --- | --- | --- |
| `p_can_receive` | output | there is room |
| `p_receive_en` | input | write `p_data_write_in` this cycle |
| `p_data_write_in [W-1:0]` | input | the item |

| the module **produces** `p` (`buffer out`) | dir | meaning |
| --- | --- | --- |
| `p_has_data` | output | an item is available |
| `p_drop_item` | input | I took it; advance |
| `p_data_read_out [W-1:0]` | output | the item |

That is `!full`/`wr_en`/`wr_data` and `!empty`/`rd_en`/`rd_data`, with first-word-fall-through: the item is on `p_data_read_out` while `p_has_data` is high, and stays there, unchanged, until you raise `p_drop_item`. Raise `p_receive_en` only while `p_can_receive` is high, and `p_drop_item` only while `p_has_data` is high — the same rule any FIFO has.

### Driving one

```verilog
// Offer whenever it has room; take whenever it has one.
mul3 u_dut (
  .clk               (clk),
  .rst_n             (rst_n),
  .src_can_receive   (src_can_receive),
  .src_receive_en    (src_can_receive && have_work),
  .src_data_write_in (sample),
  .dst_has_data      (dst_has_data),
  .dst_drop_item     (dst_has_data),
  .dst_data_read_out (result)
);
```

`tests/gowin/sequence_top.v` is this pattern against real hardware.

### Choosing what gets exported

The compiler works out which module the build is for: a **root** is one that nothing else instantiates or calls, counting only declarations in the files you named on the command line. With one root, no flag is needed. With several, it names them and asks:

```bash
ddl build src.ddl -o src.v --export top          # this one presents a FIFO
ddl build src.ddl -o src.v --export a,b,c        # several, comma-separated
ddl build src.ddl -o src.v --bare-export top     # keep the raw salt ports
```

The file it writes is the target and everything the target uses, transitively — not every declaration it compiled. Two unrelated pipelines in one source produce two different files depending on which you ask for, and an `import` you never call contributes nothing to either.

The exported module's logic keeps its shape and takes the name `<name>_core`. The module under the original name is the wrapper: the FIFO ports, one compiler-written adapter per pipe, and one instance of the core. Use `--bare-export` if you would rather speak the pointer protocol directly — it emits exactly what the lowering produces.

---

## 2. Instantiating External Verilog Modules (`extern`)

To incorporate vendor IP (PLLs, memory controllers, Ethernet MACs) or pre-existing hand-written Verilog into a DDL system, declare it with `extern`:

```ddl
extern psram_ctrl (cmd: buffer in u32, rsp: buffer out u32)

sequence worker (src: buffer in u32, dst: buffer out u32)
  let a = @rcv(src)
  |||
  @send(dst, a + 32'd1)

graph top_system (host_in: buffer in u32, host_out: buffer out u32)
  let to_psram: buffer u32
  let from_psram: buffer u32

  worker(host_in, to_psram)
  psram_ctrl(to_psram, from_psram)
  worker(from_psram, host_out)
```

The connections are type-checked, and no body is generated: your synthesizer resolves `psram_ctrl` against your own sources. What it has to match is a FIFO on each pipe:

```verilog
module psram_ctrl (
  input         clk,
  input         rst_n,
  output        cmd_can_receive,
  input         cmd_receive_en,
  input  [31:0] cmd_data_write_in,
  output        rsp_has_data,
  input         rsp_drop_item,
  output [31:0] rsp_data_read_out
);
```

Inside `top_system`, the compiler places an adapter between each internal pipe and the extern — `ddl_salt_to_wport_32` on the way in, `ddl_rport_to_salt_32` on the way out. They are modules it writes, in the same sense `@merge` and `@split` are.

`examples/fanout.ddl` instantiates one, and `examples/externs.v` is the hand-written counterpart: a stub that accepts everything, and now four lines of it.

---

## 3. Physical Hardware Pins (`wire`)

A pin cannot be backpressured — an input changes whether your logic is ready or not — so it is not a pipe. In DDL a bare signal is a **`wire`**, and because there is nothing on it to wait for, it is legal only where nothing waits: an `extern` or a `graph`.

```ddl
extern pll (locked: wire out u1, cfg: buffer in u32)

graph board (cfg: buffer in u32, lock_led: wire out u1)
  pll(lock_led, cfg)
```

- `x: wire in T` emits one input `x` of `T`'s width; `y: wire out T` emits one output. There is no enable beside it — a wire is the signal and nothing else.
- A graph routes a wire straight through to its own boundary, and checks that exactly one instance drives each output. Two drivers on one wire is not a race the protocol arbitrates; it is an `x`.
- A `process` or `sequence` cannot take one. Their parameters are pipes and constant parameters. This is the line `desc.md` draws — compute in DDL, I/O in Verilog — and it is what stops a body from hand-rolling a protocol over a raw wire.

So pin-level logic lives in an `extern` you write, and it talks to your DDL datapath through a FIFO.

---

## 4. Bridging to AMBA AXI4-Stream (`valid`/`ready`)

Many SoC interconnects use `valid`/`ready`. Against the FIFO boundary this is a **renaming**, with no state and no arithmetic — the flag is `valid`, the enable is `valid && ready`, and the data is the data.

### DDL producer → AXI4-Stream master

For an exported module's `buffer out`:

```verilog
assign m_axis_tvalid = dst_has_data;
assign m_axis_tdata  = dst_data_read_out;
assign dst_drop_item = m_axis_tvalid && m_axis_tready;
```

### AXI4-Stream slave → DDL consumer

For an exported module's `buffer in`:

```verilog
assign s_axis_tready    = src_can_receive;
assign src_data_write_in = s_axis_tdata;
assign src_receive_en   = s_axis_tvalid && s_axis_tready;
```

Both directions are legal AXI: `tvalid` here is a function of the module's registered pointer and never of `tready`, so there is no combinational path from `ready` to `valid` — the loop AXI forbids and the one the pointer protocol was built to avoid.

`tlast`, `tkeep` and `tuser` are outside what a DDL pipe carries. Put them in the payload type if the datapath needs them, and drive them alongside.

### System Integration Summary

With those six assignments at your SoC ingress and egress, the entire core datapath of your accelerator operates within DDL's loop-free dataflow domain, and nothing you wrote had to implement a handshake.
