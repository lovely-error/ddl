// Equivalence for a multi-port memory in a `process`, three ways.
//
// tb_rf_lvt_equiv.sv does this for a `sequence`. Both are needed, because the
// two constructs assemble the ports through different code and the results are
// a different shape: a process settles its write ports per state and muxes all
// its reads onto ONE port, so `--lvt-bram` here builds banks with one replica
// each where the pipeline's has two. A test of one leaves the other unwritten.
//
// Three answers compared every item, not two:
//
//   u_plain   the default build: the write ports as the source wrote them
//   u_lvt     the same source with `--lvt-bram`: a bank per write port and a
//             live value table
//   model     what the language says should happen -- this item's writes in
//             source order, then this item's reads
//
// u_plain != u_lvt means the banking is wrong. Both != model means the
// language is wrong, and only the model can say so: two builds of one wrong
// idea agree with each other perfectly.
`timescale 1ns/1ps

module tb_rf_lvt_proc_equiv;

  localparam int REQ_W  = 96;  // wa0,wd0,wa1,wd1,ra0,ra1
  localparam int RESP_W = 64;  // x,y

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
  wire tb_full = (tb_wsalt == ~pl_src_rsalt) || (tb_wsalt == ~lv_src_rsalt);
  wire push    = src_valid && !tb_full;

  wire pl_empty = (pl_dst_wsalt == tb_rsalt);
  wire lv_empty = (lv_dst_wsalt == tb_rsalt);
  wire [RESP_W-1:0] pl_item = tb_ridx ? pl_dst_data[2*RESP_W-1:RESP_W] : pl_dst_data[RESP_W-1:0];
  wire [RESP_W-1:0] lv_item = tb_ridx ? lv_dst_data[2*RESP_W-1:RESP_W] : lv_dst_data[RESP_W-1:0];
  wire pop = dst_ready && !pl_empty && !lv_empty;

  rf_lvt_proc u_plain (
      .clk(clk), .rst_n(rst_n),
      .cmd_wsalt(tb_wsalt), .cmd_rsalt(pl_src_rsalt), .cmd_data({tb_e[1], tb_e[0]}),
      .rd_wsalt(pl_dst_wsalt), .rd_rsalt(tb_rsalt), .rd_data(pl_dst_data)
  );

  rf_lvt_proc_lvt u_lvt (
      .clk(clk), .rst_n(rst_n),
      .cmd_wsalt(tb_wsalt), .cmd_rsalt(lv_src_rsalt), .cmd_data({tb_e[1], tb_e[0]}),
      .rd_wsalt(lv_dst_wsalt), .rd_rsalt(tb_rsalt), .rd_data(lv_dst_data)
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

  // Undriven at the start, exactly as the `bram` is, so an address nothing has
  // written reads as X in all three and X === X.
  logic [31:0] model [0:255];

  int errors = 0, checks = 0, cycles = 0, in_n = 0, out_n = 0, stalls = 0;
  int read_after_write = 0, w_collide = 0, defined_out = 0;
  logic [RESP_W-1:0] expect_q [$];

  logic              held = 1'b0;
  logic [RESP_W-1:0] held_item;
  logic [1:0]        prev_wsalt;

  function automatic logic [1:0] gray_next(input logic [1:0] g);
    return (g[0] ^ g[1]) ? {~g[1], g[0]} : {g[1], ~g[0]};
  endfunction

  // Mostly from a small pool, so two writes landing on one address and a read
  // of an address this item just wrote are the common case rather than a
  // one-in-256 accident.
  function automatic logic [7:0] pick_addr();
    return ($urandom_range(3) == 0) ? 8'($urandom) : 8'($urandom_range(7));
  endfunction

  function automatic logic [REQ_W-1:0] mk_req();
    logic [7:0]  wa0, wa1, ra0, ra1;
    logic [31:0] wd0, wd1;
    wa0 = pick_addr(); wa1 = pick_addr();
    ra0 = pick_addr(); ra1 = pick_addr();
    wd0 = $urandom;    wd1 = $urandom;
    return {wa0, wd0, wa1, wd1, ra0, ra1};
  endfunction

  // This item's writes in source order -- so a second write to one address
  // wins -- and then its own reads, which therefore see them. In a process the
  // reads are scheduled into states after the writes, so the array has already
  // been updated when they look; that this comes out the same as the
  // pipeline's forwarded answer is the claim being checked.
  function automatic logic [RESP_W-1:0] model_step(input logic [REQ_W-1:0] req);
    logic [7:0]  wa0, wa1, ra0, ra1;
    logic [31:0] wd0, wd1, x, y;
    {wa0, wd0, wa1, wd1, ra0, ra1} = req;
    if (wa0 == wa1) w_collide++;
    if (ra0 == wa0 || ra0 == wa1 || ra1 == wa0 || ra1 == wa1) read_after_write++;
    model[wa0] = wd0;
    model[wa1] = wd1;
    x = model[ra0];
    y = model[ra1];
    return {x, y};
  endfunction

  task automatic compare();
    checks++;
    if (pl_src_rsalt !== lv_src_rsalt) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH cmd_rsalt @%0d: plain=%b lvt=%b", cycles, pl_src_rsalt, lv_src_rsalt);
    end
    if (pl_dst_wsalt !== lv_dst_wsalt) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH rd_wsalt @%0d: plain=%b lvt=%b", cycles, pl_dst_wsalt, lv_dst_wsalt);
    end
    if (!lv_empty && (pl_item !== lv_item)) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH item @%0d: plain=%016x lvt=%016x", cycles, pl_item, lv_item);
    end

    // A published entry is frozen while we are owed it: where a read register
    // that kept going through a stall would show.
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
          $display("WRONG ITEM @%0d: got %016x want %016x", cycles, lv_item, want);
      end
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

    // A process takes several cycles per item, so it is offered one every
    // cycle and takes them at its own rate.
    for (int t = 0; t < 4000; t++)
      step(1'b1, mk_req(), 1'b1);

    // Gaps on the way in, so the machine waits in its receive state.
    for (int t = 0; t < 20000; t++)
      step($urandom_range(2) != 0, mk_req(), 1'b1);

    // Independent backpressure on both sides.
    for (int t = 0; t < 40000; t++)
      step($urandom_range(2) != 0, mk_req(), $urandom_range(2) != 0);

    // Sink jammed shut: the machine fills and must hold what it has, the
    // memory's output register included.
    for (int t = 0; t < 300; t++)
      step(1'b1, mk_req(), 1'b0);
    for (int t = 0; t < 300; t++)
      step($urandom_range(3) == 0, mk_req(), 1'b1);

    // A testbench must not pass by saying nothing.
    if (errors == 0 && out_n > 3000 && stalls > 500
        && read_after_write > 500 && w_collide > 500 && defined_out > 3000)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d in, %0d out, %0d stalls, %0d read-after-write, %0d write collisions, 0 mismatches",
               cycles, checks, in_n, out_n, stalls, read_after_write, w_collide);
    else if (errors == 0)
      $display("TB_FAIL  vacuous: %0d out, %0d stalls, %0d raw, %0d collisions, %0d defined",
               out_n, stalls, read_after_write, w_collide, defined_out);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
