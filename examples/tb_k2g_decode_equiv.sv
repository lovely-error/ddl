// Cycle-accurate equivalence: the hand-written k2g_decode.sv against the
// DDL-generated k2g_decode_ddl.v, clocked together off the same stimulus.
//
// Unlike the combinational ports, a mismatch here can be a state divergence
// that only shows up cycles later, so every output is compared on every edge
// and the first disagreement stops the run with the cycle number.
`timescale 1ns/1ps
`include "k2g_types.svh"

module tb_k2g_decode_equiv;
  import k2g_pkg::*;
  import k2g_types::*;

  logic clk = 1'b0;
  logic rst_n;
  logic [15:0] cp;
  logic cp_valid, flush, hold;

  always #5 clk = ~clk;

  logic        ref_accept, ref_uop_valid;
  uop_t        ref_uop;
  logic        ddl_accept, ddl_uop_valid;
  logic [126:0] ddl_uop;

  k2g_decode u_ref (
      .clk(clk), .rst_n(rst_n),
      .cp(cp), .cp_valid(cp_valid), .flush(flush), .hold(hold),
      .accept(ref_accept), .uop(ref_uop), .uop_valid(ref_uop_valid)
  );

  k2g_decode_ddl u_ddl (
      .clk(clk), .rst_n(rst_n),
      .cp(cp), .cp_valid(cp_valid), .flush(flush), .hold(hold),
      .accept(ddl_accept), .uop(ddl_uop), .uop_valid(ddl_uop_valid)
  );

  int errors = 0;
  int checks = 0;
  int cycles = 0;

  // Compared just before the edge, so both have settled on this cycle's
  // inputs and neither has clocked yet.
  task automatic compare();
    checks++;
    if (ref_accept !== ddl_accept) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH accept @%0d: ref=%b ddl=%b", cycles, ref_accept, ddl_accept);
    end
    if (ref_uop_valid !== ddl_uop_valid) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH uop_valid @%0d: ref=%b ddl=%b (cp=%04x)",
                 cycles, ref_uop_valid, ddl_uop_valid, cp);
    end
    if (ref_uop !== ddl_uop) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH uop @%0d (cp=%04x valid=%b):\n  ref=%032x\n  ddl=%032x",
                 cycles, cp, ref_uop_valid, ref_uop, ddl_uop);
    end
  endtask

  task automatic step(logic [15:0] c, logic v, logic f, logic h);
    cp = c; cp_valid = v; flush = f; hold = h;
    #1 compare();
    @(posedge clk);
    cycles++;
  endtask

  // A prefix code point, so random streams reach the accumulator paths rather
  // than decoding one main opcode after another.
  function automatic logic [15:0] a_prefix();
    case ($urandom_range(5))
      0: return {LB_XI,      10'($urandom)};
      1: return {LB_XIZEXT,  10'($urandom)};
      2: return {LB_BMX_0_0, 10'($urandom)};
      3: return {LB_XCP,     10'($urandom)};
      4: return {LB_XCN,     10'($urandom)};
      default: return {LB_EP1, 5'($urandom), 5'($urandom)};
    endcase
  endfunction

  initial begin
    rst_n = 1'b0;
    cp = 16'd0; cp_valid = 1'b0; flush = 1'b0; hold = 1'b0;
    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);

    // 1. Every main opcode on its own, with no prefix state.
    for (int op = 0; op < 64; op++)
      for (int a = 0; a < 4; a++)
        step({op[5:0], 5'($urandom), 5'($urandom)}, 1'b1, 1'b0, 1'b0);

    // 2. Prefix chains, which is where the accumulator and the chain-length
    //    fault live. Deliberately long enough to overrun the bound of 7.
    for (int t = 0; t < 3000; t++) begin
      automatic int n = $urandom_range(10);
      for (int i = 0; i < n; i++)
        step(a_prefix(), 1'b1, 1'b0, 1'b0);
      step($urandom, 1'b1, 1'b0, 1'b0);
    end

    // 3. LLC forms, which drive the S_LLC_HI / S_LLC_LO states.
    for (int t = 0; t < 2000; t++) begin
      step({LB_EP1, 5'($urandom), EP1_LLC_B32}, 1'b1, 1'b0, 1'b0);
      step($urandom, 1'b1, 1'b0, 1'b0);
      step($urandom, 1'b1, 1'b0, 1'b0);
      step({LB_EP1, 5'($urandom), EP1_LLC_B8}, 1'b1, 1'b0, 1'b0);
      step($urandom, 1'b1, 1'b0, 1'b0);
    end

    // 4. Fully random, including cp_valid gaps, holds and flushes. `hold` and
    //    `flush` must not disturb the decode, only the accumulator.
    for (int t = 0; t < 60000; t++)
      step($urandom,
           $urandom_range(9) != 0,      // cp_valid mostly high
           $urandom_range(19) == 0,     // occasional flush
           $urandom_range(4) == 0);     // frequent hold

    // 5. Reset in the middle of an instruction.
    for (int t = 0; t < 200; t++) begin
      step(a_prefix(), 1'b1, 1'b0, 1'b0);
      rst_n = 1'b0;
      step($urandom, 1'b1, 1'b0, 1'b0);
      rst_n = 1'b1;
      step($urandom, 1'b1, 1'b0, 1'b0);
    end

    if (errors == 0) $display("TB_PASS  %0d cycles, %0d comparisons, 0 mismatches", cycles, checks);
    else             $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
