# Guide 5: Simulation, Verification, and Build Workflows

This guide explains how to verify DDL designs using embedded simulation assertions, write SystemVerilog testbenches that interact with DDL's salt protocol, and automate verification using Verilator and Makefiles.

---

## What You Will Learn

- How to write edge-triggered simulation assertions in DDL using `@assert` and `@fatal`.
- How to write a SystemVerilog testbench that drives and monitors DDL's Gray-Code salt channels.
- How to test backpressure and pipeline stalls in simulation.
- How to use Verilator as an automated lint oracle.
- How to integrate DDL compilation into a Makefile or CI/CD workflow.

---

## 1. Simulation Assertions (`@assert` and `@fatal`)

DDL includes built-in simulation checking intrinsics:

```ddl
sequence seq_assert (src: buffer in u16, dst: buffer out u16)
  let a = @rcv(src)
  @assert(a != 16'd0)
  |||
  @assert(a != 16'd0)
  @send(dst, a)

process proc_assert (src: buffer in u8, dst: buffer out u8)
  loop
    let x = @rcv(src)
    @assert(x != 8'd0, "zero not allowed")
    @send(dst, x)
```

### Hardware Lowering Properties:
1. **Simulation-Only**: Assertions are emitted inside `` `ifdef SIMULATION `` blocks. Physical synthesizers strip them completely.
2. **Reset-Safe**: Checks are automatically gated on `if (rst_n)`. They never fire spuriously during hardware reset.
3. **Execution-Scoped**:
   - In a `process`, assertions evaluate only when their specific state and branch fire (`fire_s0`). Idle cycles do not trigger assertions on stale registers.
   - In a `sequence`, assertions evaluate only when the stage holds a valid item and the pipeline advances (`shift & v0`). Pipeline bubbles never trigger failures.

---

## 2. Writing a SystemVerilog Testbench for DDL Modules

Because DDL channels use registered Gray-code pointers (`_wsalt`, `_rsalt`, `_data`), a testbench drives them by toggling salts.

Here is a complete reusable SystemVerilog testbench template for testing any DDL buffer:

```systemverilog
`timescale 1ns/1ps

module tb_ddl_example;

  reg clk;
  reg rst_n;

  // DUT Interface signals
  reg  [1:0]  src_wsalt;
  wire [1:0]  src_rsalt;
  reg  [63:0] src_data;

  wire [1:0]  dst_wsalt;
  reg  [1:0]  dst_rsalt;
  wire [63:0] dst_data;

  // Clock generation
  initial clk = 0;
  always #5 clk = ~clk;

  // DUT Instantiation
  worker dut (
    .clk       (clk),
    .rst_n     (rst_n),
    .src_wsalt (src_wsalt),
    .src_rsalt (src_rsalt),
    .src_data  (src_data),
    .dst_wsalt (dst_wsalt),
    .dst_rsalt (dst_rsalt),
    .dst_data  (dst_data)
  );

  // -------------------------------------------------------------
  // Testbench Task: Send one item into DDL buffer
  // -------------------------------------------------------------
  task send_item(input [31:0] val);
    begin
      // Wait until DUT's input buffer has space: wsalt != ~rsalt
      while (src_wsalt == (~src_rsalt)) @(posedge clk);

      // Write data to the slot indicated by current wsalt
      if (src_wsalt[0] ^ src_wsalt[1])
        src_data[63:32] = val; // Slot 1
      else
        src_data[31:0]  = val; // Slot 0

      // Toggle wsalt to commit transfer
      if (src_wsalt[0] ^ src_wsalt[1])
        src_wsalt <= src_wsalt ^ 2'd2;
      else
        src_wsalt <= src_wsalt ^ 2'd1;

      @(posedge clk);
    end
  endtask

  // -------------------------------------------------------------
  // Testbench Task: Receive one item from DDL buffer
  // -------------------------------------------------------------
  task receive_item(output [31:0] val);
    begin
      // Wait until DUT's output buffer is not empty: wsalt != rsalt
      while (dst_wsalt == dst_rsalt) @(posedge clk);

      // Read from active slot
      if (dst_rsalt[0] ^ dst_rsalt[1])
        val = dst_data[63:32];
      else
        val = dst_data[31:0];

      // Toggle rsalt to commit receive
      if (dst_rsalt[0] ^ dst_rsalt[1])
        dst_rsalt <= dst_rsalt ^ 2'd2;
      else
        dst_rsalt <= dst_rsalt ^ 2'd1;

      @(posedge clk);
    end
  endtask

  // -------------------------------------------------------------
  // Test Stimulus
  // -------------------------------------------------------------
  initial begin
    reg [31:0] res;

    // Reset sequence
    rst_n = 0;
    src_wsalt = 2'b00;
    dst_rsalt = 2'b00;
    src_data  = 64'd0;
    #20;
    rst_n = 1;
    @(posedge clk);

    // Stream 10 items
    for (int i = 1; i <= 10; i++) begin
      send_item(i * 10);
    end

    // Receive 10 results
    for (int i = 1; i <= 10; i++) begin
      receive_item(res);
      $display("Received result: %0d", res);
    end

    $display("TEST PASSED");
    $finish;
  end

endmodule
```

### Testing Backpressure:
To simulate downstream stalls, simply delay calling `receive_item(...)`. The DUT's output buffer will fill its primary slot and skid slot, after which the DUT will cleanly stall its upstream pipeline without losing data.

---

## 3. Automated Netlist Linting with Verilator

Before running FPGA synthesis, lint your emitted Verilog using Verilator:

```bash
verilator --lint-only -Wall -DSIMULATION my_design.v
```

This verifies:
- **No inferred latches** (all `if`/`else` paths assign outputs).
- **No combinational loops**.
- **No width or truncation mismatches**.
- **No multi-driven nets**.

---

## 4. Build Automation with Make

Add a `Makefile` to automatically rebuild generated Verilog and verify against checked-in sources:

```makefile
DDL = ./target/debug/ddl
SOURCES = $(wildcard src/*.ddl)
VERILOG = $(SOURCES:src/%.ddl=build/%.v)

all: $(VERILOG)

build/%.v: src/%.ddl
	@mkdir -p build
	$(DDL) build $< -o $@

# Verify checked-in Verilog without modifying files
check:
	@for f in $(SOURCES); do \
		out="build/$$(basename $$f .ddl).v"; \
		$(DDL) build $$f -o $$out --check || exit 1; \
	done
	@echo "All generated Verilog files match source!"

clean:
	rm -rf build
```
