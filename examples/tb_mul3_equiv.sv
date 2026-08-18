// Equivalence for a generated pipeline.
//
// Beyond diffing against the hand-written version, an independent model checks
// that every item leaving carries 4x the value that went in -- a pipeline that
// mis-registered a crossing value would still agree with itself but would pair
// an item with a neighbour's data, so the value check is what catches it.
`timescale 1ns/1ps

module tb_mul3_equiv;

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  logic        src_valid;
  logic [15:0] src_data;
  logic        dst_ready;

  logic        ref_src_ready, ref_dst_valid;
  logic [31:0] ref_dst_data;
  logic        ddl_src_ready, ddl_dst_valid;
  logic [31:0] ddl_dst_data;

  mul3_ref u_ref (
      .clk(clk), .rst_n(rst_n),
      .src_valid(src_valid), .src_ready(ref_src_ready), .src_data(src_data),
      .dst_valid(ref_dst_valid), .dst_ready(dst_ready), .dst_data(ref_dst_data)
  );

  mul3 u_ddl (
      .clk(clk), .rst_n(rst_n),
      .src_valid(src_valid), .src_ready(ddl_src_ready), .src_data(src_data),
      .dst_valid(ddl_dst_valid), .dst_ready(dst_ready), .dst_data(ddl_dst_data)
  );

  int errors = 0, checks = 0, cycles = 0, in_n = 0, out_n = 0;
  logic [31:0] expect_q[$];

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

  task automatic step(logic v, logic [15:0] d, logic r);
    src_valid = v; src_data = d; dst_ready = r;
    #1 compare();

    // Items enter in order and must leave in order. The first doubling is
    // 16-bit and wraps -- the DDL says `let doubled: i16 = a + a`, and the
    // widening happens only in the next stage -- so the model has to wrap too.
    if (ddl_src_ready && v) begin
      automatic logic [15:0] doubled = d + d;
      expect_q.push_back({16'd0, doubled} << 1);
      in_n++;
    end
    if (ddl_dst_valid && r) begin
      automatic logic [31:0] want = expect_q.pop_front();
      checks++;
      if (ddl_dst_data !== want) begin
        errors++;
        if (errors <= 5)
          $display("WRONG ITEM @%0d: got %08x want %08x", cycles, ddl_dst_data, want);
      end
      out_n++;
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
    src_valid = 1'b0; src_data = 16'd0; dst_ready = 1'b0;
    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);

    // Full rate: one item in and one out every cycle once the pipe fills.
    for (int t = 0; t < 800; t++)
      step(1'b1, 16'($urandom), 1'b1);

    // Bubbles on the way in, so the validity chain has to carry gaps.
    for (int t = 0; t < 4000; t++)
      step($urandom_range(2) != 0, 16'($urandom), 1'b1);

    // Independent backpressure on both sides.
    for (int t = 0; t < 40000; t++)
      step($urandom_range(2) != 0, 16'($urandom), $urandom_range(2) != 0);

    // Sink jammed shut: the pipeline fills and must hold everything in place.
    for (int t = 0; t < 300; t++)
      step(1'b1, 16'($urandom), 1'b0);
    for (int t = 0; t < 300; t++)
      step($urandom_range(3) == 0, 16'($urandom), 1'b1);

    for (int t = 0; t < 200; t++) begin
      step(1'b1, 16'($urandom), 1'b0);
      check_rule3();
    end

    if (errors == 0)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d in, %0d out, 0 mismatches",
               cycles, checks, in_n, out_n);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
