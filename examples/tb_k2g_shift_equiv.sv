// Equivalence harness: the hand-written k2g_shift.sv against the DDL-generated
// k2g_shift_ddl.v, driven by the same stimulus.
`timescale 1ns/1ps
`include "k2g_types.svh"

module tb_k2g_shift_equiv;
  import k2g_types::*;

  logic [31:0] value, src;
  logic [4:0]  amount, bm_start, bm_span;
  logic [1:0]  op_bits;

  logic [31:0] ref_shift, ref_bext, ref_bins;
  logic [31:0] ddl_shift, ddl_bext, ddl_bins;

  k2g_shift u_ref (
      .value(value), .src(src), .amount(amount),
      .shift_op(shift_e'(op_bits)),
      .bm_start(bm_start), .bm_span(bm_span),
      .shift_result(ref_shift), .bext_result(ref_bext), .bins_result(ref_bins)
  );

  k2g_shift_ddl u_ddl (
      .value(value), .src(src), .amount(amount),
      .shift_op(op_bits),
      .bm_start(bm_start), .bm_span(bm_span),
      .shift_result(ddl_shift), .bext_result(ddl_bext), .bins_result(ddl_bins)
  );

  int errors = 0;
  int checks = 0;

  task automatic check(string what, logic [31:0] a, logic [31:0] b);
    checks++;
    if (a !== b) begin
      errors++;
      if (errors <= 10)
        $display("MISMATCH %s: ref=%08x ddl=%08x  (value=%08x src=%08x amount=%0d op=%0d start=%0d span=%0d)",
                 what, a, b, value, src, amount, op_bits, bm_start, bm_span);
    end
  endtask

  task automatic drive(logic [31:0] v, logic [31:0] s, logic [4:0] amt,
                       logic [1:0] op, logic [4:0] st, logic [4:0] sp);
    value = v; src = s; amount = amt; op_bits = op; bm_start = st; bm_span = sp;
    #1;
    check("shift", ref_shift, ddl_shift);
    check("bext",  ref_bext,  ddl_bext);
    check("bins",  ref_bins,  ddl_bins);
  endtask

  initial begin
    // Directed corners first: the documented edge cases are span 0 (all-zero
    // mask, BINS a no-op), amount 0 and 31, and the sign-fill boundary.
    drive(32'h8000_0000, 32'hFFFF_FFFF, 5'd0,  2'd2, 5'd0,  5'd0);
    drive(32'h8000_0000, 32'hFFFF_FFFF, 5'd31, 2'd2, 5'd0,  5'd31);
    drive(32'h7FFF_FFFF, 32'h0000_0001, 5'd1,  2'd0, 5'd31, 5'd1);
    drive(32'h0000_0000, 32'h0000_0000, 5'd0,  2'd1, 5'd0,  5'd0);
    drive(32'hDEAD_BEEF, 32'hCAFE_BABE, 5'd16, 2'd3, 5'd8,  5'd16);

    // Then exhaustive over the small fields, random over the wide ones.
    for (int op = 0; op < 4; op++)
      for (int amt = 0; amt < 32; amt++)
        for (int sp = 0; sp < 32; sp++)
          for (int st = 0; st < 32; st += 7)
            drive($urandom(), $urandom(), amt[4:0], op[1:0], st[4:0], sp[4:0]);

    for (int i = 0; i < 20000; i++)
      drive($urandom(), $urandom(), $urandom(), $urandom(), $urandom(), $urandom());

    if (errors == 0) $display("TB_PASS  %0d comparisons, 0 mismatches", checks);
    else             $display("TB_FAIL  %0d comparisons, %0d mismatches", checks, errors);
    $finish;
  end
endmodule
