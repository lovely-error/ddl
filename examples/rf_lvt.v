// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/rf_lvt.ddl -o examples/rf_lvt.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module rf_lvt (
    input          clk,
    input          rst_n,
    input  [1:0]   req_wsalt,
    output [1:0]   req_rsalt,
    input  [191:0] req_data,
    output [1:0]   resp_wsalt,
    input  [1:0]   resp_rsalt,
    output [255:0] resp_data
);

  reg v0;
  reg v1;
  reg v2;
  reg [1:0] src_rsalt_q;
  reg [127:0] out_e0;
  reg [127:0] out_e1;
  reg [1:0] out_wsalt_q;
  reg vals_fwd0_s1;
  reg [31:0] vals_wdata0_s1;
  reg vals_fwd1_s1;
  reg [31:0] vals_wdata1_s1;
  reg [31:0] sum_s2;
  reg [31:0] x_s2;
  reg [31:0] y_s2;
  reg [31:0] dif_s3;
  reg [31:0] sum_s3;
  reg [31:0] x_s3;
  reg [31:0] y_s3;

  reg [31:0] vals [0:255];
  reg [31:0] vals_q0;
  reg [31:0] vals_q1;

  wire resp_full = out_wsalt_q == (~resp_rsalt);
  wire shift = !resp_full;
  wire req_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [95:0] req_item = req_ridx ? req_data[191:96] : req_data[95:0];
  wire [7:0] n18 = req_item[95:88];
  wire [31:0] n19 = req_item[87:56];
  wire [7:0] n21 = req_item[55:48];
  wire [31:0] n22 = req_item[47:16];
  wire [7:0] n24 = req_item[15:8];
  wire n26 = n24 == n21;
  wire [7:0] n29 = req_item[7:0];
  wire n31 = n29 == n21;
  wire req_empty = req_wsalt == src_rsalt_q;
  wire n36 = !req_empty;
  wire n37 = n36 & shift;
  wire [31:0] n43 = vals_fwd0_s1 ? vals_wdata0_s1 : vals_q0;
  wire [31:0] n49 = vals_fwd1_s1 ? vals_wdata1_s1 : vals_q1;
  wire [31:0] sum = n43 + n49;
  wire [31:0] dif = x_s2 - y_s2;
  wire [127:0] n66 = {x_s3, y_s3, sum_s3, dif_s3};
  wire vals_re0 = n36 & shift;
  wire vals_re1 = n36 & shift;
  wire out_push = shift & v2;
  wire out_widx = out_wsalt_q[0] ^ out_wsalt_q[1];
  wire src_take = n36 & shift;

  assign req_rsalt = src_rsalt_q;
  assign resp_wsalt = out_wsalt_q;
  assign resp_data = {out_e1, out_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      v2 <= 1'b0;
      src_rsalt_q <= 2'd0;
      out_e0 <= 128'd0;
      out_e1 <= 128'd0;
      out_wsalt_q <= 2'd0;
      vals_fwd0_s1 <= 1'b0;
      vals_wdata0_s1 <= 32'd0;
      vals_fwd1_s1 <= 1'b0;
      vals_wdata1_s1 <= 32'd0;
      sum_s2 <= 32'd0;
      x_s2 <= 32'd0;
      y_s2 <= 32'd0;
      dif_s3 <= 32'd0;
      sum_s3 <= 32'd0;
      x_s3 <= 32'd0;
      y_s3 <= 32'd0;
    end else begin
      v0 <= (shift ? n36 : v0);
      v1 <= (shift ? v0 : v1);
      v2 <= (shift ? v1 : v2);
      src_rsalt_q <= (src_take ? (src_rsalt_q ^ (req_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      out_e0 <= ((out_push & (!out_widx)) ? n66 : out_e0);
      out_e1 <= ((out_push & out_widx) ? n66 : out_e1);
      out_wsalt_q <= (out_push ? (out_wsalt_q ^ (out_widx ? 2'd2 : 2'd1)) : out_wsalt_q);
      vals_fwd0_s1 <= (shift ? ((n24 == n18) | n26) : vals_fwd0_s1);
      vals_wdata0_s1 <= (shift ? (n26 ? n22 : n19) : vals_wdata0_s1);
      vals_fwd1_s1 <= (shift ? ((n29 == n18) | n31) : vals_fwd1_s1);
      vals_wdata1_s1 <= (shift ? (n31 ? n22 : n19) : vals_wdata1_s1);
      sum_s2 <= (shift ? sum : sum_s2);
      x_s2 <= (shift ? n43 : x_s2);
      y_s2 <= (shift ? n49 : y_s2);
      dif_s3 <= (shift ? dif : dif_s3);
      sum_s3 <= (shift ? sum_s2 : sum_s3);
      x_s3 <= (shift ? x_s2 : x_s3);
      y_s3 <= (shift ? y_s2 : y_s3);
    end
  end

  // vals [0:255] -- block RAM: 2 sync write ports, sync reads
  always @(posedge clk) begin
    if (n37) vals[n18] <= n19;
    if (n37) vals[n21] <= n22;
    if (vals_re0) vals_q0 <= vals[n24];
    if (vals_re1) vals_q1 <= vals[n29];
  end

endmodule
