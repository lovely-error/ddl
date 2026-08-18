// The hand-written k2g_regfile.sv against the DDL version, which is behind a
// channel rather than behind 24 flat wires.
//
// The two no longer have the same ports and cannot: a process takes data
// through pipes, so the DDL module answers a REQUEST with a RESPONSE. What
// must still be true is that the arrays behave identically -- same values,
// same per-field write enables, same read-before-write -- so the reference is
// driven with exactly the requests the DDL accepts, and the two answer streams
// are compared item by item.
//
// A shadow model runs alongside both. Two implementations of the same idea can
// share a mistake; a plain array in a testbench cannot share this one, and it
// is what checks that a read and a write of the same register in one cycle
// give the OLD value -- which k2g_regfile.sv:118 needs, because a write-first
// bypass there closes a combinational loop through the file.
`timescale 1ns/1ps

module tb_k2g_regfile_equiv;

  import k2g_pkg::*;

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  // The request, as the DDL packs it: first field in the high bits.
  logic [4:0]  ra_addr, rb_addr, rc_addr;
  logic        we_value, we_tag, we_overflow, we_flag;
  logic [4:0]  w_addr;
  logic [31:0] w_value;
  logic [2:0]  w_tag;
  logic        w_overflow, w_flag;

  logic        offer, rsp_ready;
  logic        ddl_req_ready, ddl_rsp_valid;
  logic [110:0] ddl_rsp;

  wire [60:0] req = {ra_addr, rb_addr, rc_addr,
                     we_value, we_tag, we_overflow, we_flag,
                     w_addr, w_value, w_tag, w_overflow, w_flag};

  // The reference sees exactly the requests the DDL accepts.
  wire xfer = offer & ddl_req_ready;

  logic [31:0] ref_ra_value, ref_rb_value, ref_rc_value;
  logic [2:0]  ref_ra_tag, ref_rb_tag, ref_rc_tag;
  logic        ref_ra_overflow, ref_rb_overflow, ref_rc_overflow;
  logic        ref_ra_flag, ref_rb_flag, ref_rc_flag;

  k2g_regfile u_ref (
      .clk(clk), .rst_n(rst_n),
      .ra_addr(ra_addr), .rb_addr(rb_addr), .rc_addr(rc_addr),
      .ra_value(ref_ra_value), .rb_value(ref_rb_value), .rc_value(ref_rc_value),
      .ra_tag(ref_ra_tag), .rb_tag(ref_rb_tag), .rc_tag(ref_rc_tag),
      .ra_overflow(ref_ra_overflow), .rb_overflow(ref_rb_overflow),
      .rc_overflow(ref_rc_overflow),
      .ra_flag(ref_ra_flag), .rb_flag(ref_rb_flag), .rc_flag(ref_rc_flag),
      .we_value(we_value & xfer), .we_tag(we_tag & xfer),
      .we_overflow(we_overflow & xfer), .we_flag(we_flag & xfer),
      .w_addr(w_addr), .w_value(w_value), .w_tag(rdt_e'(w_tag)),
      .w_overflow(w_overflow), .w_flag(w_flag)
  );

  k2g_regfile_ddl u_ddl (
      .clk(clk), .rst_n(rst_n),
      .req_valid(offer), .req_ready(ddl_req_ready), .req_data(req),
      .rsp_valid(ddl_rsp_valid), .rsp_ready(rsp_ready), .rsp_data(ddl_rsp)
  );

  // The reference's answer, packed the way the DDL packs its response.
  wire [110:0] ref_rsp = {ref_ra_value, ref_rb_value, ref_rc_value,
                          ref_ra_tag, ref_rb_tag, ref_rc_tag,
                          ref_ra_overflow, ref_rb_overflow, ref_rc_overflow,
                          ref_ra_flag, ref_rb_flag, ref_rc_flag};

  int errors = 0, checks = 0, cycles = 0, writes = 0, collisions = 0, compared = 0;

  logic [110:0] ref_q[$];
  logic [110:0] ddl_q[$];
  logic [110:0] mdl_q[$];

  // The independent model. Reset values are written here rather than taken
  // from either module, so a wrong reset fails instead of matching itself.
  logic [31:0] m_value    [0:31];
  logic [2:0]  m_tag      [0:31];
  logic        m_overflow [0:31];
  logic        m_flag     [0:31];

  logic         held_valid = 1'b0;
  logic [110:0] held_rsp;

  task automatic drain();
    while (ref_q.size() > 0 && ddl_q.size() > 0) begin
      automatic logic [110:0] a = ref_q.pop_front();
      automatic logic [110:0] b = ddl_q.pop_front();
      automatic logic [110:0] m = mdl_q.pop_front();
      checks += 2;
      compared++;
      if (a !== b) begin
        errors++;
        if (errors <= 5)
          $display("MISMATCH rsp #%0d @%0d:\n  ref=%028x\n  ddl=%028x", compared, cycles, a, b);
      end
      if (m !== b) begin
        errors++;
        if (errors <= 5)
          $display("MODEL rsp #%0d @%0d:\n  model=%028x\n  ddl  =%028x", compared, cycles, m, b);
      end
    end
  endtask

  task automatic step(logic v, logic r);
    offer = v; rsp_ready = r;
    #1;

    // Rule 2: an offer may not be withdrawn or altered before it is taken.
    checks++;
    if (held_valid && !(ddl_rsp_valid && rsp_ready)) begin
      if (!ddl_rsp_valid) begin
        errors++;
        if (errors <= 5) $display("RULE2 withdrawn @%0d", cycles);
      end else if (ddl_rsp !== held_rsp) begin
        errors++;
        if (errors <= 5) $display("RULE2 data changed @%0d", cycles);
      end
    end

    if (xfer) begin
      // The model answers from the arrays as they are NOW; the writes below
      // land at the edge, which is read-before-write.
      mdl_q.push_back({m_value[ra_addr], m_value[rb_addr], m_value[rc_addr],
                       m_tag[ra_addr], m_tag[rb_addr], m_tag[rc_addr],
                       m_overflow[ra_addr], m_overflow[rb_addr], m_overflow[rc_addr],
                       m_flag[ra_addr], m_flag[rb_addr], m_flag[rc_addr]});
      ref_q.push_back(ref_rsp);

      if (we_value && (w_addr == ra_addr || w_addr == rb_addr || w_addr == rc_addr))
        collisions++;
      if (we_value)    begin m_value[w_addr]    = w_value;    writes++; end
      if (we_tag)      begin m_tag[w_addr]      = w_tag;      writes++; end
      if (we_overflow) begin m_overflow[w_addr] = w_overflow; writes++; end
      if (we_flag)     begin m_flag[w_addr]     = w_flag;     writes++; end
    end
    if (ddl_rsp_valid && rsp_ready) ddl_q.push_back(ddl_rsp);
    drain();

    held_valid = ddl_rsp_valid && !rsp_ready;
    held_rsp   = ddl_rsp;
    @(posedge clk);
    cycles++;
  endtask

  // Rule 3: toggling `ready` within a cycle must not move `valid` or `data`.
  task automatic check_rule3();
    logic         v0;
    logic [110:0] d0;
    rsp_ready = 1'b0; #1;
    v0 = ddl_rsp_valid; d0 = ddl_rsp;
    rsp_ready = 1'b1; #1;
    checks++;
    if (ddl_rsp_valid !== v0 || ddl_rsp !== d0) begin
      errors++;
      if (errors <= 5) $display("RULE3 valid/data moved with ready @%0d", cycles);
    end
    rsp_ready = 1'b0;
  endtask

  task automatic put(logic [4:0] a, logic [4:0] b, logic [4:0] c,
                     logic wv, logic wt, logic wo, logic wf,
                     logic [4:0] wa, logic [31:0] dv, logic [2:0] dt,
                     logic dov, logic df);
    ra_addr = a; rb_addr = b; rc_addr = c;
    we_value = wv; we_tag = wt; we_overflow = wo; we_flag = wf;
    w_addr = wa; w_value = dv; w_tag = dt; w_overflow = dov; w_flag = df;
  endtask

  int i;
  initial begin
    rst_n = 1'b0;
    put(5'd0, 5'd0, 5'd0, 1'b0, 1'b0, 1'b0, 1'b0, 5'd0, 32'd0, 3'd0, 1'b0, 1'b0);
    offer = 1'b0; rsp_ready = 1'b0;
    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);

    for (i = 0; i < 32; i = i + 1) begin
      m_value[i]    = 32'd0;
      m_tag[i]      = 3'b010;  // RDT_U32, the emulator's reset tag
      m_overflow[i] = 1'b0;
      m_flag[i]     = 1'b0;
    end

    // Every register read from every port before anything is written, which
    // proves the reset happened rather than being masked by a write.
    for (i = 0; i < 32; i = i + 1) begin
      put(i[4:0], (31 - i), (i * 7) % 32, 1'b0, 1'b0, 1'b0, 1'b0,
          5'd0, 32'd0, 3'd0, 1'b0, 1'b0);
      step(1'b1, 1'b1);
    end

    // Write every register with a distinct pattern, then read them all back.
    for (i = 0; i < 32; i = i + 1) begin
      put(5'd0, 5'd0, 5'd0, 1'b1, 1'b1, 1'b1, 1'b1,
          i[4:0], 32'hA5A5_0000 + i, (i % 7), i[0], ~i[0]);
      step(1'b1, 1'b1);
    end
    for (i = 0; i < 32; i = i + 1) begin
      put(i[4:0], (31 - i), (i * 5) % 32, 1'b0, 1'b0, 1'b0, 1'b0,
          5'd0, 32'd0, 3'd0, 1'b0, 1'b0);
      step(1'b1, 1'b1);
    end

    // Independent per-field enables under random traffic, with the sink
    // refusing often so the response slot fills and back-pressure reaches the
    // requester. The write address comes from the same small set as the read
    // addresses, so collisions happen constantly rather than never.
    for (i = 0; i < 60000; i = i + 1) begin
      automatic logic wt = $urandom_range(1);
      // RDT_UNCLAIMED_7 is reserved and the DDL asserts it is never written.
      // It IS driven when the enable is low: an assertion written inside
      // `if r.we_tag` must stay silent then, and one that lost its guard
      // would fail the run.
      automatic logic [2:0] tag = wt ? $urandom_range(6) : $urandom_range(7);
      put($urandom_range(31), $urandom_range(31), $urandom_range(31),
          $urandom_range(1), wt, $urandom_range(1), $urandom_range(1),
          $urandom_range(31), $urandom, tag, $urandom_range(1), $urandom_range(1));
      step($urandom_range(2) != 0, $urandom_range(2) != 0);
    end

    // Deliberate collisions: read and write the same register every cycle.
    for (i = 0; i < 2000; i = i + 1) begin
      automatic logic [4:0] r = $urandom_range(31);
      put(r, r, r, 1'b1, 1'b1, 1'b1, 1'b1,
          r, $urandom, $urandom_range(6), $urandom_range(1), $urandom_range(1));
      step(1'b1, $urandom_range(1));
    end

    for (i = 0; i < 200; i = i + 1) begin
      put($urandom_range(31), $urandom_range(31), $urandom_range(31),
          1'b0, 1'b0, 1'b0, 1'b0, 5'd0, 32'd0, 3'd0, 1'b0, 1'b0);
      step(1'b1, 1'b0);
      check_rule3();
    end

    // Drain, so a response one side produced and the other swallowed cannot
    // hide in a queue.
    for (i = 0; i < 16; i = i + 1)
      step(1'b0, 1'b1);

    if (ref_q.size() != 0 || ddl_q.size() != 0) begin
      errors++;
      $display("LEFTOVER ref=%0d ddl=%0d", ref_q.size(), ddl_q.size());
    end
    if (compared < 20000 || collisions < 1000) begin
      $display("TB_FAIL  vacuous: %0d responses, %0d collisions", compared, collisions);
      $finish;
    end

    if (errors == 0)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d responses, %0d writes, %0d collisions, 0 mismatches",
               cycles, checks, compared, writes, collisions);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
