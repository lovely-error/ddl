// Two things, both about `lib/ddl_cdc_fifo.v` being trustworthy.
//
// 1. EQUIVALENCE. `cdc-demo/cdc_fifo.v` is the frozen artifact that ran on the
//    board as arm E -- zero errors at 100% of the ceiling. The shipped module is
//    that design with `WIDTH` and `AW` restored. "Restoring parameters changes
//    nothing" is an assertion, and the hardware evidence only transfers if it is
//    true, so this drives both under identical stimulus at two independent
//    clocks and fails on the first cycle they disagree about anything.
//
// 2. THE PARAMETERS. WIDTH and AW are now user-facing, so they get exercised:
//    integrity and throughput across widths and depths, including AW = 2, the
//    smallest legal value, where the full test's `[AW-2:0]` degenerates to a
//    single bit.

`timescale 1ns / 1ps

module tb_cdc_lib;

  parameter real PERIOD_A = 12.346;   // 81 MHz
  parameter real PERIOD_B = 9.259;    // 108 MHz

  reg clk_a = 1'b0, clk_b = 1'b0, rst_n = 1'b0;
  always #(PERIOD_A / 2.0) clk_a = ~clk_a;
  always #(PERIOD_B / 2.0) clk_b = ~clk_b;

  reg rst_a_n, rst_b_n, a_m, b_m;
  always @(posedge clk_a) begin a_m <= rst_n; rst_a_n <= a_m; end
  always @(posedge clk_b) begin b_m <= rst_n; rst_b_n <= b_m; end

  // ---------------------------------------------------------- 1. equivalence
  reg  [15:0] src = 16'd0;
  wire        full_ref, full_lib, empty_ref, empty_lib;
  wire [15:0] data_ref, data_lib;

  // Both see the same push, gated on the REFERENCE's full, so a divergence in
  // `full` shows up as a data difference rather than quietly giving the two
  // different stimulus.
  wire push = !full_ref;
  wire pop  = !empty_ref;

  always @(posedge clk_a) begin
    if (!rst_a_n)   src <= 16'd0;
    else if (push)  src <= src + 16'd1;
  end

  cdc_fifo u_ref (
      .wclk(clk_a), .wrst_n(rst_a_n), .wpush(push), .wdata(src), .wfull(full_ref),
      .rclk(clk_b), .rrst_n(rst_b_n), .rpop(pop),   .rdata(data_ref), .rempty(empty_ref)
  );

  ddl_cdc_fifo #(.WIDTH(16), .AW(3)) u_lib (
      .wclk(clk_a), .wrst_n(rst_a_n), .wpush(push), .wdata(src), .wfull(full_lib),
      .rclk(clk_b), .rrst_n(rst_b_n), .rpop(pop),   .rdata(data_lib), .rempty(empty_lib)
  );

  integer mismatches = 0;
  always @(posedge clk_a) if (rst_a_n && full_ref !== full_lib) begin
    mismatches = mismatches + 1;
    $display("  MISMATCH wfull at %0t: ref=%b lib=%b", $time, full_ref, full_lib);
  end
  always @(posedge clk_b) if (rst_b_n) begin
    if (empty_ref !== empty_lib) begin
      mismatches = mismatches + 1;
      $display("  MISMATCH rempty at %0t: ref=%b lib=%b", $time, empty_ref, empty_lib);
    end
    if (!empty_ref && data_ref !== data_lib) begin
      mismatches = mismatches + 1;
      $display("  MISMATCH rdata at %0t: ref=%h lib=%h", $time, data_ref, data_lib);
    end
  end

  // ---------------------------------------------------------- 2. parameters
  // One instance per shape, each with its own source and checker.
  genvar g;
  generate
    for (g = 0; g < 4; g = g + 1) begin : g_shape
      localparam W  = (g == 0) ? 8  : (g == 1) ? 16 : (g == 2) ? 32 : 64;
      localparam AW = (g == 0) ? 2  : (g == 1) ? 3  : (g == 2) ? 4  : 3;

      reg  [W-1:0] s = {W{1'b0}};
      wire         f, e;
      wire [W-1:0] d;
      wire         p = !f;

      always @(posedge clk_a) begin
        if (!rst_a_n) s <= {W{1'b0}};
        else if (p)   s <= s + {{(W-1){1'b0}}, 1'b1};
      end

      ddl_cdc_fifo #(.WIDTH(W), .AW(AW)) u (
          .wclk(clk_a), .wrst_n(rst_a_n), .wpush(p),  .wdata(s), .wfull(f),
          .rclk(clk_b), .rrst_n(rst_b_n), .rpop(!e),  .rdata(d), .rempty(e)
      );

      integer rx = 0, err = 0;
      reg [W-1:0] expect_v = {W{1'b0}};
      reg armed = 1'b0;
      always @(posedge clk_b) if (rst_b_n && !e) begin
        rx <= rx + 1;
        if (armed && d !== expect_v) err <= err + 1;
        expect_v <= d + {{(W-1){1'b0}}, 1'b1};
        armed    <= 1'b1;
      end
    end
  endgenerate

  integer cyc = 0;
  always @(posedge clk_b) if (rst_b_n) cyc <= cyc + 1;

  real ceil_v;
  initial begin
    #400; rst_n = 1'b1;
    while (cyc < 200000) @(posedge clk_b);

    ceil_v = (PERIOD_B / PERIOD_A) > 1.0 ? 1.0 : (PERIOD_B / PERIOD_A);
    $display("");
    $display("  EQUIVALENCE  cdc_fifo (the artifact arm E ran) vs ddl_cdc_fifo #(16,3)");
    if (mismatches == 0)
      $display("    OK: bit-identical on wfull, rempty and rdata for %0d cycles.", cyc);
    else
      $display("    FAIL: %0d disagreements -- the board evidence does NOT transfer.", mismatches);

    $display("");
    $display("  PARAMETERS   ceiling is the slower clock = %0.3f items/clk_b cycle", ceil_v);
    $display("    %-22s %10s %10s %8s", "shape", "items/cyc", "% ceiling", "errors");
    $display("    WIDTH  8, AW 2 (d4)    %10.3f %9.1f%% %8d",
             g_shape[0].rx * 1.0 / cyc, g_shape[0].rx * 100.0 / cyc / ceil_v, g_shape[0].err);
    $display("    WIDTH 16, AW 3 (d8)    %10.3f %9.1f%% %8d",
             g_shape[1].rx * 1.0 / cyc, g_shape[1].rx * 100.0 / cyc / ceil_v, g_shape[1].err);
    $display("    WIDTH 32, AW 4 (d16)   %10.3f %9.1f%% %8d",
             g_shape[2].rx * 1.0 / cyc, g_shape[2].rx * 100.0 / cyc / ceil_v, g_shape[2].err);
    $display("    WIDTH 64, AW 3 (d8)    %10.3f %9.1f%% %8d",
             g_shape[3].rx * 1.0 / cyc, g_shape[3].rx * 100.0 / cyc / ceil_v, g_shape[3].err);
    $display("");
    $display("    AW 2 is the smallest legal depth and is expected to be SLOWER,");
    $display("    not wrong: depth/RTT caps it below the ceiling. AW 3 and above");
    $display("    should reach the ceiling and all four must show zero errors.");
    $display("");
    $finish;
  end

endmodule
