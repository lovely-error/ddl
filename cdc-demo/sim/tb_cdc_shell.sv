// The generated shells, driven the way the compiler wires them.
//
// The shells are twenty lines each, but they are twenty lines of DIRECTION, and
// a swapped master/slave face or a `wpush` taken from the wrong side is exactly
// the kind of error that lints clean and then loses every item. So each of the
// four is driven here the way its boundary drives it, and checked for the same
// contract everything else in this directory is checked for: nothing moves
// without room, items arrive in order, once each.
//
//   In          an export target's `buffer in`  -- outside writes, DDL consumes
//   Out         an export target's `buffer out` -- DDL produces, outside reads
//   ToExtern    DDL sends to an extern
//   FromExtern  an extern produces, DDL consumes
//
// `In`/`FromExtern` carry data INTO the core, so their FIFO is written on the
// foreign clock; `Out`/`ToExtern` the other way. Both orientations are here.

`timescale 1ns / 1ps

module tb_cdc_shell;

  parameter real PERIOD_C = 12.346;   // 81 MHz core
  parameter real PERIOD_F = 9.259;    // 108 MHz foreign

  reg clk = 1'b0, f_clk = 1'b0, rst_n = 1'b0;
  always #(PERIOD_C / 2.0) clk   = ~clk;
  always #(PERIOD_F / 2.0) f_clk = ~f_clk;

  integer fails = 0;

  // ---------------------------------------------------------------- In ----
  // Foreign side writes; the core side behaves as the adapter, always able to
  // receive, and checks what arrives.
  reg  [15:0] in_src;
  wire        in_f_can, in_c_en;
  wire [15:0] in_c_data;
  wire        in_f_en = in_f_can;          // write whenever there is room

  always @(posedge f_clk) begin
    if (!rst_n)       in_src <= 16'd0;
    else if (in_f_en) in_src <= in_src + 16'd1;
  end

  ddl_cdc_in_16x8 u_in (
      .clk(clk), .rst_n(rst_n), .f_clk(f_clk),
      .f_can_receive(in_f_can), .f_receive_en(in_f_en), .f_data_write_in(in_src),
      .c_can_receive(1'b1), .c_receive_en(in_c_en), .c_data_write_in(in_c_data)
  );

  integer in_rx = 0, in_err = 0;
  reg [15:0] in_exp = 16'd0;
  reg        in_armed = 1'b0;
  // `armed` because the source starts counting when the TESTBENCH's rst_n
  // releases, while the shell's own handshake releases a few cycles later --
  // so the first items are generated into a FIFO that is still held. What is
  // being checked is "in order, none lost", not "starts at zero".
  always @(posedge clk) if (rst_n && in_c_en) begin
    in_rx <= in_rx + 1;
    if (in_armed && in_c_data !== in_exp) in_err <= in_err + 1;
    in_exp   <= in_c_data + 16'd1;
    in_armed <= 1'b1;
  end

  // --------------------------------------------------------------- Out ----
  // The core side behaves as the adapter, always having data; the foreign side
  // reads and checks.
  reg  [15:0] out_src;
  wire        out_c_drop, out_f_has;
  wire [15:0] out_f_data;
  wire        out_f_drop = out_f_has;

  always @(posedge clk) begin
    if (!rst_n)          out_src <= 16'd0;
    else if (out_c_drop) out_src <= out_src + 16'd1;
  end

  ddl_cdc_out_16x8 u_out (
      .clk(clk), .rst_n(rst_n), .f_clk(f_clk),
      .c_has_data(1'b1), .c_drop_item(out_c_drop), .c_data_read_out(out_src),
      .f_has_data(out_f_has), .f_drop_item(out_f_drop), .f_data_read_out(out_f_data)
  );

  integer out_rx = 0, out_err = 0;
  reg [15:0] out_exp = 16'd0;
  reg        out_armed = 1'b0;
  // `armed` because the source starts counting when the TESTBENCH's rst_n
  // releases, while the shell's own handshake releases a few cycles later --
  // so the first items are generated into a FIFO that is still held. What is
  // being checked is "in order, none lost", not "starts at zero".
  always @(posedge f_clk) if (rst_n && out_f_drop) begin
    out_rx <= out_rx + 1;
    if (out_armed && out_f_data !== out_exp) out_err <= out_err + 1;
    out_exp   <= out_f_data + 16'd1;
    out_armed <= 1'b1;
  end

  // ---------------------------------------------------------- ToExtern ----
  // The adapter drives the core side as a master; the extern answers on the
  // foreign side as a slave that is always ready.
  reg  [15:0] te_src;
  wire        te_c_can, te_f_en;
  wire [15:0] te_f_data;
  wire        te_c_en = te_c_can;

  always @(posedge clk) begin
    if (!rst_n)       te_src <= 16'd0;
    else if (te_c_en) te_src <= te_src + 16'd1;
  end

  ddl_cdc_to_ext_16x8 u_te (
      .clk(clk), .rst_n(rst_n), .f_clk(f_clk),
      .c_can_receive(te_c_can), .c_receive_en(te_c_en), .c_data_write_in(te_src),
      .f_can_receive(1'b1), .f_receive_en(te_f_en), .f_data_write_in(te_f_data)
  );

  integer te_rx = 0, te_err = 0;
  reg [15:0] te_exp = 16'd0;
  reg        te_armed = 1'b0;
  // `armed` because the source starts counting when the TESTBENCH's rst_n
  // releases, while the shell's own handshake releases a few cycles later --
  // so the first items are generated into a FIFO that is still held. What is
  // being checked is "in order, none lost", not "starts at zero".
  always @(posedge f_clk) if (rst_n && te_f_en) begin
    te_rx <= te_rx + 1;
    if (te_armed && te_f_data !== te_exp) te_err <= te_err + 1;
    te_exp   <= te_f_data + 16'd1;
    te_armed <= 1'b1;
  end

  // -------------------------------------------------------- FromExtern ----
  // The extern produces on the foreign side; the adapter pulls on the core side.
  reg  [15:0] fe_src;
  wire        fe_f_drop, fe_c_has;
  wire [15:0] fe_c_data;
  wire        fe_c_drop = fe_c_has;

  always @(posedge f_clk) begin
    if (!rst_n)         fe_src <= 16'd0;
    else if (fe_f_drop) fe_src <= fe_src + 16'd1;
  end

  ddl_cdc_from_ext_16x8 u_fe (
      .clk(clk), .rst_n(rst_n), .f_clk(f_clk),
      .f_has_data(1'b1), .f_drop_item(fe_f_drop), .f_data_read_out(fe_src),
      .c_has_data(fe_c_has), .c_drop_item(fe_c_drop), .c_data_read_out(fe_c_data)
  );

  integer fe_rx = 0, fe_err = 0;
  reg [15:0] fe_exp = 16'd0;
  reg        fe_armed = 1'b0;
  // `armed` because the source starts counting when the TESTBENCH's rst_n
  // releases, while the shell's own handshake releases a few cycles later --
  // so the first items are generated into a FIFO that is still held. What is
  // being checked is "in order, none lost", not "starts at zero".
  always @(posedge clk) if (rst_n && fe_c_drop) begin
    fe_rx <= fe_rx + 1;
    if (fe_armed && fe_c_data !== fe_exp) fe_err <= fe_err + 1;
    fe_exp   <= fe_c_data + 16'd1;
    fe_armed <= 1'b1;
  end

  // ---------------------------------------------------------------- run ---
  task check(input [8*12-1:0] name, input integer rx, input integer err);
    begin
      $display("    %-11s %8d items  %6d errors", name, rx, err);
      if (rx < 1000) begin fails = fails + 1; $display("      FAIL: barely moved"); end
      if (err != 0)  begin fails = fails + 1; $display("      FAIL: out of order or corrupt"); end
    end
  endtask

  initial begin
    rst_n = 1'b0;
    repeat (50) @(posedge clk);
    rst_n = 1'b1;
    repeat (20000) @(posedge clk);

    $display("");
    $display("  generated shells, core %0.1f MHz -> foreign %0.1f MHz",
             1000.0 / PERIOD_C, 1000.0 / PERIOD_F);
    check("In",         in_rx,  in_err);
    check("Out",        out_rx, out_err);
    check("ToExtern",   te_rx,  te_err);
    check("FromExtern", fe_rx,  fe_err);
    $display("");
    if (fails == 0) $display("  OK: all four shells move items in order, none lost.");
    else            $display("  FAILED: %0d checks", fails);
    $display("");
    $finish;
  end

endmodule
