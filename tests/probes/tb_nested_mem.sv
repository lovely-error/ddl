`timescale 1ns/1ps
// Writing part of an element of a memory -- the emitted Verilog, not the
// lowered circuit. See tests/probes/nested_mem.ddl for what each module does.
//
// Every module answers the same thing for every item it takes, so both entries
// of a two-deep output buffer hold the same word and the bench does not have to
// know how many items got through -- only that two did (`o_wsalt === 3`, with
// the reader's salt held at 0 so nothing is ever consumed).
//
// The expected values are written out here rather than read back from a second
// instance of the same RTL: two builds of one wrong idea agree perfectly.
module tb_nested_mem;
  reg clk=0; always #5 clk=~clk;
  reg rst_n=0;

  reg [1:0] ws=0;
  reg [31:0] lane_in=0;
  reg [15:0] two_in=0, field_in=0, then_in=0, else_in=0, seq_in=0;

  wire [1:0] lane_r, lane_w;   wire [63:0] lane_d;
  wire [1:0] two_r, two_w;     wire [63:0] two_d;
  wire [1:0] field_r, field_w; wire [31:0] field_d;
  wire [1:0] then_r, then_w;   wire [63:0] then_d;
  wire [1:0] else_r, else_w;   wire [63:0] else_d;
  wire [1:0] seq_r, seq_w;     wire [63:0] seq_d;

  nm_lane   ul (clk, rst_n, ws, lane_r,  lane_in,  lane_w,  2'b00, lane_d);
  nm_two    ut (clk, rst_n, ws, two_r,   two_in,   two_w,   2'b00, two_d);
  nm_field  uf (clk, rst_n, ws, field_r, field_in, field_w, 2'b00, field_d);
  // Two instances rather than two items, so neither arm depends on which
  // entry the process happens to take first.
  nm_branch ubt(clk, rst_n, ws, then_r,  then_in,  then_w,  2'b00, then_d);
  nm_branch ube(clk, rst_n, ws, else_r,  else_in,  else_w,  2'b00, else_d);
  nm_seq    us (clk, rst_n, ws, seq_r,   seq_in,   seq_w,   2'b00, seq_d);

  initial begin
    repeat(3) @(negedge clk);
    rst_n=1;
    repeat(4) @(negedge clk);

    // Both entries of each input hold the same item.
    lane_in  = 32'h0F250F25;  // address 5, lane 2, byte 8'h3c
    two_in   = 16'h0505;      // address 5
    field_in = 16'h0303;      // address 3
    then_in  = 16'h8585;      // address 5, bit 7 set -- the `then` arm
    else_in  = 16'h0505;      // address 5, bit 7 clear -- the `else` arm
    seq_in   = 16'h0505;      // address 5
    ws = 3;
    repeat(80) @(negedge clk);

    if (lane_w !== 3 || lane_d !== 64'h003c0000_003c0000)
      $fatal(1, "TB_FAIL: computed lane index, got %h", lane_d);
    // The one that fails if the second read does not see the first write.
    if (two_w !== 3 || two_d !== 64'h0000bbaa_0000bbaa)
      $fatal(1, "TB_FAIL: two lanes of one row in one cycle, got %h", two_d);
    if (field_w !== 3 || field_d !== 32'h09040904)
      $fatal(1, "TB_FAIL: struct field of an element, got %h", field_d);
    if (then_w !== 3 || then_d !== 64'h00000011_00000011)
      $fatal(1, "TB_FAIL: nested write in the `then` arm, got %h", then_d);
    if (else_w !== 3 || else_d !== 64'h22000000_22000000)
      $fatal(1, "TB_FAIL: nested write in the `else` arm, got %h", else_d);
    if (seq_w !== 3 || seq_d !== 64'h00000500_00000500)
      $fatal(1, "TB_FAIL: nested write in a pipeline stage, got %h", seq_d);

    $display("TB_PASS: nested memory writes -- computed and constant lanes, struct fields, both arms of a branch, and a pipeline stage");
    $finish;
  end
endmodule
