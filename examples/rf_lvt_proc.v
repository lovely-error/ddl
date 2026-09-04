// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/rf_lvt_proc.ddl -o examples/rf_lvt_proc.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module rf_lvt_proc (
    input          clk,
    input          rst_n,
    input  [1:0]   cmd_wsalt,
    output [1:0]   cmd_rsalt,
    input  [191:0] cmd_data,
    output [1:0]   rd_wsalt,
    input  [1:0]   rd_rsalt,
    output [127:0] rd_data
);

  reg [1:0] state;
  reg [1:0] cmd_rsalt_q;
  reg [63:0] rd_e0;
  reg [63:0] rd_e1;
  reg [1:0] rd_wsalt_q;
  reg rd_valid_s1;
  reg rd_valid_s2;
  reg [95:0] q_r;
  reg [31:0] x_r;
  reg [31:0] y_r;

  reg [31:0] vals [0:255];
  reg [31:0] vals_q;

  wire cmd_empty = cmd_wsalt == cmd_rsalt_q;
  wire cmd_ridx = cmd_rsalt_q[0] ^ cmd_rsalt_q[1];
  wire [95:0] cmd_item = cmd_ridx ? cmd_data[191:96] : cmd_data[95:0];
  wire rd_full = rd_wsalt_q == (~rd_rsalt);
  wire rd_widx = rd_wsalt_q[0] ^ rd_wsalt_q[1];
  wire in_s0 = state == 2'd0;
  wire in_s1 = state == 2'd1;
  wire in_s2 = state == 2'd2;
  wire in_s3 = state == 2'd3;
  wire fire_s0 = in_s0 & (!cmd_empty);
  wire fire_s3 = in_s3 & (!rd_full);
  wire [31:0] n47 = vals_q;
  wire [31:0] x_live = rd_valid_s1 ? n47 : x_r;
  wire [31:0] n54 = vals_q;
  wire [31:0] y_live = rd_valid_s2 ? n54 : y_r;
  wire [63:0] n60 = {x_live, y_live};
  wire n104 = in_s1 | in_s2;
  wire [7:0] n105 = in_s2 ? q_r[7:0] : q_r[15:8];

  assign cmd_rsalt = cmd_rsalt_q;
  assign rd_wsalt = rd_wsalt_q;
  assign rd_data = {rd_e1, rd_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      state <= 2'd0;
      cmd_rsalt_q <= 2'd0;
      rd_e0 <= 64'd0;
      rd_e1 <= 64'd0;
      rd_wsalt_q <= 2'd0;
      rd_valid_s1 <= 1'b0;
      rd_valid_s2 <= 1'b0;
      q_r <= 96'd0;
      x_r <= 32'd0;
      y_r <= 32'd0;
    end else begin
      state <= ((((fire_s0 | in_s1) | in_s2) | fire_s3) ? (fire_s0 ? 2'd1 : (in_s1 ? 2'd2 : (in_s2 ? 2'd3 : (fire_s3 ? 2'd0 : 2'd0)))) : state);
      cmd_rsalt_q <= (fire_s0 ? (cmd_rsalt_q ^ (cmd_ridx ? 2'd2 : 2'd1)) : cmd_rsalt_q);
      rd_e0 <= ((fire_s3 & (!rd_widx)) ? n60 : rd_e0);
      rd_e1 <= ((fire_s3 & rd_widx) ? n60 : rd_e1);
      rd_wsalt_q <= (fire_s3 ? (rd_wsalt_q ^ (rd_widx ? 2'd2 : 2'd1)) : rd_wsalt_q);
      rd_valid_s1 <= in_s1;
      rd_valid_s2 <= in_s2;
      q_r <= (fire_s0 ? cmd_item : q_r);
      x_r <= (rd_valid_s1 ? n47 : x_r);
      y_r <= (rd_valid_s2 ? n54 : y_r);
    end
  end

  // vals [0:255] -- block RAM: 2 sync write ports, sync reads
  always @(posedge clk) begin
    if (fire_s0) vals[cmd_item[95:88]] <= cmd_item[87:56];
    if (fire_s0) vals[cmd_item[55:48]] <= cmd_item[47:16];
    if (n104) vals_q <= vals[n105];
  end

endmodule
