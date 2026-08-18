// Equivalence for a generated state machine.
//
// Both are driven from one stimulus with independent random backpressure on
// each side, which is what exercises stalling in every state. The DDL side is
// also checked against channel rules 2 and 3 directly, since holding them by
// construction is the claim being made.
`timescale 1ns/1ps

module tb_fsm_adder_equiv;

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  logic        src_valid;
  logic [31:0] src_data;
  logic        dst_ready;

  logic        ref_src_ready, ref_dst_valid;
  logic [31:0] ref_dst_data;
  logic        ddl_src_ready, ddl_dst_valid;
  logic [31:0] ddl_dst_data;

  fsm_adder_ref u_ref (
      .clk(clk), .rst_n(rst_n),
      .src_valid(src_valid), .src_ready(ref_src_ready), .src_data(src_data),
      .dst_valid(ref_dst_valid), .dst_ready(dst_ready), .dst_data(ref_dst_data)
  );

  fsm_adder u_ddl (
      .clk(clk), .rst_n(rst_n),
      .src_valid(src_valid), .src_ready(ddl_src_ready), .src_data(src_data),
      .dst_valid(ddl_dst_valid), .dst_ready(dst_ready), .dst_data(ddl_dst_data)
  );

  int errors = 0, checks = 0, cycles = 0, sent = 0, got = 0;
  logic [31:0] expect_q[$];
  logic [31:0] pending_a;
  bit          have_a = 1'b0;

  logic        held_valid = 1'b0;
  logic [31:0] held_data;

  task automatic compare();
    checks++;
    if (ref_src_ready !== ddl_src_ready) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH src_ready @%0d: ref=%b ddl=%b", cycles, ref_src_ready, ddl_src_ready);
    end
    if (ref_dst_valid !== ddl_dst_valid) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH dst_valid @%0d: ref=%b ddl=%b", cycles, ref_dst_valid, ddl_dst_valid);
    end
    if (ref_dst_valid && (ref_dst_data !== ddl_dst_data)) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH dst_data @%0d: ref=%08x ddl=%08x", cycles, ref_dst_data, ddl_dst_data);
    end
    // Rule 2: an offer may not be withdrawn or altered before it is taken.
    if (held_valid && !(ddl_dst_valid && dst_ready)) begin
      if (!ddl_dst_valid) begin
        errors++;
        if (errors <= 5) $display("RULE2 withdrawn @%0d", cycles);
      end else if (ddl_dst_data !== held_data) begin
        errors++;
        if (errors <= 5) $display("RULE2 data changed @%0d", cycles);
      end
    end
  endtask

  task automatic step(logic v, logic [31:0] d, logic r);
    src_valid = v; src_data = d; dst_ready = r;
    #1 compare();

    // Model the protocol independently: two accepted inputs make one sum.
    if (ddl_src_ready && v) begin
      if (!have_a) begin
        pending_a = d;
        have_a = 1'b1;
      end else begin
        expect_q.push_back(pending_a + d);
        have_a = 1'b0;
      end
      sent++;
    end
    if (ddl_dst_valid && r) begin
      automatic logic [31:0] want = expect_q.pop_front();
      checks++;
      if (ddl_dst_data !== want) begin
        errors++;
        if (errors <= 5)
          $display("WRONG SUM @%0d: got %08x want %08x", cycles, ddl_dst_data, want);
      end
      got++;
    end

    held_valid = ddl_dst_valid && !dst_ready;
    held_data  = ddl_dst_data;
    @(posedge clk);
    cycles++;
  endtask

  // Rule 3: toggling `ready` within a cycle must not move `valid` or `data`.
  task automatic check_rule3();
    logic        v0;
    logic [31:0] d0;
    dst_ready = 1'b0; #1;
    v0 = ddl_dst_valid; d0 = ddl_dst_data;
    dst_ready = 1'b1; #1;
    checks++;
    if (ddl_dst_valid !== v0 || ddl_dst_data !== d0) begin
      errors++;
      if (errors <= 5) $display("RULE3 valid/data moved with ready @%0d", cycles);
    end
  endtask

  initial begin
    rst_n = 1'b0;
    src_valid = 1'b0; src_data = 32'd0; dst_ready = 1'b0;
    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);

    // Streaming with no backpressure at all.
    for (int t = 0; t < 600; t++)
      step(1'b1, $urandom, 1'b1);

    // Independent random backpressure: the part that stalls every state.
    for (int t = 0; t < 40000; t++)
      step($urandom_range(2) != 0, $urandom, $urandom_range(2) != 0);

    // Output jammed shut, so the machine parks in its send state.
    for (int t = 0; t < 400; t++)
      step(1'b1, $urandom, 1'b0);
    for (int t = 0; t < 400; t++)
      step($urandom_range(3) == 0, $urandom, 1'b1);

    for (int t = 0; t < 200; t++) begin
      step(1'b1, $urandom, 1'b0);
      check_rule3();
    end

    if (errors == 0)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d in, %0d out, 0 mismatches",
               cycles, checks, sent, got);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
