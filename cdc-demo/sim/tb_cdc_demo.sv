// Throughput across a clock crossing, measured where it is deterministic.
//
// This bench answers ONE of the four arms and deliberately not the others. Arm
// C's claim -- that a two-entry pipe cannot stream across a crossing -- is a
// counting argument about a credit loop, so it is exact in RTL simulation and
// there is no reason to spend a bitstream discovering it. Arm B's claim is the
// opposite: Verilog samples old-or-new and models no metastability, so a
// crossing with no synchronizers at all SIMULATES PERFECTLY HERE. If you take
// the DUT-0 rows below as evidence that the design works, you have measured the
// simulator's abstraction, not the hardware. That asymmetry is why arms B and D
// are on a board and this one is not.
//
// Three DUTs, driven identically:
//
//   DUT-0  pipe_cdc SYNC=0   the crossing as the compiler emits it
//   DUT-1  pipe_cdc SYNC=1   the same, with two-flop synchronizers -- arm C
//   DUT-2  cdc_fifo          the proven depth-8 async FIFO -- arm E
//
// Each is a source in clock A pushing an incrementing counter whenever it has
// room, and a sink in clock B taking an item whenever one is offered. The
// numbers reported are items per clock-B cycle, and the sequence check is the
// same one the board runs.

`timescale 1ns / 1ps

module tb_cdc_demo;

  // 81 MHz and 135 MHz, the pair build.sh uses by default. Both come off one
  // crystal on the board; here they are simply two independent clocks, which is
  // the more conservative assumption for a throughput measurement.
  parameter real PERIOD_A = 12.346;
  parameter real PERIOD_B = 7.407;

  parameter integer WINDOW = 200000;   // clock-B cycles to measure over

  reg clk_a = 1'b0;
  reg clk_b = 1'b0;
  reg rst_n = 1'b0;

  always #(PERIOD_A / 2.0) clk_a = ~clk_a;
  always #(PERIOD_B / 2.0) clk_b = ~clk_b;

  // The reset discipline the board uses: one source, synchronized per domain,
  // held long past the floor, so both sides start from zero together.
  reg rst_a_n, rst_b_n;
  reg a_m, b_m;
  always @(posedge clk_a) begin a_m <= rst_n; rst_a_n <= a_m; end
  always @(posedge clk_b) begin b_m <= rst_n; rst_b_n <= b_m; end

  // ---------------------------------------------------------------- sources
  reg [15:0] src0, src1, src2;
  wire       full0, full1, full2;

  always @(posedge clk_a) if (!rst_a_n) src0 <= 16'd0; else if (!full0) src0 <= src0 + 16'd1;
  always @(posedge clk_a) if (!rst_a_n) src1 <= 16'd0; else if (!full1) src1 <= src1 + 16'd1;
  always @(posedge clk_a) if (!rst_a_n) src2 <= 16'd0; else if (!full2) src2 <= src2 + 16'd1;

  // ------------------------------------------------------------------- DUTs
  wire        valid0, valid1, valid2;
  wire [15:0] item0,  item1,  item2;

  pipe_cdc #(.SYNC(0)) u_dut0 (
      .sender_clk(clk_a), .sender_rst_n(rst_a_n),
      .put(1'b1), .data_in(src0), .full(full0),
      .reciever_clk(clk_b), .reciever_rst_n(rst_b_n),
      .drop(valid0), .valid(valid0), .item(item0)
  );

  pipe_cdc #(.SYNC(1)) u_dut1 (
      .sender_clk(clk_a), .sender_rst_n(rst_a_n),
      .put(1'b1), .data_in(src1), .full(full1),
      .reciever_clk(clk_b), .reciever_rst_n(rst_b_n),
      .drop(valid1), .valid(valid1), .item(item1)
  );

  wire empty2;
  assign valid2 = !empty2;
  cdc_fifo u_dut2 (
      .wclk(clk_a), .wrst_n(rst_a_n), .wpush(1'b1), .wdata(src2), .wfull(full2),
      .rclk(clk_b), .rrst_n(rst_b_n), .rpop(valid2), .rdata(item2), .rempty(empty2)
  );

  // ------------------------------------------------------------------ sinks
  integer rx0 = 0, rx1 = 0, rx2 = 0;
  integer err0 = 0, err1 = 0, err2 = 0;
  reg [15:0] exp0, exp1, exp2;
  reg armed0 = 1'b0, armed1 = 1'b0, armed2 = 1'b0;
  integer cyc = 0;
  reg measuring = 1'b0;

  always @(posedge clk_b) begin
    if (rst_b_n && measuring) cyc <= cyc + 1;

    if (valid0) begin
      if (measuring) begin
        rx0 <= rx0 + 1;
        if (armed0 && item0 !== exp0) err0 <= err0 + 1;
      end
      exp0 <= item0 + 16'd1;
      armed0 <= 1'b1;
    end

    if (valid1) begin
      if (measuring) begin
        rx1 <= rx1 + 1;
        if (armed1 && item1 !== exp1) err1 <= err1 + 1;
      end
      exp1 <= item1 + 16'd1;
      armed1 <= 1'b1;
    end

    if (valid2) begin
      if (measuring) begin
        rx2 <= rx2 + 1;
        if (armed2 && item2 !== exp2) err2 <= err2 + 1;
      end
      exp2 <= item2 + 16'd1;
      armed2 <= 1'b1;
    end
  end

  // ------------------------------------------------------------------- run
  real t0, t1, t2;

  initial begin
    rst_n = 1'b0;
    #500;
    rst_n = 1'b1;
    #500;                     // warm-up, excluded from the counts
    @(posedge clk_b) measuring = 1'b1;

    while (cyc < WINDOW) @(posedge clk_b);
    measuring = 1'b0;

    t0 = rx0 * 1.0 / cyc;
    t1 = rx1 * 1.0 / cyc;
    t2 = rx2 * 1.0 / cyc;

    $display("");
    $display("clock A %0.3f ns   clock B %0.3f ns   window %0d clock-B cycles",
             PERIOD_A, PERIOD_B, cyc);
    $display("");
    $display("  DUT                        items   items/cyc   %% of clk_b   errors");
    $display("  pipe_cdc SYNC=0 (arm B)  %7d     %6.3f     %6.1f%%   %6d", rx0, t0, t0 * 100.0, err0);
    $display("  pipe_cdc SYNC=1 (arm C)  %7d     %6.3f     %6.1f%%   %6d", rx1, t1, t1 * 100.0, err1);
    $display("  cdc_fifo depth 8 (arm E) %7d     %6.3f     %6.1f%%   %6d", rx2, t2, t2 * 100.0, err2);
    $display("");
    $display("  The source is in clock A at %0.1f MHz and the sink in clock B at %0.1f MHz,",
             1000.0 / PERIOD_A, 1000.0 / PERIOD_B);
    $display("  so the ceiling is the SLOWER of the two: %0.3f items per clock-B cycle.",
             (PERIOD_B / PERIOD_A) > 1.0 ? 1.0 : (PERIOD_B / PERIOD_A));
    $display("");
    $display("  Arm B's row is NOT evidence of a working design: this simulator");
    $display("  models no metastability, so an unsynchronized crossing is clean here");
    $display("  by construction. Only its THROUGHPUT column means anything.");
    $display("");

    if (err1 != 0 || err2 != 0)
      $display("  FAIL: a synchronized crossing lost or reordered items in simulation.");

    $finish;
  end

endmodule
