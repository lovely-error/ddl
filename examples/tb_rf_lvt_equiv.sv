// Equivalence for a memory with several ports, three ways.
//
// The obvious test is the default build against the `--lvt-bram` build, and it
// is not enough on its own: if the memory MODEL is wrong -- the order writes
// take effect in, what a read of a just-written address sees, which stage a
// value belongs to -- both builds are wrong in the same way and agree with
// each other perfectly. tb_mul3_equiv.sv makes the same point about pipeline
// registers: "a pipeline that mis-registered a crossing value would still
// agree with itself".
//
// So there are three answers compared every item, not two:
//
//   u_plain   the default build: the write ports emitted as the source wrote
//             them, for a target whose memory compiler can make that cell
//   u_lvt     the same source built with `--lvt-bram`: one bank per write
//             port, replicated per read port, with a live value table
//   model     an associative model of what the language says should happen --
//             this item's writes applied in source order, then this item's
//             reads taken
//
// u_plain != u_lvt means the banking is wrong. Both != model means the
// language is wrong. Only the second can be found here at all.
//
// The testbench owns the other two sides of the protocol: a salt producer
// upstream and a salt consumer downstream, each holding its own two bits --
// the same scaffolding tb_mul3_equiv.sv uses.
`timescale 1ns/1ps

module tb_rf_lvt_equiv;

  localparam int REQ_W  = 96;   // wa0,wd0,wa1,wd1,ra0,ra1
  localparam int RESP_W = 128;  // x,y,sum,dif

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  logic [1:0]        tb_wsalt, tb_rsalt;
  logic [REQ_W-1:0]  tb_e [0:1];
  logic              src_valid;
  logic [REQ_W-1:0]  src_data;
  logic              dst_ready;

  logic [1:0]          pl_src_rsalt, lv_src_rsalt;
  logic [1:0]          pl_dst_wsalt, lv_dst_wsalt;
  logic [2*RESP_W-1:0] pl_dst_data,  lv_dst_data;

  wire tb_widx = tb_wsalt[0] ^ tb_wsalt[1];
  wire tb_ridx = tb_rsalt[0] ^ tb_rsalt[1];
  // Held back by the slower of the two, so a divergence is theirs.
  wire tb_full = (tb_wsalt == ~pl_src_rsalt) || (tb_wsalt == ~lv_src_rsalt);
  wire push    = src_valid && !tb_full;

  wire pl_empty = (pl_dst_wsalt == tb_rsalt);
  wire lv_empty = (lv_dst_wsalt == tb_rsalt);
  wire [RESP_W-1:0] pl_item = tb_ridx ? pl_dst_data[2*RESP_W-1:RESP_W] : pl_dst_data[RESP_W-1:0];
  wire [RESP_W-1:0] lv_item = tb_ridx ? lv_dst_data[2*RESP_W-1:RESP_W] : lv_dst_data[RESP_W-1:0];
  wire pop = dst_ready && !pl_empty && !lv_empty;

  rf_lvt u_plain (
      .clk(clk), .rst_n(rst_n),
      .req_wsalt(tb_wsalt), .req_rsalt(pl_src_rsalt), .req_data({tb_e[1], tb_e[0]}),
      .resp_wsalt(pl_dst_wsalt), .resp_rsalt(tb_rsalt), .resp_data(pl_dst_data)
  );

  rf_lvt_lvt u_lvt (
      .clk(clk), .rst_n(rst_n),
      .req_wsalt(tb_wsalt), .req_rsalt(lv_src_rsalt), .req_data({tb_e[1], tb_e[0]}),
      .resp_wsalt(lv_dst_wsalt), .resp_rsalt(tb_rsalt), .resp_data(lv_dst_data)
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

  // ---- the model ----------------------------------------------------------
  //
  // Undriven at the start, exactly as the `bram` is: an address nothing has
  // written reads as X in all three, and X === X, so the comparison neither
  // passes vacuously nor fails on it.
  logic [31:0] model [0:255];

  int errors = 0, checks = 0, cycles = 0, in_n = 0, out_n = 0, stalls = 0;
  int fwd_hits = 0, w_collide = 0, defined_out = 0;
  logic [RESP_W-1:0] expect_q [$];

  logic              held = 1'b0;
  logic [RESP_W-1:0] held_item;
  logic [1:0]        prev_wsalt;

  function automatic logic [1:0] gray_next(input logic [1:0] g);
    return (g[0] ^ g[1]) ? {~g[1], g[0]} : {g[1], ~g[0]};
  endfunction

  // Addresses mostly from a small pool, so that two writes hitting one address
  // and a read hitting an address this item just wrote are the common case
  // rather than a one-in-256 accident. Forwarding and write priority are the
  // whole point, and uniform addresses would almost never exercise them.
  function automatic logic [7:0] pick_addr();
    return ($urandom_range(3) == 0) ? 8'($urandom) : 8'($urandom_range(7));
  endfunction

  function automatic logic [REQ_W-1:0] mk_req();
    logic [7:0]  wa0, wa1, ra0, ra1;
    logic [31:0] wd0, wd1;
    wa0 = pick_addr(); wa1 = pick_addr();
    ra0 = pick_addr(); ra1 = pick_addr();
    wd0 = $urandom;    wd1 = $urandom;
    // The struct packs first field highest, the way SystemVerilog packs one,
    // so this concatenation IS the payload.
    return {wa0, wd0, wa1, wd1, ra0, ra1};
  endfunction

  // What the language says this item produces: its own two writes applied in
  // source order -- so a second write to one address wins -- and then its own
  // two reads, which therefore see them.
  function automatic logic [RESP_W-1:0] model_step(input logic [REQ_W-1:0] req);
    logic [7:0]  wa0, wa1, ra0, ra1;
    logic [31:0] wd0, wd1, x, y, sum, dif;
    {wa0, wd0, wa1, wd1, ra0, ra1} = req;
    if (wa0 == wa1) w_collide++;
    if (ra0 == wa0 || ra0 == wa1 || ra1 == wa0 || ra1 == wa1) fwd_hits++;
    model[wa0] = wd0;
    model[wa1] = wd1;
    x   = model[ra0];
    y   = model[ra1];
    sum = x + y;
    dif = x - y;
    return {x, y, sum, dif};
  endfunction

  task automatic compare();
    checks++;
    if (pl_src_rsalt !== lv_src_rsalt) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH src_rsalt @%0d: plain=%b lvt=%b", cycles, pl_src_rsalt, lv_src_rsalt);
    end
    if (pl_dst_wsalt !== lv_dst_wsalt) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH dst_wsalt @%0d: plain=%b lvt=%b", cycles, pl_dst_wsalt, lv_dst_wsalt);
    end
    if (!lv_empty && (pl_item !== lv_item)) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH item @%0d:\n  plain=%032x\n  lvt  =%032x", cycles, pl_item, lv_item);
    end

    // A published entry is frozen while we are owed it. This is where a read
    // register that free-ran through a stall would show: `_q` and the
    // forwarding registers are all tied to `shift`, and if one were not, the
    // held entry would change under a jammed sink.
    if (held) begin
      checks++;
      if (lv_dst_wsalt !== prev_wsalt && lv_dst_wsalt !== gray_next(prev_wsalt)) begin
        errors++;
        if (errors <= 5) $display("RULE2 wsalt jumped @%0d", cycles);
      end
      if (lv_item !== held_item) begin
        errors++;
        if (errors <= 5) $display("RULE2 entry changed @%0d", cycles);
      end
    end
  endtask

  task automatic step(logic v, logic [REQ_W-1:0] d, logic r);
    src_valid = v; src_data = d; dst_ready = r;
    #1 compare();

    if (push) begin
      expect_q.push_back(model_step(d));
      in_n++;
    end
    if (pop) begin
      automatic logic [RESP_W-1:0] want = expect_q.pop_front();
      checks++;
      if (lv_item !== want) begin
        errors++;
        if (errors <= 5)
          $display("WRONG ITEM @%0d:\n  got  %032x\n  want %032x", cycles, lv_item, want);
      end
      // Items whose reads landed on addresses something had written: the ones
      // that say the memory works rather than that X equals X.
      if (^want !== 1'bx) defined_out++;
      out_n++;
    end
    if (src_valid && tb_full) stalls++;

    held       = !lv_empty && !pop;
    held_item  = lv_item;
    prev_wsalt = lv_dst_wsalt;
    @(posedge clk);
    cycles++;
  endtask

  initial begin
    rst_n = 1'b0;
    src_valid = 1'b0; src_data = '0; dst_ready = 1'b0;
    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);

    // Full rate: one item in and one out every cycle once the pipe fills, so
    // four items are in flight and each read has to arrive beside its own.
    for (int t = 0; t < 800; t++)
      step(1'b1, mk_req(), 1'b1);

    // Bubbles in, so the validity chain carries gaps and a read enable that
    // fired on a bubble would put the wrong value in `_q`.
    for (int t = 0; t < 4000; t++)
      step($urandom_range(2) != 0, mk_req(), 1'b1);

    // Independent backpressure on both sides.
    for (int t = 0; t < 40000; t++)
      step($urandom_range(2) != 0, mk_req(), $urandom_range(2) != 0);

    // Sink jammed shut: the pipeline fills and must hold everything in place,
    // including the memory's output registers and the forwarding registers.
    for (int t = 0; t < 300; t++)
      step(1'b1, mk_req(), 1'b0);
    for (int t = 0; t < 300; t++)
      step($urandom_range(3) == 0, mk_req(), 1'b1);

    // A testbench must not pass by saying nothing: it has to have moved items,
    // stalled, forwarded a write to a read, collided two writes on one
    // address, and produced answers that are not all X.
    if (errors == 0 && out_n > 5000 && stalls > 500
        && fwd_hits > 500 && w_collide > 500 && defined_out > 5000)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d in, %0d out, %0d stalls, %0d forwarded, %0d write collisions, 0 mismatches",
               cycles, checks, in_n, out_n, stalls, fwd_hits, w_collide);
    else if (errors == 0)
      $display("TB_FAIL  vacuous: %0d out, %0d stalls, %0d forwarded, %0d collisions, %0d defined",
               out_n, stalls, fwd_hits, w_collide, defined_out);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
