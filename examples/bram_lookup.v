// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/bram_lookup.ddl -o examples/bram_lookup.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module bram_lookup (
    input         clk,
    input         rst_n,
    input         cmd_valid,
    output        cmd_ready,
    input  [15:0] cmd_data,
    input         din_valid,
    output        din_ready,
    input  [31:0] din_data,
    output        resp_valid,
    input         resp_ready,
    output [31:0] resp_data
);

  reg [1:0] state;
  reg [7:0] addr_r;

  reg [31:0] t [0:255];
  reg [31:0] t_q;

  wire in_s0 = state == 2'd0;
  wire in_s1 = state == 2'd1;
  wire in_s2 = state == 2'd2;
  wire in_s3 = state == 2'd3;
  wire fire_s0 = in_s0 & cmd_valid;
  wire fire_s1 = in_s1 & din_valid;
  wire fire_s3 = in_s3 & resp_ready;
  wire [7:0] addr = cmd_data[7:0];
  wire branch_s0 = cmd_data[15];

  assign cmd_ready = in_s0;
  assign din_ready = in_s1;
  assign resp_valid = in_s3;
  assign resp_data = t_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      state <= 2'd0;
      addr_r <= 8'd0;
    end else begin
      state <= ((((fire_s0 | fire_s1) | in_s2) | fire_s3) ? (fire_s0 ? (branch_s0 ? 2'd1 : 2'd2) : (fire_s1 ? 2'd0 : (in_s2 ? 2'd3 : (fire_s3 ? 2'd0 : 2'd0)))) : state);
      addr_r <= (fire_s0 ? addr : addr_r);
    end
  end

  // t [0:255] -- block RAM: one sync write port, sync reads
  always @(posedge clk) begin
    if (fire_s1) begin
      t[addr_r] <= din_data;
    end
    if (in_s2) t_q <= t[addr_r];
  end

endmodule
