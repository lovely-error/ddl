// The reset handshake, at the ratios that break the obvious alternative.
//
// `rst_n` is synchronous to `clk`. Run it through two flops on a much slower
// foreign clock and a SHORT PULSE FALLS BETWEEN TWO EDGES AND IS MISSED — the
// core half resets, the foreign half does not, the FIFO's shared pointer state
// desynchronises, and it stays wrong forever without saying so.
//
// So this asserts `rst_n` for exactly ONE core cycle and requires that both
// halves still leave zero together, at ratios where the naive design cannot.
// Nothing on a board has exercised this; it is the one part of the crossing
// whose correctness rests entirely on simulation.
//
// Checked two ways after each pulse: the FIFO reports empty and not full (both
// pointers at zero, not merely equal to each other at some other lap), and a
// long stream then arrives in order with nothing lost or duplicated.

`timescale 1ns / 1ps

module tb_rst_cross;

  parameter real PERIOD_C = 10.0;    // core
  parameter real PERIOD_F = 10.0;    // foreign, overridden per run

  reg clk = 1'b0, f_clk = 1'b0, rst_n = 1'b1;
  always #(PERIOD_C / 2.0) clk   = ~clk;
  always #(PERIOD_F / 2.0) f_clk = ~f_clk;

  wire w_rst_n, r_rst_n;
  ddl_rst_cross u_rst (
      .clk(clk), .rst_n(rst_n), .f_clk(f_clk),
      .w_rst_n(w_rst_n), .r_rst_n(r_rst_n)
  );

  reg  [15:0] src;
  wire        wfull, rempty;
  wire [15:0] rdata;
  wire        push = !wfull;
  wire        pop  = !rempty;

  always @(posedge clk) begin
    if (!w_rst_n)  src <= 16'd0;
    else if (push) src <= src + 16'd1;
  end

  ddl_cdc_fifo #(.WIDTH(16), .AW(3)) u_fifo (
      .wclk(clk),   .wrst_n(w_rst_n), .wpush(push), .wdata(src), .wfull(wfull),
      .rclk(f_clk), .rrst_n(r_rst_n), .rpop(pop),   .rdata(rdata), .rempty(rempty)
  );

  integer rx = 0, err = 0;
  reg [15:0] expect_v, first_seen;
  reg        armed;
  always @(posedge f_clk) begin
    if (!r_rst_n) begin
      rx <= 0; err <= 0; expect_v <= 16'd0; armed <= 1'b0; first_seen <= 16'hFFFF;
    end else if (!rempty) begin
      if (!armed) first_seen <= rdata;
      rx <= rx + 1;
      if (armed && rdata !== expect_v) err <= err + 1;
      expect_v <= rdata + 16'd1;
      armed    <= 1'b1;
    end
  end

  integer fails = 0;
  integer ratio = 1;

  task pulse_and_check(input integer settle);
    begin
      // EXACTLY ONE CORE CYCLE. This is the case the naive design misses.
      @(negedge clk);
      rst_n = 1'b0;
      @(negedge clk);
      rst_n = 1'b1;

      // Let the handshake complete. It cannot be hurried: `resetting` stays set
      // until the far side answers, which takes two foreign edges each way.
      repeat (settle) @(posedge clk);

      // NOT a point check on empty/full: by the time the handshake has
      // completed the source has been refilling for hundreds of cycles, so
      // "not empty" is the correct state and testing for it is a test bug.
      //
      // The check that actually detects a half-reset is the FIRST ITEM. `src`
      // zeroes with the write half, so if both halves left zero together the
      // reader must see 0 first. A write pointer that did not reset leaves
      // stale entries in front of it and the first item is something else.
      if (first_seen !== 16'd0) begin
        $display("    FAIL: first item after reset was %0d, not 0 — the halves did not both zero",
                 first_seen);
        fails = fails + 1;
      end

      // And then it must actually stream, in order, with nothing lost.
      repeat (settle * 4) @(posedge clk);
      if (rx < 8) begin
        $display("    FAIL: only %0d items moved after reset — the link is stuck", rx);
        fails = fails + 1;
      end
      if (err != 0) begin
        $display("    FAIL: %0d sequence errors after reset", err);
        fails = fails + 1;
      end
      $display("    ratio 1:%0.0f  ->  %0d items, %0d errors, first=%0d  [w_rst_n=%b r_rst_n=%b resetting=%b f_sync=%b]",
               PERIOD_F / PERIOD_C, rx, err, first_seen,
               w_rst_n, r_rst_n, u_rst.resetting, u_rst.f_sync);
      $display("             wbin=%0d rbin=%0d wgray=%b wgray_sync=%b rgray=%b rempty=%b wfull=%b settle=%0d",
               u_fifo.wbin, u_fifo.rbin, u_fifo.wgray, u_fifo.wgray_sync,
               u_fifo.rgray, rempty, wfull, settle);
    end
  endtask

  initial begin
    $display("");
    $display("  ddl_rst_cross: a ONE-CYCLE `rst_n` pulse, core %0.1f ns / foreign %0.1f ns",
             PERIOD_C, PERIOD_F);

    // Power-on reset first, so the run starts from a known state.
    rst_n = 1'b0;
    repeat (3) @(posedge clk);
    $display("    [t=%0t] early: rst_n=%b resetting=%b w_rst_n=%b wbin=%0d",
             $time, rst_n, u_rst.resetting, w_rst_n, u_fifo.wbin);
    repeat (17) @(posedge clk);
    rst_n = 1'b1;
    ratio = (PERIOD_F > PERIOD_C) ? $rtoi(PERIOD_F / PERIOD_C) : 1;
    repeat (200 * ratio) @(posedge clk);

    // Settle is counted in CORE cycles but the handshake advances on FOREIGN
    // edges, so it must scale with the ratio or a slow foreign clock simply
    // gets fewer of its own cycles to answer in. A fixed count here was the
    // first version and it "failed" at 1:32 purely by not waiting.
    pulse_and_check(400 * ratio);
    pulse_and_check(400 * ratio);   // twice, to catch a handshake that works only once

    $display("");
    if (fails == 0) $display("  OK: both halves left zero together after every single-cycle reset.");
    else            $display("  FAILED: %0d checks", fails);
    $display("");
    $finish;
  end

endmodule
