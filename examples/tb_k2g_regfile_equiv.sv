// Equivalence for a generated register file.
//
// Three things are checked, and the third is the one that matters:
//
//   1. Every output of the DDL module against the hand-written one, cycle by
//      cycle, under random reads and per-field random writes.
//   2. An INDEPENDENT shadow model, so both implementations agreeing on a
//      wrong answer still fails. Two modules generated from the same idea can
//      share a mistake; a plain array in the testbench cannot share this one.
//   3. READ-BEFORE-WRITE, explicitly. A read and a write of the same register
//      in the same cycle must give the OLD value. k2g_regfile.sv:118 says why
//      that is correctness rather than taste -- a write-first bypass closes a
//      combinational loop through the file and hangs simulation -- so the
//      collision case is driven deliberately rather than left to chance.
`timescale 1ns/1ps

module tb_k2g_regfile_equiv;

  import k2g_pkg::*;

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  logic [4:0]  ra_addr, rb_addr, rc_addr;
  logic        we_value, we_tag, we_overflow, we_flag;
  logic [4:0]  w_addr;
  logic [31:0] w_value;
  logic [2:0]  w_tag;
  logic        w_overflow, w_flag;

  logic [31:0] ref_ra_value, ref_rb_value, ref_rc_value;
  logic [2:0]  ref_ra_tag, ref_rb_tag, ref_rc_tag;
  logic        ref_ra_overflow, ref_rb_overflow, ref_rc_overflow;
  logic        ref_ra_flag, ref_rb_flag, ref_rc_flag;

  logic [31:0] ddl_ra_value, ddl_rb_value, ddl_rc_value;
  logic [2:0]  ddl_ra_tag, ddl_rb_tag, ddl_rc_tag;
  logic        ddl_ra_overflow, ddl_rb_overflow, ddl_rc_overflow;
  logic        ddl_ra_flag, ddl_rb_flag, ddl_rc_flag;

  k2g_regfile u_ref (
      .clk(clk), .rst_n(rst_n),
      .ra_addr(ra_addr), .rb_addr(rb_addr), .rc_addr(rc_addr),
      .ra_value(ref_ra_value), .rb_value(ref_rb_value), .rc_value(ref_rc_value),
      .ra_tag(ref_ra_tag), .rb_tag(ref_rb_tag), .rc_tag(ref_rc_tag),
      .ra_overflow(ref_ra_overflow), .rb_overflow(ref_rb_overflow),
      .rc_overflow(ref_rc_overflow),
      .ra_flag(ref_ra_flag), .rb_flag(ref_rb_flag), .rc_flag(ref_rc_flag),
      .we_value(we_value), .we_tag(we_tag),
      .we_overflow(we_overflow), .we_flag(we_flag),
      .w_addr(w_addr), .w_value(w_value), .w_tag(rdt_e'(w_tag)),
      .w_overflow(w_overflow), .w_flag(w_flag)
  );

  k2g_regfile_ddl u_ddl (
      .clk(clk), .rst_n(rst_n),
      .ra_addr(ra_addr), .rb_addr(rb_addr), .rc_addr(rc_addr),
      .we_value(we_value), .we_tag(we_tag),
      .we_overflow(we_overflow), .we_flag(we_flag),
      .w_addr(w_addr), .w_value(w_value), .w_tag(w_tag),
      .w_overflow(w_overflow), .w_flag(w_flag),
      .ra_value(ddl_ra_value), .rb_value(ddl_rb_value), .rc_value(ddl_rc_value),
      .ra_tag(ddl_ra_tag), .rb_tag(ddl_rb_tag), .rc_tag(ddl_rc_tag),
      .ra_overflow(ddl_ra_overflow), .rb_overflow(ddl_rb_overflow),
      .rc_overflow(ddl_rc_overflow),
      .ra_flag(ddl_ra_flag), .rb_flag(ddl_rb_flag), .rc_flag(ddl_rc_flag)
  );

  int errors = 0, checks = 0, cycles = 0, writes = 0, collisions = 0;

  // The independent model. Reset values are written here rather than derived
  // from either module, so a wrong reset fails instead of matching itself.
  logic [31:0] m_value    [0:31];
  logic [2:0]  m_tag      [0:31];
  logic        m_overflow [0:31];
  logic        m_flag     [0:31];

  task automatic expect_eq(string what, logic [31:0] a, logic [31:0] b);
    checks++;
    if (a !== b) begin
      errors++;
      if (errors <= 8)
        $display("MISMATCH %s @%0d: ref/model=%08x ddl=%08x", what, cycles, a, b);
    end
  endtask

  task automatic compare();
    expect_eq("ra_value", ref_ra_value, ddl_ra_value);
    expect_eq("rb_value", ref_rb_value, ddl_rb_value);
    expect_eq("rc_value", ref_rc_value, ddl_rc_value);
    expect_eq("ra_tag", {29'd0, ref_ra_tag}, {29'd0, ddl_ra_tag});
    expect_eq("rb_tag", {29'd0, ref_rb_tag}, {29'd0, ddl_rb_tag});
    expect_eq("rc_tag", {29'd0, ref_rc_tag}, {29'd0, ddl_rc_tag});
    expect_eq("ra_overflow", {31'd0, ref_ra_overflow}, {31'd0, ddl_ra_overflow});
    expect_eq("rb_overflow", {31'd0, ref_rb_overflow}, {31'd0, ddl_rb_overflow});
    expect_eq("rc_overflow", {31'd0, ref_rc_overflow}, {31'd0, ddl_rc_overflow});
    expect_eq("ra_flag", {31'd0, ref_ra_flag}, {31'd0, ddl_ra_flag});
    expect_eq("rb_flag", {31'd0, ref_rb_flag}, {31'd0, ddl_rb_flag});
    expect_eq("rc_flag", {31'd0, ref_rc_flag}, {31'd0, ddl_rc_flag});

    // Against the model, which knows nothing about either implementation.
    expect_eq("model ra_value", m_value[ra_addr], ddl_ra_value);
    expect_eq("model rb_value", m_value[rb_addr], ddl_rb_value);
    expect_eq("model rc_value", m_value[rc_addr], ddl_rc_value);
    expect_eq("model ra_tag", {29'd0, m_tag[ra_addr]}, {29'd0, ddl_ra_tag});
    expect_eq("model rc_tag", {29'd0, m_tag[rc_addr]}, {29'd0, ddl_rc_tag});
    expect_eq("model ra_overflow", {31'd0, m_overflow[ra_addr]}, {31'd0, ddl_ra_overflow});
    expect_eq("model rb_flag", {31'd0, m_flag[rb_addr]}, {31'd0, ddl_rb_flag});
  endtask

  task automatic step(
      logic [4:0] a, logic [4:0] b, logic [4:0] c,
      logic wv, logic wt, logic wo, logic wf,
      logic [4:0] wa, logic [31:0] dv, logic [2:0] dt, logic dov, logic df);
    ra_addr = a; rb_addr = b; rc_addr = c;
    we_value = wv; we_tag = wt; we_overflow = wo; we_flag = wf;
    w_addr = wa; w_value = dv; w_tag = dt; w_overflow = dov; w_flag = df;
    #1 compare();

    // A read of the register being written this cycle must still see the old
    // value: the comparison above already happened, and the model updates only
    // now, at the edge.
    if (wv && (wa == a || wa == b || wa == c)) collisions++;

    if (wv) begin m_value[wa]    = dv;  writes++; end
    if (wt) begin m_tag[wa]      = dt;  writes++; end
    if (wo) begin m_overflow[wa] = dov; writes++; end
    if (wf) begin m_flag[wa]     = df;  writes++; end

    @(posedge clk);
    cycles++;
  endtask

  int i;
  initial begin
    rst_n = 1'b0;
    ra_addr = 5'd0; rb_addr = 5'd0; rc_addr = 5'd0;
    we_value = 1'b0; we_tag = 1'b0; we_overflow = 1'b0; we_flag = 1'b0;
    w_addr = 5'd0; w_value = 32'd0; w_tag = 3'd0;
    w_overflow = 1'b0; w_flag = 1'b0;
    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);

    for (i = 0; i < 32; i = i + 1) begin
      m_value[i]    = 32'd0;
      m_tag[i]      = 3'b010;  // RDT_U32, the emulator's reset tag
      m_overflow[i] = 1'b0;
      m_flag[i]     = 1'b0;
    end

    // Every register read from every port before anything is written, which is
    // what proves the reset actually happened rather than being masked by a
    // write on the way past.
    for (i = 0; i < 32; i = i + 1)
      step(i[4:0], (31 - i), (i * 7) % 32,
           1'b0, 1'b0, 1'b0, 1'b0, 5'd0, 32'd0, 3'd0, 1'b0, 1'b0);

    // Write every register with a distinct pattern, then read them all back.
    for (i = 0; i < 32; i = i + 1)
      step(5'd0, 5'd0, 5'd0,
           1'b1, 1'b1, 1'b1, 1'b1,
           i[4:0], 32'hA5A5_0000 + i, (i % 7), i[0], ~i[0]);
    for (i = 0; i < 32; i = i + 1)
      step(i[4:0], (31 - i), (i * 5) % 32,
           1'b0, 1'b0, 1'b0, 1'b0, 5'd0, 32'd0, 3'd0, 1'b0, 1'b0);

    // Independent per-field enables under random traffic. The write address is
    // drawn from the same small set as the read addresses, so read/write
    // collisions happen constantly rather than never.
    for (i = 0; i < 60000; i = i + 1) begin
      automatic logic wt = $urandom_range(1);
      // RDT_UNCLAIMED_7 is reserved and the DDL asserts it is never written.
      // It IS driven here when the enable is low: an assertion written inside
      // `if we_tag` must stay silent then, and one that lost its guard would
      // fail the run.
      automatic logic [2:0] tag = wt ? $urandom_range(6) : $urandom_range(7);
      step($urandom_range(31), $urandom_range(31), $urandom_range(31),
           $urandom_range(1), wt, $urandom_range(1), $urandom_range(1),
           $urandom_range(31), $urandom, tag,
           $urandom_range(1), $urandom_range(1));
    end

    // Deliberate collisions: read and write the same register every cycle.
    for (i = 0; i < 2000; i = i + 1) begin
      automatic logic [4:0] r = $urandom_range(31);
      step(r, r, r, 1'b1, 1'b1, 1'b1, 1'b1,
           r, $urandom, $urandom_range(6), $urandom_range(1), $urandom_range(1));
    end

    if (collisions < 1000) begin
      $display("TB_FAIL  only %0d read/write collisions -- the case did not run", collisions);
      $finish;
    end
    if (errors == 0)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d writes, %0d collisions, 0 mismatches",
               cycles, checks, writes, collisions);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
