// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/reg_port.ddl -o examples/reg_port.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module reg_port (
    input         clk,
    input         rst_n,
    input  [1:0]  cmd_wsalt,
    output [1:0]  cmd_rsalt,
    input  [15:0] cmd_data,
    input  [1:0]  din_wsalt,
    output [1:0]  din_rsalt,
    input  [63:0] din_data,
    output [1:0]  dout_wsalt,
    input  [1:0]  dout_rsalt,
    output [63:0] dout_data
);

  reg [31:0] cell_;
  reg [1:0] state;
  reg [1:0] cmd_rsalt_q;
  reg [1:0] din_rsalt_q;
  reg [31:0] dout_e0;
  reg [31:0] dout_e1;
  reg [1:0] dout_wsalt_q;

  wire cmd_empty = cmd_wsalt == cmd_rsalt_q;
  wire cmd_ridx = cmd_rsalt_q[0] ^ cmd_rsalt_q[1];
  wire [7:0] cmd_item = cmd_ridx ? cmd_data[15:8] : cmd_data[7:0];
  wire din_empty = din_wsalt == din_rsalt_q;
  wire din_ridx = din_rsalt_q[0] ^ din_rsalt_q[1];
  wire [31:0] din_item = din_ridx ? din_data[63:32] : din_data[31:0];
  wire dout_full = dout_wsalt_q == (~dout_rsalt);
  wire dout_widx = dout_wsalt_q[0] ^ dout_wsalt_q[1];
  wire in_s0 = state == 2'd0;
  wire in_s1 = state == 2'd1;
  wire in_s2 = state == 2'd2;
  wire fire_s0 = in_s0 & (!cmd_empty);
  wire fire_s1 = in_s1 & (!din_empty);
  wire fire_s2 = in_s2 & (!dout_full);
  wire branch_s0 = cmd_item[0];

  assign cmd_rsalt = cmd_rsalt_q;
  assign din_rsalt = din_rsalt_q;
  assign dout_wsalt = dout_wsalt_q;
  assign dout_data = {dout_e1, dout_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      cell_ <= 32'd0;
      state <= 2'd0;
      cmd_rsalt_q <= 2'd0;
      din_rsalt_q <= 2'd0;
      dout_e0 <= 32'd0;
      dout_e1 <= 32'd0;
      dout_wsalt_q <= 2'd0;
    end else begin
      cell_ <= (fire_s1 ? din_item : cell_);
      state <= (((fire_s0 | fire_s1) | fire_s2) ? (fire_s0 ? (branch_s0 ? 2'd1 : 2'd2) : (fire_s1 ? 2'd0 : (fire_s2 ? 2'd0 : 2'd0))) : state);
      cmd_rsalt_q <= (fire_s0 ? (cmd_rsalt_q ^ (cmd_ridx ? 2'd2 : 2'd1)) : cmd_rsalt_q);
      din_rsalt_q <= (fire_s1 ? (din_rsalt_q ^ (din_ridx ? 2'd2 : 2'd1)) : din_rsalt_q);
      dout_e0 <= ((fire_s2 & (!dout_widx)) ? cell_ : dout_e0);
      dout_e1 <= ((fire_s2 & dout_widx) ? cell_ : dout_e1);
      dout_wsalt_q <= (fire_s2 ? (dout_wsalt_q ^ (dout_widx ? 2'd2 : 2'd1)) : dout_wsalt_q);
    end
  end

endmodule
