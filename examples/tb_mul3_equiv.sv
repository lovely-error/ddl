// Equivalence for a generated pipeline.
//
// Beyond diffing against the hand-written version, an independent model checks
// that every item leaving carries 4x the value that went in -- a pipeline that
// mis-registered a crossing value would still agree with itself but would pair
// an item with a neighbour's data, so the value check is what catches it.
//
// The testbench owns the other two sides of the protocol: a salt producer
// upstream and a salt consumer downstream, each holding its own two bits.
`timescale 1ns/1ps

module tb_mul3_equiv;

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  logic [1:0]  tb_wsalt, tb_rsalt;
  logic [15:0] tb_e [0:1];
  logic        src_valid;   // "offer"
  logic [15:0] src_data;
  logic        dst_ready;   // "accept"

  logic [1:0]  ref_src_rsalt, ddl_src_rsalt;
  logic [1:0]  ref_dst_wsalt, ddl_dst_wsalt;
  logic [63:0] ref_dst_data, ddl_dst_data;

  wire tb_widx = tb_wsalt[0] ^ tb_wsalt[1];
  wire tb_ridx = tb_rsalt[0] ^ tb_rsalt[1];
  // Held back by the slower of the two, so a divergence is theirs rather than
  // the testbench's.
  wire tb_full = (tb_wsalt == ~ref_src_rsalt) || (tb_wsalt == ~ddl_src_rsalt);
  wire push    = src_valid && !tb_full;

  wire ref_empty = (ref_dst_wsalt == tb_rsalt);
  wire ddl_empty = (ddl_dst_wsalt == tb_rsalt);
  wire [31:0] ref_item = tb_ridx ? ref_dst_data[63:32] : ref_dst_data[31:0];
  wire [31:0] ddl_item = tb_ridx ? ddl_dst_data[63:32] : ddl_dst_data[31:0];
  wire pop = dst_ready && !ddl_empty && !ref_empty;

  mul3_ref u_ref (
      .clk(clk), .rst_n(rst_n),
      .src_wsalt(tb_wsalt), .src_rsalt(ref_src_rsalt), .src_data({tb_e[1], tb_e[0]}),
      .dst_wsalt(ref_dst_wsalt), .dst_rsalt(tb_rsalt), .dst_data(ref_dst_data)
  );

  mul3 u_ddl (
      .clk(clk), .rst_n(rst_n),
      .src_wsalt(tb_wsalt), .src_rsalt(ddl_src_rsalt), .src_data({tb_e[1], tb_e[0]}),
      .dst_wsalt(ddl_dst_wsalt), .dst_rsalt(tb_rsalt), .dst_data(ddl_dst_data)
  );

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      tb_wsalt <= 2'b00;
      tb_rsalt <= 2'b00;
    end else begin
      if (push) begin
        tb_e[tb_widx] <= src_data;
        tb_wsalt[tb_widx] <= ~tb_wsalt[tb_widx];
      end
      if (pop) tb_rsalt[tb_ridx] <= ~tb_rsalt[tb_ridx];
    end
  end

  int errors = 0, checks = 0, cycles = 0, in_n = 0, out_n = 0, stalls = 0;
  logic [31:0] expect_q[$];

  logic        held = 1'b0;
  logic [31:0] held_item;
  logic [1:0]  prev_wsalt;

  // 00 -> 01 -> 11 -> 10 -> 00; toggle the bit the index names.
  function automatic logic [1:0] gray_next(input logic [1:0] g);
    return (g[0] ^ g[1]) ? {~g[1], g[0]} : {g[1], ~g[0]};
  endfunction

  task automatic compare();
    checks++;
    if (ref_src_rsalt !== ddl_src_rsalt) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH src_rsalt @%0d: ref=%b ddl=%b", cycles, ref_src_rsalt, ddl_src_rsalt);
    end
    if (ref_dst_wsalt !== ddl_dst_wsalt) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH dst_wsalt @%0d: ref=%b ddl=%b", cycles, ref_dst_wsalt, ddl_dst_wsalt);
    end
    if (!ddl_empty && (ref_item !== ddl_item)) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH item @%0d: ref=%08x ddl=%08x", cycles, ref_item, ddl_item);
    end

    // Rule 2: while we are owed an entry, that entry is frozen and `wsalt` may
    // advance at most one gray step -- the producer filling the OTHER entry,
    // which is legal and is the two-deep-ness working.
    if (held) begin
      checks++;
      if (ddl_dst_wsalt !== prev_wsalt && ddl_dst_wsalt !== gray_next(prev_wsalt)) begin
        errors++;
        if (errors <= 5) $display("RULE2 wsalt jumped @%0d", cycles);
      end
      if (ddl_item !== held_item) begin
        errors++;
        if (errors <= 5) $display("RULE2 entry changed @%0d", cycles);
      end
    end
  endtask

  task automatic step(logic v, logic [15:0] d, logic r);
    src_valid = v; src_data = d; dst_ready = r;
    #1 compare();

    // Items enter in order and must leave in order. The first doubling is
    // 16-bit and wraps -- the DDL says `let doubled: u16 = a + a`, and the
    // widening happens only in the next stage -- so the model has to wrap too.
    if (push) begin
      automatic logic [15:0] doubled = d + d;
      expect_q.push_back({16'd0, doubled} << 1);
      in_n++;
    end
    if (pop) begin
      automatic logic [31:0] want = expect_q.pop_front();
      checks++;
      if (ddl_item !== want) begin
        errors++;
        if (errors <= 5)
          $display("WRONG ITEM @%0d: got %08x want %08x", cycles, ddl_item, want);
      end
      out_n++;
    end
    if (src_valid && tb_full) stalls++;

    held       = !ddl_empty && !pop;
    held_item  = ddl_item;
    prev_wsalt = ddl_dst_wsalt;
    @(posedge clk);
    cycles++;
  endtask

  // Rule 3, BOTH WAYS: driving one side's salt to an arbitrary value within a
  // cycle must not move anything the other side publishes. The second half has
  // no counterpart under valid/ready, where nothing forbade `ready` moving
  // with `valid`.
  task automatic check_rule3();
    logic [1:0]  w0, r0, save_r, save_w;
    logic [63:0] d0;
    save_r = tb_rsalt; save_w = tb_wsalt;

    tb_rsalt = 2'b00; #1; w0 = ddl_dst_wsalt; d0 = ddl_dst_data;
    tb_rsalt = 2'b11; #1;
    checks++;
    if (ddl_dst_wsalt !== w0 || ddl_dst_data !== d0) begin
      errors++;
      if (errors <= 5) $display("RULE3 producer moved with rsalt @%0d", cycles);
    end
    tb_rsalt = save_r;

    tb_wsalt = 2'b00; #1; r0 = ddl_src_rsalt;
    tb_wsalt = 2'b11; #1;
    checks++;
    if (ddl_src_rsalt !== r0) begin
      errors++;
      if (errors <= 5) $display("RULE3 consumer moved with wsalt @%0d", cycles);
    end
    // Restored before the edge: arbitrary salts make full/empty meaningless,
    // and letting one clock in would corrupt both sides' state.
    tb_wsalt = save_w;
    #1;
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

    if (errors == 0 && out_n > 5000 && stalls > 500)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d in, %0d out, %0d stalls, 0 mismatches",
               cycles, checks, in_n, out_n, stalls);
    else if (errors == 0)
      $display("TB_FAIL  vacuous: %0d out, %0d stalls", out_n, stalls);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
