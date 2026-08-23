// The hand-written k2g_decode.sv against the DDL-generated k2g_decode_ddl.v.
//
// These no longer have the same ports, and that is the point of the test. The
// SystemVerilog carries `cp_valid` / `accept` / `uop_valid` / `hold`, four
// signals hand-wired into a protocol. The DDL declares `cps: buffer in i16`
// and `uop: buffer out uop_t` and the compiler writes the same protocol --
// with `hold` GONE, because `hold` only ever existed to stand in for the
// back-pressure `uop_valid` had no `ready` to carry.
//
// So the two are held in lockstep on the thing that must agree -- which code
// point is consumed on which cycle -- and compared on the thing that matters:
// the SEQUENCE of micro-ops they produce. The DDL's output is registered
// (channel rule 3 by construction), so a cycle-by-cycle comparison of
// `uop_valid` would only be measuring that one pipeline stage, not whether the
// decode is the same.
//
// Lockstep is arranged by driving the reference's `cp_valid` from the DDL's
// own transfer. When the DDL's output slot is full and the sink is refusing,
// `cps_ready` falls, no code point moves, and the reference does not advance
// either -- which is exactly the behaviour `hold` was wired up to produce.
//
// The redirect is IN THE STREAM. The SystemVerilog takes it on a separate
// `flush` wire that lands whether or not a code point is consumed; the DDL
// carries a `restart` bit on the item, so it lands with the transfer. The
// reference is driven with `flush = xfer & restart` to match, which is what
// makes the two comparable -- and which is the whole behavioural difference
// the change makes.
`timescale 1ns/1ps
`include "k2g_types.svh"

module tb_k2g_decode_equiv;
  import k2g_pkg::*;
  import k2g_types::*;

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  logic [15:0] cp;
  logic        restart;     // this code point begins a new instruction stream
  logic        offer;       // the producer has a code point available
  logic        uop_ready;   // the consumer will take a micro-op

  logic         ref_accept, ref_uop_valid;
  uop_t         ref_uop;

  // ---- the testbench is the salt producer on `cps` -------------------------
  logic [1:0]  tb_wsalt;
  logic [16:0] tb_e [0:1];        // {cp, restart}, first field high
  logic [1:0]  ddl_cps_rsalt;
  wire  tb_widx = tb_wsalt[0] ^ tb_wsalt[1];
  wire  tb_full = (tb_wsalt == ~ddl_cps_rsalt);
  wire  push    = offer && !tb_full;

  // ---- and the salt consumer on `uop` --------------------------------------
  logic [1:0]   tb_rsalt;
  logic [1:0]   ddl_uop_wsalt;
  logic [253:0] ddl_uop_pair;
  wire  tb_ridx  = tb_rsalt[0] ^ tb_rsalt[1];
  wire  ddl_empty = (ddl_uop_wsalt == tb_rsalt);
  wire [126:0] ddl_uop = tb_ridx ? ddl_uop_pair[253:127] : ddl_uop_pair[126:0];
  wire  pop = uop_ready && !ddl_empty;

  // THE BRIDGE. `push` is NOT the reference's `cp_valid`: push is when an item
  // enters the pipe, and under salt it can sit in an entry for cycles before
  // the DDL looks at it. The reference has to be driven from the DDL's own
  // consume decision, which is `cps_take` -- the enable on its `rsalt` toggle,
  // and the direct heir of the `ddl_cps_ready` this bridge used to use.
  wire xfer = u_ddl.cps_take;
  // And the redirect belongs to the entry BEING CONSUMED, not to whatever the
  // testbench happens to be offering this cycle. Getting this wrong flushes on
  // the wrong code point and reads as a decode bug.
  wire [16:0] taken = u_ddl.cps_ridx ? tb_e[1] : tb_e[0];
  wire [15:0] taken_cp = taken[16:1];
  wire        flush    = xfer & taken[0];

  k2g_decode u_ref (
      .clk(clk), .rst_n(rst_n),
      .cp(taken_cp), .cp_valid(xfer), .flush(flush), .hold(1'b0),
      .accept(ref_accept), .uop(ref_uop), .uop_valid(ref_uop_valid)
  );

  k2g_decode_ddl u_ddl (
      .clk(clk), .rst_n(rst_n),
      .cps_wsalt(tb_wsalt), .cps_rsalt(ddl_cps_rsalt),
      .cps_data({tb_e[1], tb_e[0]}),
      .uop_wsalt(ddl_uop_wsalt), .uop_rsalt(tb_rsalt), .uop_data(ddl_uop_pair)
  );

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      tb_wsalt <= 2'b00;
      tb_rsalt <= 2'b00;
    end else begin
      if (push) begin
        tb_e[tb_widx] <= {cp, restart};
        tb_wsalt[tb_widx] <= ~tb_wsalt[tb_widx];
      end
      if (pop) tb_rsalt[tb_ridx] <= ~tb_rsalt[tb_ridx];
    end
  end

  bit armed = 1'b0;
  int diverged_at = -1;
  int errors = 0, checks = 0, cycles = 0;
  int ref_n = 0, ddl_n = 0, compared = 0, stalls = 0;

  logic [126:0] ref_q[$];
  logic [126:0] ddl_q[$];
  int           ref_cyc[$];
  int           ddl_cyc[$];

  logic         held = 1'b0;
  logic [1:0]   prev_wsalt;

  // 00 -> 01 -> 11 -> 10 -> 00; toggle the bit the index names.
  function automatic logic [1:0] gray_next(input logic [1:0] g);
    return (g[0] ^ g[1]) ? {~g[1], g[0]} : {g[1], ~g[0]};
  endfunction

  logic         held_valid_unused = 1'b0;
  logic [126:0] held_uop;

  task automatic drain();
    while (ref_q.size() > 0 && ddl_q.size() > 0) begin
      automatic logic [126:0] a = ref_q.pop_front();
      automatic logic [126:0] b = ddl_q.pop_front();
      automatic int ca = ref_cyc.pop_front();
      automatic int cb = ddl_cyc.pop_front();
      checks++;
      compared++;
      if (a !== b) begin
        errors++;
        if (errors <= 5)
          $display("MISMATCH uop #%0d (ref pushed @%0d, ddl @%0d):\n  ref=%032x\n  ddl=%032x",
                   compared, ca, cb, a, b);
      end
    end
  endtask

  task automatic step(logic [15:0] c, logic v, logic f, logic r);
    cp = c; offer = v; restart = f; uop_ready = r;
    #1;

    // Rule 2: while we are owed an entry, that entry is frozen and `wsalt`
    // may advance at most one gray step -- the producer filling the other one.
    checks++;
    if (held) begin
      if (ddl_uop_wsalt !== prev_wsalt && ddl_uop_wsalt !== gray_next(prev_wsalt)) begin
        errors++;
        if (errors <= 5) $display("RULE2 wsalt jumped @%0d", cycles);
      end
      if (ddl_uop !== held_uop) begin
        errors++;
        if (errors <= 5) $display("RULE2 entry changed @%0d", cycles);
      end
    end

    if (ref_uop_valid) begin
      ref_q.push_back(ref_uop); ref_cyc.push_back(cycles); ref_n++;
    end
    if (pop) begin
      ddl_q.push_back(ddl_uop); ddl_cyc.push_back(cycles); ddl_n++;
    end
    if (offer && tb_full) stalls++;

    if (armed && diverged_at < 0 && (ddl_n > ref_n || ref_n - ddl_n > 1)) begin
      diverged_at = cycles;
      $display("COUNT @%0d ref_n=%0d ddl_n=%0d empty=%b rdy=%b xfer=%b flush=%b offer=%b refv=%b",
               cycles, ref_n, ddl_n, ddl_empty, uop_ready, xfer,
               flush, offer, ref_uop_valid);
    end

    if (armed && diverged_at < 0) begin
      if (u_ref.pfx !== u_ddl.pfx || u_ref.state !== u_ddl.state) begin
        diverged_at = cycles;
        $display("STATE DIVERGE @%0d cp=%04x offer=%b xfer=%b rdy=%b flush=%b refv=%b empty=%b",
                 cycles, cp, offer, xfer, uop_ready, flush,
                 ref_uop_valid, ddl_empty);
        $display("  ref pfx=%026x state=%0d", u_ref.pfx, u_ref.state);
        $display("  ddl pfx=%026x state=%0d", u_ddl.pfx, u_ddl.state);
      end
    end

    drain();

    held       = !ddl_empty && !pop;
    held_uop   = ddl_uop;
    prev_wsalt = ddl_uop_wsalt;
    @(posedge clk);
    cycles++;
  endtask

  // Rule 3, BOTH WAYS: driving one side's salt within a cycle must not move
  // anything the other publishes.
  task automatic check_rule3();
    logic [1:0]   w0, r0, save_r, save_w;
    logic [253:0] d0;
    save_r = tb_rsalt; save_w = tb_wsalt;

    tb_rsalt = 2'b00; #1; w0 = ddl_uop_wsalt; d0 = ddl_uop_pair;
    tb_rsalt = 2'b11; #1;
    checks++;
    if (ddl_uop_wsalt !== w0 || ddl_uop_pair !== d0) begin
      errors++;
      if (errors <= 5) $display("RULE3 producer moved with rsalt @%0d", cycles);
    end
    tb_rsalt = save_r;

    tb_wsalt = 2'b00; #1; r0 = ddl_cps_rsalt;
    tb_wsalt = 2'b11; #1;
    checks++;
    if (ddl_cps_rsalt !== r0) begin
      errors++;
      if (errors <= 5) $display("RULE3 consumer moved with wsalt @%0d", cycles);
    end
    tb_wsalt = save_w;
    #1;
    uop_ready = 1'b0;
  endtask

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
    cp = 16'd0; offer = 1'b0; restart = 1'b0; uop_ready = 1'b1;
    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);

    armed = 1'b1;

    // 1. Every main opcode on its own, with no prefix state and no stalling.
    for (int op = 0; op < 64; op++)
      for (int a = 0; a < 4; a++)
        step({op[5:0], 5'($urandom), 5'($urandom)}, 1'b1, 1'b0, 1'b1);

    // 2. Prefix chains, where the accumulator and the chain-length fault live.
    //    Long enough to overrun the bound of 7.
    for (int t = 0; t < 3000; t++) begin
      automatic int n = $urandom_range(10);
      for (int i = 0; i < n; i++)
        step(a_prefix(), 1'b1, 1'b0, 1'b1);
      step($urandom, 1'b1, 1'b0, 1'b1);
    end

    // 3. LLC forms, which drive the S_LLC_HI / S_LLC_LO states.
    for (int t = 0; t < 2000; t++) begin
      step({LB_EP1, 5'($urandom), EP1_LLC_B32}, 1'b1, 1'b0, 1'b1);
      step($urandom, 1'b1, 1'b0, 1'b1);
      step($urandom, 1'b1, 1'b0, 1'b1);
      step({LB_EP1, 5'($urandom), EP1_LLC_B8}, 1'b1, 1'b0, 1'b1);
      step($urandom, 1'b1, 1'b0, 1'b1);
    end

    // 4. Random traffic with a sink that refuses often. This is what `hold`
    //    used to be, and nothing in the DDL source mentions it. The redirect
    //    rides in-band, so it only lands on cycles an item actually moves.
    for (int t = 0; t < 60000; t++)
      step($urandom,
           $urandom_range(9) != 0,      // the producer mostly has something
           $urandom_range(19) == 0,     // occasional flush
           $urandom_range(2) != 0);     // the sink refuses about a third

    // 5. Sink jammed shut, so the slot fills and back-pressure reaches the
    //    producer, then released.
    for (int t = 0; t < 300; t++)
      step($urandom, 1'b1, 1'b0, 1'b0);
    for (int t = 0; t < 2000; t++)
      step($urandom, 1'b1, 1'b0, $urandom_range(3) == 0);

    // 6. Reset in the middle of an instruction.
    for (int t = 0; t < 200; t++) begin
      step(a_prefix(), 1'b1, 1'b0, 1'b1);

      // Quiesce first: stop offering and let the slot drain, so the two are
      // level before reset is applied.
      armed = 1'b0;
      step(16'd0, 1'b0, 1'b0, 1'b1);
      step(16'd0, 1'b0, 1'b0, 1'b1);
      rst_n = 1'b0;
      step(16'd0, 1'b0, 1'b0, 1'b1);
      rst_n = 1'b1;
      ref_q.delete();
      ddl_q.delete();
      ref_cyc.delete();
      ddl_cyc.delete();
      armed = 1'b1;
      step($urandom, 1'b1, 1'b0, 1'b1);
    end

    for (int t = 0; t < 200; t++) begin
      step($urandom, 1'b1, 1'b0, 1'b0);
      check_rule3();
    end

    // Drain whatever the DDL still holds, so a micro-op the reference produced
    // and the DDL swallowed cannot hide in the queue.
    for (int t = 0; t < 64; t++)
      step(16'd0, 1'b0, 1'b0, 1'b1);

    if (ref_q.size() != 0 || ddl_q.size() != 0) begin
      errors++;
      $display("LEFTOVER ref=%0d ddl=%0d -- one side produced micro-ops the other did not",
               ref_q.size(), ddl_q.size());
    end
    // Anti-vacuous: a run that compared nothing, or never stalled, proves
    // nothing about either the decode or the generated handshake.
    if (compared < 40000 || stalls < 1000) begin
      $display("TB_FAIL  vacuous: %0d micro-ops compared, %0d stalled cycles", compared, stalls);
      $finish;
    end

    if (errors == 0)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d micro-ops, %0d stalls, 0 mismatches",
               cycles, checks, compared, stalls);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
