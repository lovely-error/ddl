// Equivalence harness: the hand-written k2g_alu.sv against the DDL-generated
// k2g_alu_ddl.v, driven by the same stimulus.
`timescale 1ns/1ps
`include "k2g_types.svh"

module tb_k2g_alu_equiv;
  import k2g_types::*;

  logic [31:0] a, b;
  logic [2:0]  a_tag, b_tag;
  logic [1:0]  arith_bits, logic_bits, unary_bits;
  logic [2:0]  cmp_bits;

  logic [31:0] ref_arith, ref_logic, ref_unary;
  logic        ref_ovf, ref_cmp;
  logic [31:0] ddl_arith, ddl_logic, ddl_unary;
  logic        ddl_ovf, ddl_cmp;

  k2g_alu u_ref (
      .a(a), .b(b), .a_tag(a_tag), .b_tag(b_tag),
      .arith_op(arith_e'(arith_bits)),
      .logic_op(logic_e'(logic_bits)),
      .cmp_op(cmp_e'(cmp_bits)),
      .unary_op(unary_e'(unary_bits)),
      .arith_result(ref_arith), .arith_overflow(ref_ovf),
      .logic_result(ref_logic), .cmp_result(ref_cmp),
      .unary_result(ref_unary)
  );

  k2g_alu_ddl u_ddl (
      .a(a), .b(b), .a_tag(a_tag), .b_tag(b_tag),
      .arith_op(arith_bits),
      .logic_op(logic_bits),
      .cmp_op(cmp_bits),
      .unary_op(unary_bits),
      .arith_result(ddl_arith), .arith_overflow(ddl_ovf),
      .logic_result(ddl_logic), .cmp_result(ddl_cmp),
      .unary_result(ddl_unary)
  );

  int errors = 0;
  int checks = 0;

  task automatic check(string what, logic [31:0] x, logic [31:0] y);
    checks++;
    if (x !== y) begin
      errors++;
      if (errors <= 10)
        $display("MISMATCH %s: ref=%08x ddl=%08x  (a=%08x b=%08x a_tag=%0d b_tag=%0d arith=%0d logic=%0d cmp=%0d unary=%0d)",
                 what, x, y, a, b, a_tag, b_tag, arith_bits, logic_bits, cmp_bits, unary_bits);
    end
  endtask

  task automatic drive(logic [31:0] va, logic [31:0] vb,
                       logic [2:0] ta, logic [2:0] tb,
                       logic [1:0] ar, logic [1:0] lo,
                       logic [2:0] cm, logic [1:0] un);
    a = va; b = vb; a_tag = ta; b_tag = tb;
    arith_bits = ar; logic_bits = lo; cmp_bits = cm; unary_bits = un;
    #1;
    check("arith", ref_arith, ddl_arith);
    check("ovf",   {31'd0, ref_ovf}, {31'd0, ddl_ovf});
    check("logic", ref_logic, ddl_logic);
    check("cmp",   {31'd0, ref_cmp}, {31'd0, ddl_cmp});
    check("unary", ref_unary, ddl_unary);
  endtask

  // The values where sign, carry and borrow all change behaviour.
  localparam int NCORNER = 8;
  logic [31:0] corner [NCORNER];

  initial begin
    corner[0] = 32'h0000_0000;
    corner[1] = 32'h0000_0001;
    corner[2] = 32'h7FFF_FFFF;
    corner[3] = 32'h8000_0000;
    corner[4] = 32'hFFFF_FFFF;
    corner[5] = 32'h0000_00FF;
    corner[6] = 32'h0000_FFFF;
    corner[7] = 32'hDEAD_BEEF;

    // Exhaustive over every control field, with corner operands. This is the
    // part that matters: the comparison path has to be right for all four
    // signed/unsigned tag combinations, which is where a 32-bit comparator
    // would misorder (spec 5.9).
    for (int ar = 0; ar < 4; ar++)
      for (int lo = 0; lo < 4; lo++)
        for (int cm = 0; cm < 8; cm++)
          for (int un = 0; un < 4; un++)
            for (int ta = 0; ta < 8; ta++)
              for (int tb = 0; tb < 8; tb++)
                drive(corner[$urandom_range(NCORNER-1)],
                      corner[$urandom_range(NCORNER-1)],
                      ta[2:0], tb[2:0], ar[1:0], lo[1:0], cm[2:0], un[1:0]);

    // Every corner pair against every tag pair, for the comparison ordering.
    for (int i = 0; i < NCORNER; i++)
      for (int j = 0; j < NCORNER; j++)
        for (int ta = 0; ta < 8; ta++)
          for (int tb = 0; tb < 8; tb++)
            for (int cm = 0; cm < 8; cm++)
              drive(corner[i], corner[j], ta[2:0], tb[2:0],
                    2'd0, 2'd0, cm[2:0], 2'd0);

    for (int i = 0; i < 50000; i++)
      drive($urandom(), $urandom(), $urandom(), $urandom(),
            $urandom(), $urandom(), $urandom(), $urandom());

    if (errors == 0) $display("TB_PASS  %0d comparisons, 0 mismatches", checks);
    else             $display("TB_FAIL  %0d comparisons, %0d mismatches", checks, errors);
    $finish;
  end
endmodule
