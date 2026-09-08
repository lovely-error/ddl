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

An exported DDL module presents a FIFO on each pipe, so a testbench drives it the way it would drive any FIFO. For a pipe `p`: `p_can_receive` / `p_receive_en` / `p_data_write_in` going in, and `p_has_data` / `p_drop_item` / `p_data_read_out` coming out.

Two rules, and they are the only ones:

- Raise `p_receive_en` only while `p_can_receive` is high, and `p_drop_item` only while `p_has_data` is high.
- While `p_has_data` is high and `p_drop_item` is low, `p_data_read_out` holds the same item. You may look at it for as long as you like before taking it.

```systemverilog
`timescale 1ns/1ps

module tb_ddl_example;

  logic clk = 0;
  logic rst_n;
  always #5 clk = ~clk;

  logic        src_can_receive, src_receive_en;
  logic [31:0] src_data_write_in;
  logic        dst_has_data, dst_drop_item;
  logic [31:0] dst_data_read_out;

  worker dut (
    .clk               (clk),
    .rst_n             (rst_n),
    .src_can_receive   (src_can_receive),
    .src_receive_en    (src_receive_en),
    .src_data_write_in (src_data_write_in),
    .dst_has_data      (dst_has_data),
    .dst_drop_item     (dst_drop_item),
    .dst_data_read_out (dst_data_read_out)
  );

  // Offer one item, and wait until it is taken.
  task send_item(input logic [31:0] val);
    begin
      src_data_write_in <= val;
      src_receive_en    <= 1'b1;
      do @(posedge clk); while (!src_can_receive);
      src_receive_en <= 1'b0;
    end
  endtask

  // Wait for one item, read it, and acknowledge it.
  task receive_item(output logic [31:0] val);
    begin
      while (!dst_has_data) @(posedge clk);
      val           = dst_data_read_out;
      dst_drop_item <= 1'b1;
      @(posedge clk);
      dst_drop_item <= 1'b0;
    end
  endtask

  initial begin
    logic [31:0] res;

    rst_n             = 0;
    src_receive_en    = 0;
    src_data_write_in = 0;
    dst_drop_item     = 0;
    #20;
    rst_n = 1;
    @(posedge clk);

    fork
      for (int i = 1; i <= 10; i++) send_item(i * 10);
      for (int i = 1; i <= 10; i++) begin
        receive_item(res);
        $display("Received result: %0d", res);
      end
    join

    $display("TEST PASSED");
    $finish;
  end

endmodule
```

The two loops run concurrently because a pipeline holds several items at once: sending all ten before reading any would deadlock as soon as the design filled, which is the design working correctly.

### Testing backpressure

Hold `dst_drop_item` low for a while. The output fills its primary and skid slots, the pipeline stalls cleanly behind it, and `src_can_receive` goes low — nothing is lost. Holding `src_receive_en` high permanently is also safe: the module ignores it while `src_can_receive` is low, which `tests/adapters.rs` checks by simulation.

### Testing the pointer protocol directly

If you built with `--bare-export`, the module has `_wsalt` / `_rsalt` / `_data` instead, and the testbench has to toggle gray-code pointers itself. `examples/tb_mul3_equiv.sv` is a worked example — it drives that form because it compares the generated logic against a hand-written module that speaks it.

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
