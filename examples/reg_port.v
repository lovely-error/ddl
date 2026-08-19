// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/reg_port.ddl -o examples/reg_port.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module reg_port (
    input         clk,
    input         rst_n,
    input         cmd_valid,
    output        cmd_ready,
    input  [7:0]  cmd_data,
    input         din_valid,
    output        din_ready,
    input  [31:0] din_data,
    output        dout_valid,
    input         dout_ready,
    output [31:0] dout_data
);

  reg [31:0] cell_;
  reg [1:0] state;

  wire in_s0 = state == 2'd0;
  wire in_s1 = state == 2'd1;
  wire in_s2 = state == 2'd2;
  wire fire_s0 = in_s0 & cmd_valid;
  wire fire_s1 = in_s1 & din_valid;
  wire fire_s2 = in_s2 & dout_ready;
  wire branch_s0 = cmd_data[0];

  assign cmd_ready = in_s0;
  assign din_ready = in_s1;
  assign dout_valid = in_s2;
  assign dout_data = cell_;

  always @(posedge clk) begin
    if (!rst_n) begin
      cell_ <= 32'd0;
      state <= 2'd0;
    end else begin
      cell_ <= (fire_s1 ? din_data : cell_);
      state <= (((fire_s0 | fire_s1) | fire_s2) ? (fire_s0 ? (branch_s0 ? 2'd1 : 2'd2) : (fire_s1 ? 2'd0 : (fire_s2 ? 2'd0 : 2'd0))) : state);
    end
  end

endmodule
