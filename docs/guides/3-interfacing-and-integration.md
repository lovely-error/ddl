# Guide 3: Interfacing DDL with Existing Verilog, AXI-Stream, and FPGA Pins

This guide explains how to integrate DDL-generated modules into existing FPGA or ASIC systems, connect directly to physical chip pins, and bridge between DDL channels and standard AXI4-Stream buses.

---

## What You Will Learn

- How to drive physical FPGA pins (LEDs, buttons, streaming wires) using `port`.
- How to instantiate external or vendor Verilog IP inside a DDL `graph` using `extern`.
- How to bridge DDL's Gray-Code salt protocol to standard **AMBA AXI4-Stream** master and slave interfaces.

---

## 1. Physical Hardware Pins (`port`)

When communicating with external board hardware (e.g. GPIO pins, pushbuttons, LEDs, UART wires), the other side cannot be backpressured: an input pin changes whether your logic is ready or not.

In DDL, direct wire connections are declared using **`port`**:

```ddl
process blinker (btn: port in u1, led: port out u1)
  var state: u1 = 1'b0
  loop
    let b = @rcv(btn)
    if b then
      state = ~state
    @send(led, state)
```

### Generated Port Interfaces:
A `port` emits raw data and enable signals without FIFO storage or salt pointers:
- `btn: port in u1` becomes input wires: `btn` and `btn_en`.
- `led: port out u1` becomes output wires: `led` and `led_en`.

If `btn_en` is tied high (e.g. `assign btn_en = 1'b1;`), the process reads the button state every cycle. When a process consists entirely of `port` interfaces, it compiles into a standard synchronous Verilog module.

---

## 2. Instantiating External Verilog Modules (`extern`)

To incorporate vendor IP (e.g., PLLs, memory controllers, Ethernet MACs) or pre-existing hand-written Verilog modules into a DDL system, declare them with `extern`:

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

### Synthesis Behavior:
- The DDL compiler type-checks all channel connections between `worker` and `psram_ctrl`.
- In the generated `top_system.v`, the compiler instantiates `psram_ctrl` and automatically connects its `<p>_wsalt`, `<p>_rsalt`, and `<p>_data` ports to internal wires.
- No dummy body is generated for `psram_ctrl`, allowing your synthesizer to resolve it against your vendor IP or external Verilog sources.

---

## 3. Bridging to AMBA AXI4-Stream (`valid`/`ready`)

Many SoC interconnects (e.g., Xilinx AXI-Stream, Intel Avalon-ST) use traditional `valid`/`ready` handshakes. Bridging between DDL and AXI-Stream requires lightweight protocol adapters.

### Adapter 1: DDL Buffer $\rightarrow$ AXI4-Stream Master

Use this module when a DDL block produces data that must be streamed into an external AXI-Stream slave IP:

```verilog
module ddl_to_axis #(
    parameter WIDTH = 32
)(
    input  wire                 clk,
    input  wire                 rst_n,

    // DDL Buffer Interface (Consumer side)
    input  wire [1:0]           ddl_wsalt,
    output wire [1:0]           ddl_rsalt,
    input  wire [2*WIDTH-1:0]   ddl_data,

    // AXI4-Stream Master Interface
    output wire                 m_axis_tvalid,
    input  wire                 m_axis_tready,
    output wire [WIDTH-1:0]     m_axis_tdata
);

  reg [1:0] rsalt_q;

  // Buffer is not empty when write salt != read salt
  wire empty = (ddl_wsalt == rsalt_q);
  assign m_axis_tvalid = !empty;

  // Select slot based on current read salt
  wire ridx = rsalt_q[0] ^ rsalt_q[1];
  assign m_axis_tdata = ridx ? ddl_data[2*WIDTH-1:WIDTH] : ddl_data[WIDTH-1:0];

  // Transfer commits when both valid and ready are high
  wire transfer = m_axis_tvalid && m_axis_tready;
  wire [1:0] toggle = ridx ? 2'd2 : 2'd1;

  assign ddl_rsalt = rsalt_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      rsalt_q <= 2'b00;
    end else if (transfer) begin
      rsalt_q <= rsalt_q ^ toggle;
    end
  end

endmodule
```

---

### Adapter 2: AXI4-Stream Slave $\rightarrow$ DDL Buffer

Use this module when an external AXI-Stream master IP streams data into a downstream DDL block:

```verilog
module axis_to_ddl #(
    parameter WIDTH = 32
)(
    input  wire                 clk,
    input  wire                 rst_n,

    // AXI4-Stream Slave Interface
    input  wire                 s_axis_tvalid,
    output wire                 s_axis_tready,
    input  wire [WIDTH-1:0]     s_axis_tdata,

    // DDL Buffer Interface (Producer side)
    output wire [1:0]           ddl_wsalt,
    input  wire [1:0]           ddl_rsalt,
    output wire [2*WIDTH-1:0]   ddl_data
);

  reg [1:0]         wsalt_q;
  reg [WIDTH-1:0]   slot0;
  reg [WIDTH-1:0]   slot1;

  // Buffer is full when wsalt_q == ~ddl_rsalt
  wire full = (wsalt_q == (~ddl_rsalt));
  assign s_axis_tready = !full;

  wire widx = wsalt_q[0] ^ wsalt_q[1];
  wire transfer = s_axis_tvalid && s_axis_tready;
  wire [1:0] toggle = widx ? 2'd2 : 2'd1;

  assign ddl_wsalt = wsalt_q;
  assign ddl_data  = {slot1, slot0};

  always @(posedge clk) begin
    if (!rst_n) begin
      wsalt_q <= 2'b00;
      slot0   <= {WIDTH{1'b0}};
      slot1   <= {WIDTH{1'b0}};
    end else if (transfer) begin
      wsalt_q <= wsalt_q ^ toggle;
      if (!widx)
        slot0 <= s_axis_tdata;
      else
        slot1 <= s_axis_tdata;
    end
  end

endmodule
```

### System Integration Summary:
By placing `axis_to_ddl` at your SoC ingress and `ddl_to_axis` at your egress, the entire core datapath of your accelerator operates within DDL's mathematically guaranteed, loop-free dataflow domain.
