// Equivalence for a generated channel handshake.
//
// Drives both stages from one stimulus with arbitrary backpressure -- the
// upstream offering and the downstream accepting are both random and
// independent, which is what exercises stalling, draining and the full slot.
//
// The testbench is the OTHER TWO SIDES of the protocol: a salt producer
// upstream and a salt consumer downstream, each holding its own two bits. That
// is a bigger job than driving `valid` and sampling `ready`, and it is also the
// point -- a testbench that could not hold up its end would not be testing a
// protocol, only a pinout.
//
// Also checks the protocol's own rules on the DDL side, since holding them by
// construction is the claim being made:
//   2. an occupied entry is immutable and `wsalt` only ever advances one gray
//      step -- a producer may neither withdraw an offer nor alter it.
//   3. neither side's published wires move when the other's do. Under
//      valid/ready only one direction could be checked; here both can.
//   plus: occupancy is never 3, which is a push into a full buffer.
`timescale 1ns/1ps

module tb_k3g_stage_equiv;

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  // ---- the testbench's producer side --------------------------------------
  logic [1:0]  tb_wsalt;
  logic [31:0] tb_e [0:1];
  logic        offer;
  logic [31:0] offer_data;

  logic [1:0]  ref_iops_rsalt, ddl_iops_rsalt;
  wire         tb_widx = tb_wsalt[0] ^ tb_wsalt[1];
  // The producer must not run ahead of the SLOWER of the two, or they diverge
  // for a reason that is the testbench's fault.
  wire         tb_full = (tb_wsalt == ~ref_iops_rsalt)
                      || (tb_wsalt == ~ddl_iops_rsalt);
  wire         push    = offer && !tb_full;

  // ---- the testbench's consumer side --------------------------------------
  logic [1:0]  tb_rsalt;
  logic        accept;
  logic [1:0]  ref_uops_wsalt, ddl_uops_wsalt;
  logic [97:0] ref_uops_data,  ddl_uops_data;
  wire         tb_ridx  = tb_rsalt[0] ^ tb_rsalt[1];
  wire         ref_empty = (ref_uops_wsalt == tb_rsalt);
  wire         ddl_empty = (ddl_uops_wsalt == tb_rsalt);
  wire  [48:0] ref_item = tb_ridx ? ref_uops_data[97:49] : ref_uops_data[48:0];
  wire  [48:0] ddl_item = tb_ridx ? ddl_uops_data[97:49] : ddl_uops_data[48:0];
  wire         pop      = accept && !ddl_empty && !ref_empty;

  k3g_stage_ref u_ref (
      .clk(clk), .rst_n(rst_n),
      .iops_wsalt(tb_wsalt), .iops_rsalt(ref_iops_rsalt),
      .iops_data({tb_e[1], tb_e[0]}),
      .uops_wsalt(ref_uops_wsalt), .uops_rsalt(tb_rsalt), .uops_data(ref_uops_data)
  );

  k3g_stage u_ddl (
      .clk(clk), .rst_n(rst_n),
      .iops_wsalt(tb_wsalt), .iops_rsalt(ddl_iops_rsalt),
      .iops_data({tb_e[1], tb_e[0]}),
      .uops_wsalt(ddl_uops_wsalt), .uops_rsalt(tb_rsalt), .uops_data(ddl_uops_data)
  );

  int errors = 0, checks = 0, cycles = 0, transfers = 0, stalls = 0;

  // Rule 2 state, watched on the DDL side.
  logic        held      = 1'b0;
  logic [48:0] held_item;
  logic [1:0]  prev_wsalt;

  // 00 -> 01 -> 11 -> 10 -> 00
  function automatic logic [1:0] gray_next(input logic [1:0] g);
    // Toggle the bit the index names: index 0 flips bit 0.
    return (g[0] ^ g[1]) ? {~g[1], g[0]} : {g[1], ~g[0]};
  endfunction

  // Gray position, so occupancy can be measured.
  function automatic int gpos(input logic [1:0] g);
    case (g)
      2'b00: return 0;
      2'b01: return 1;
      2'b11: return 2;
      default: return 3;
    endcase
  endfunction

  task automatic compare();
    checks++;
    if (ref_iops_rsalt !== ddl_iops_rsalt) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH iops_rsalt @%0d: ref=%b ddl=%b",
                 cycles, ref_iops_rsalt, ddl_iops_rsalt);
    end
    if (ref_uops_wsalt !== ddl_uops_wsalt) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH uops_wsalt @%0d: ref=%b ddl=%b",
                 cycles, ref_uops_wsalt, ddl_uops_wsalt);
    end
    // The entry we are owed has to agree while it is being offered.
    if (!ddl_empty && (ref_item !== ddl_item)) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH item @%0d:\n  ref=%013x\n  ddl=%013x",
                 cycles, ref_item, ddl_item);
    end

    // Occupancy is 0, 1 or 2 and never 3. Three means a push into a full
    // buffer, and it fires on the cycle it happens rather than N items later.
    checks++;
    if (((gpos(ddl_uops_wsalt) - gpos(tb_rsalt)) & 3) == 3) begin
      errors++;
      if (errors <= 5) $display("OCCUPANCY 3 on uops @%0d", cycles);
    end
    if (((gpos(tb_wsalt) - gpos(ddl_iops_rsalt)) & 3) == 3) begin
      errors++;
      if (errors <= 5) $display("OCCUPANCY 3 on iops @%0d", cycles);
    end

    // Rule 2, as the salt protocol states it: while we are owed an entry, that
    // entry is frozen, and `wsalt` may only advance one gray step -- which is
    // the producer filling the OTHER entry, and is legal.
    if (held) begin
      checks++;
      if (ddl_uops_wsalt !== prev_wsalt && ddl_uops_wsalt !== gray_next(prev_wsalt)) begin
        errors++;
        if (errors <= 5) $display("RULE2 wsalt jumped @%0d", cycles);
      end
      if (ddl_item !== held_item) begin
        errors++;
        if (errors <= 5) $display("RULE2 entry changed @%0d", cycles);
      end
    end
  endtask

  task automatic step(logic v, logic [31:0] d, logic r);
    offer = v; offer_data = d; accept = r;
    #1 compare();
    if (pop) transfers++;
    if (offer && tb_full) stalls++;
    held      = !ddl_empty && !pop;
    held_item = ddl_item;
    prev_wsalt = ddl_uops_wsalt;
    @(posedge clk);
    cycles++;
  endtask

  // Rule 3, BOTH WAYS. Driving one side's salt to an arbitrary value within a
  // cycle must not move anything the other side publishes. The second half has
  // no counterpart under valid/ready -- nothing there forbade `ready` moving
  // with `valid` -- and it is what catches an `rsalt` that accidentally became
  // a driven value rather than a register.
  task automatic check_rule3();
    logic [1:0]  w0, r0;
    logic [97:0] d0;
    logic [1:0]  save_r, save_w;
    save_r = tb_rsalt; save_w = tb_wsalt;

    tb_rsalt = 2'b00; #1; w0 = ddl_uops_wsalt; d0 = ddl_uops_data;
    tb_rsalt = 2'b11; #1;
    checks++;
    if (ddl_uops_wsalt !== w0 || ddl_uops_data !== d0) begin
      errors++;
      if (errors <= 5) $display("RULE3 producer moved with rsalt @%0d", cycles);
    end
    tb_rsalt = save_r;

    tb_wsalt = 2'b00; #1; r0 = ddl_iops_rsalt;
    tb_wsalt = 2'b11; #1;
    checks++;
    if (ddl_iops_rsalt !== r0) begin
      errors++;
      if (errors <= 5) $display("RULE3 consumer moved with wsalt @%0d", cycles);
    end
    // Restored before the edge: arbitrary salts make full/empty meaningless,
    // and letting one clock in would corrupt both sides' state.
    tb_wsalt = save_w;
    #1;
  endtask

  // The producer and consumer the testbench owns.
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      tb_wsalt <= 2'b00;
      tb_rsalt <= 2'b00;
    end else begin
      if (push) begin
        tb_e[tb_widx] <= offer_data;
        tb_wsalt[tb_widx] <= ~tb_wsalt[tb_widx];
      end
      if (pop) tb_rsalt[tb_ridx] <= ~tb_rsalt[tb_ridx];
    end
  end

  initial begin
    rst_n = 1'b0;
    offer = 1'b0; offer_data = 32'd0; accept = 1'b0;
    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);

    // Every kind, with the downstream always accepting.
    for (int k = 0; k < 8; k++)
      for (int t = 0; t < 8; t++)
        step(1'b1, {3'($urandom), k[2:0], 5'($urandom), 5'($urandom), 16'($urandom)},
             1'b1);

    // Independent random backpressure on both sides. This is the part that
    // exercises the full buffer, the stall and the drain.
    for (int t = 0; t < 40000; t++)
      step($urandom_range(2) != 0, $urandom, $urandom_range(2) != 0);

    // Downstream jammed shut, so both entries fill and stay full.
    for (int t = 0; t < 500; t++)
      step(1'b1, $urandom, 1'b0);

    // Then drained one item at a time.
    for (int t = 0; t < 500; t++)
      step($urandom_range(3) == 0, $urandom, 1'b1);

    // Rule 3, at several points in the cycle.
    for (int t = 0; t < 200; t++) begin
      step(1'b1, $urandom, 1'b0);
      check_rule3();
    end

    // A run where nothing ever stalled would prove nothing about the buffer.
    if (errors == 0 && transfers > 5000 && stalls > 500)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d transfers, %0d stalls, 0 mismatches",
               cycles, checks, transfers, stalls);
    else if (errors == 0)
      $display("TB_FAIL  vacuous: %0d transfers, %0d stalls", transfers, stalls);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
