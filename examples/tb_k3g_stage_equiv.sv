// Equivalence for a generated channel handshake.
//
// Drives both stages from one stimulus with arbitrary backpressure -- the
// upstream offering and the downstream accepting are both random and
// independent, which is what exercises stalling, draining and the full slot.
//
// Also checks the channel rules themselves on the DDL side, since holding them
// by construction is the claim being made:
//   2. once `valid` is raised it stays raised, with `data` unchanged, until a
//      transfer happens -- a producer may not withdraw an offer.
//   3. `valid` must not depend combinationally on `ready`.
`timescale 1ns/1ps

module tb_k3g_stage_equiv;

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  logic        iops_valid;
  logic [31:0] iops_data;
  logic        uops_ready;

  logic        ref_iops_ready, ref_uops_valid;
  logic [48:0] ref_uops_data;
  logic        ddl_iops_ready, ddl_uops_valid;
  logic [48:0] ddl_uops_data;

  k3g_stage_ref u_ref (
      .clk(clk), .rst_n(rst_n),
      .iops_valid(iops_valid), .iops_ready(ref_iops_ready), .iops_data(iops_data),
      .uops_valid(ref_uops_valid), .uops_ready(uops_ready), .uops_data(ref_uops_data)
  );

  k3g_stage u_ddl (
      .clk(clk), .rst_n(rst_n),
      .iops_valid(iops_valid), .iops_ready(ddl_iops_ready), .iops_data(iops_data),
      .uops_valid(ddl_uops_valid), .uops_ready(uops_ready), .uops_data(ddl_uops_data)
  );

  int errors = 0, checks = 0, cycles = 0, transfers = 0;

  // Rule 2 state, watched on the DDL side.
  logic        held_valid = 1'b0;
  logic [48:0] held_data;

  task automatic compare();
    checks++;
    if (ref_iops_ready !== ddl_iops_ready) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH iops_ready @%0d: ref=%b ddl=%b", cycles, ref_iops_ready, ddl_iops_ready);
    end
    if (ref_uops_valid !== ddl_uops_valid) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH uops_valid @%0d: ref=%b ddl=%b", cycles, ref_uops_valid, ddl_uops_valid);
    end
    // Data only has to agree while it is being offered.
    if (ref_uops_valid && (ref_uops_data !== ddl_uops_data)) begin
      errors++;
      if (errors <= 5)
        $display("MISMATCH uops_data @%0d:\n  ref=%013x\n  ddl=%013x",
                 cycles, ref_uops_data, ddl_uops_data);
    end

    // Rule 2: an offer may not be withdrawn or altered before it is taken.
    if (held_valid && !(ddl_uops_valid && uops_ready)) begin
      if (!ddl_uops_valid) begin
        errors++;
        if (errors <= 5) $display("RULE2 withdrawn @%0d", cycles);
      end else if (ddl_uops_data !== held_data) begin
        errors++;
        if (errors <= 5) $display("RULE2 data changed @%0d", cycles);
      end
    end
  endtask

  task automatic step(logic v, logic [31:0] d, logic r);
    iops_valid = v; iops_data = d; uops_ready = r;
    #1 compare();
    if (ddl_uops_valid && uops_ready) transfers++;
    held_valid = ddl_uops_valid && !uops_ready;
    held_data  = ddl_uops_data;
    @(posedge clk);
    cycles++;
  endtask

  // Rule 3: `valid` must not depend combinationally on `ready`. Toggling
  // `ready` within a cycle must leave `valid` and `data` untouched.
  task automatic check_rule3();
    logic        v0;
    logic [48:0] d0;
    uops_ready = 1'b0; #1;
    v0 = ddl_uops_valid; d0 = ddl_uops_data;
    uops_ready = 1'b1; #1;
    checks++;
    if (ddl_uops_valid !== v0 || ddl_uops_data !== d0) begin
      errors++;
      if (errors <= 5) $display("RULE3 valid/data moved with ready @%0d", cycles);
    end
  endtask

  initial begin
    rst_n = 1'b0;
    iops_valid = 1'b0; iops_data = 32'd0; uops_ready = 1'b0;
    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);

    // Every kind, with the downstream always accepting.
    for (int k = 0; k < 8; k++)
      for (int t = 0; t < 4; t++)
        step(1'b1, {3'($urandom), k[2:0], 5'($urandom), 5'($urandom), 16'($urandom)},
             1'b1);

    // Independent random backpressure on both sides. This is the part that
    // exercises the full slot, the stall and the drain.
    for (int t = 0; t < 40000; t++)
      step($urandom_range(2) != 0, $urandom, $urandom_range(2) != 0);

    // Downstream jammed shut, so the slot fills and stays full.
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

    if (errors == 0)
      $display("TB_PASS  %0d cycles, %0d comparisons, %0d transfers, 0 mismatches",
               cycles, checks, transfers);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
