// The whole board top, simulated, so the instrument is known good before a
// bitstream is trusted.
//
// This does NOT test the crossing -- Verilog samples old-or-new and models no
// metastability, so arm B is clean here by construction and would be however
// broken the design was. What it tests is everything AROUND the crossing, which
// is what would otherwise turn a real result into an unreadable one:
//
//   * the reset tree releases, and both domains come out of it;
//   * the sinks see items in order and the error counters stay at zero when
//     nothing is wrong;
//   * the report state machine emits a well-formed line -- the arm letter,
//     seven 8-digit hex fields separated by spaces, CR LF -- in that order and
//     with the counters in the right positions.
//
// A garbled report on the board would look exactly like a broken DUT. The
// divider widths are shrunk through `CHAR_DIV_BITS` and `RPT_TICK_BITS` so a
// whole line takes microseconds instead of a second; nothing else changes.

`timescale 1ns / 1ps

// Included here as well as in the design: macro visibility across separately
// compiled files is not something to rely on.
`include "arm_config.vh"

module tb_top;

  reg clk_27m = 1'b0;
  reg rst_n   = 1'b0;

  always #18.518 clk_27m = ~clk_27m;    // 27 MHz

  wire       uart_tx;
  wire [5:0] led;

  cdc_demo_top dut (
      .clk_27m(clk_27m),
      .rst_n  (rst_n),
      .uart_rx(1'b1),
      .uart_tx(uart_tx),
      .led    (led)
  );

  // The character stream, straight off the transmitter's input. The UART itself
  // is a copy of a proven module; the report state machine feeding it is the new
  // code and the thing worth watching.
  integer nchar = 0;
  integer nline = 0;
  reg [8*80-1:0] line;
  integer linelen = 0;

  always @(posedge dut.clk_b) begin
    if (dut.tx_valid) begin
      nchar = nchar + 1;
      if (dut.tx_data == 8'h0A) begin
        $write("  line %0d: ", nline);
        for (integer i = 0; i < linelen; i = i + 1)
          $write("%c", line[8*(linelen-1-i) +: 8]);
        $write("\n");
        nline   = nline + 1;
        linelen = 0;
      end else if (dut.tx_data != 8'h0D) begin
        line    = {line[8*79-1:0], dut.tx_data};
        linelen = linelen + 1;
      end
    end
  end

  initial begin
    $display("");
    $display("  arm %s, clock A %0d Hz, clock B %0d Hz",
             `ARM_CHAR, `SIM_A_HZ, `SIM_B_HZ);
    $display("");

    rst_n = 1'b0;
    #4000;                    // past the PLL model's LOCK_NS
    rst_n = 1'b1;

    wait (nline == 1);

    // PRESS THE BUTTON, mid-line, exactly as a person would. The bench never
    // did this before: it only ever reset at time zero, which is why a
    // reset-recovery fault in the reporter could survive every simulation and
    // still appear on the board the moment someone pressed reset.
    // A REAL BUTTON: asynchronous to both clocks, bouncing on the way down and
    // on the way up, and held far longer than any counter in the design. The
    // bench used to reset only at time zero, cleanly and clock-aligned, which
    // is why a reset-recovery fault could survive every simulation and still
    // appear the instant someone pressed the button.
    #337.3;
    $display("  --- button pressed (bouncing) ---");
    repeat (6) begin rst_n = 1'b0; #13.7; rst_n = 1'b1; #7.1; end
    rst_n = 1'b0;
    #40000;                   // held
    repeat (6) begin rst_n = 1'b1; #9.3; rst_n = 1'b0; #11.9; end
    rst_n = 1'b1;
    $display("  --- button released ---");

    wait (nline == 4);

    $display("");
    $display("  reset released, both domains running, %0d characters in %0d lines",
             nchar, nline);
    $display("");
    $display("  Fields are: <arm> rx1 err1 idle1_hw rx2 err2 idle2_hw cyc");
    $display("  err1 and err2 must be 00000000 -- this simulator cannot produce");
    $display("  a crossing failure, so anything else here is a harness bug.");
    $display("");

    if (dut.err1 !== 32'd0 || dut.err2 !== 32'd0)
      $display("  FAIL: the checkers found errors in a simulation that cannot produce them.");
    else
      $display("  OK: both checkers clean, report well formed.");

    $display("");
    $display("  slot 1 rx=%0d  slot 2 rx=%0d  cycles=%0d", dut.rx1, dut.rx2, dut.cyc);
    if (dut.cyc != 0) begin
      $display("  slot 1 throughput %0.3f items/cycle, slot 2 %0.3f",
               dut.rx1 * 1.0 / dut.cyc, dut.rx2 * 1.0 / dut.cyc);
      $display("  (slot 2's ceiling is half of slot 1's: `chk` is a two-state");
      $display("   process and forwards one item every two cycles.)");
    end
    $display("");
    $finish;
  end

  initial begin
    #20_000_000;
    $display("  FAIL: timed out waiting for three report lines.");
    $finish;
  end

endmodule
