// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/rf_lvt.ddl -o examples/rf_lvt.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module rf_lvt_core (
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
  reg [1:0] req_rsalt_q;
  reg [127:0] resp_e0;
  reg [127:0] resp_e1;
  reg [1:0] resp_wsalt_q;
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

  wire resp_full = resp_wsalt_q == (~resp_rsalt);
  wire shift3 = !resp_full;
  wire req_ridx = req_rsalt_q[0] ^ req_rsalt_q[1];
  wire [95:0] req_item = req_ridx ? req_data[191:96] : req_data[95:0];
  wire [7:0] n18 = req_item[95:88];
  wire [31:0] n19 = req_item[87:56];
  wire [7:0] n21 = req_item[55:48];
  wire [31:0] n22 = req_item[47:16];
  wire [7:0] n24 = req_item[15:8];
  wire n26 = n24 == n21;
  wire [7:0] n29 = req_item[7:0];
  wire n31 = n29 == n21;
  wire [31:0] n37 = vals_fwd0_s1 ? vals_wdata0_s1 : vals_q0;
  wire [31:0] n41 = vals_fwd1_s1 ? vals_wdata1_s1 : vals_q1;
  wire [31:0] sum = n37 + n41;
  wire [31:0] dif = x_s2 - y_s2;
  wire [127:0] n51 = {x_s3, y_s3, sum_s3, dif_s3};
  wire shift2 = (!v2) | shift3;
  wire shift1 = (!v1) | shift2;
  wire shift0 = (!v0) | shift1;
  wire req_empty = req_wsalt == req_rsalt_q;
  wire req_present = !req_empty;
  wire n64 = req_present & shift0;
  wire vals_re0 = req_present & shift0;
  wire vals_re1 = req_present & shift0;
  wire push = shift3 & v2;
  wire resp_widx = resp_wsalt_q[0] ^ resp_wsalt_q[1];
  wire take = req_present & shift0;

  assign resp_data = {resp_e1, resp_e0};
  assign resp_wsalt = resp_wsalt_q;
  assign req_rsalt = req_rsalt_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      v2 <= 1'b0;
      req_rsalt_q <= 2'd0;
      resp_e0 <= 128'd0;
      resp_e1 <= 128'd0;
      resp_wsalt_q <= 2'd0;
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
      v0 <= (shift0 ? req_present : v0);
      v1 <= (shift1 ? v0 : v1);
      v2 <= (shift2 ? v1 : v2);
      req_rsalt_q <= (take ? (req_rsalt_q ^ (req_ridx ? 2'd2 : 2'd1)) : req_rsalt_q);
      resp_e0 <= ((push & (!resp_widx)) ? n51 : resp_e0);
      resp_e1 <= ((push & resp_widx) ? n51 : resp_e1);
      resp_wsalt_q <= (push ? (resp_wsalt_q ^ (resp_widx ? 2'd2 : 2'd1)) : resp_wsalt_q);
      vals_fwd0_s1 <= (shift0 ? ((n24 == n18) | n26) : vals_fwd0_s1);
      vals_wdata0_s1 <= (shift0 ? (n26 ? n22 : n19) : vals_wdata0_s1);
      vals_fwd1_s1 <= (shift0 ? ((n29 == n18) | n31) : vals_fwd1_s1);
      vals_wdata1_s1 <= (shift0 ? (n31 ? n22 : n19) : vals_wdata1_s1);
      sum_s2 <= (shift1 ? sum : sum_s2);
      x_s2 <= (shift1 ? n37 : x_s2);
      y_s2 <= (shift1 ? n41 : y_s2);
      dif_s3 <= (shift2 ? dif : dif_s3);
      sum_s3 <= (shift2 ? sum_s2 : sum_s3);
      x_s3 <= (shift2 ? x_s2 : x_s3);
      y_s3 <= (shift2 ? y_s2 : y_s3);
    end
  end

  // vals [0:255] -- block RAM: 2 sync write ports, sync reads
  always @(posedge clk) begin
    if (n64) vals[n18] <= n19;
    if (n64) vals[n21] <= n22;
    if (vals_re0) vals_q0 <= vals[n24];
    if (vals_re1) vals_q1 <= vals[n29];
  end

endmodule

module ddl_wport_to_salt_96 (
    input          clk,
    input          rst_n,
    output [1:0]   o_wsalt,
    input  [1:0]   o_rsalt,
    output [191:0] o_data,
    output         can_receive,
    input          receive_en,
    input  [95:0]  data_write_in
);

  reg [95:0] o_e0;
  reg [95:0] o_e1;
  reg [1:0] o_wsalt_q;

  wire o_widx = o_wsalt_q[0] ^ o_wsalt_q[1];
  wire o_full = o_wsalt_q == (~o_rsalt);
  wire room = !o_full;
  wire push = receive_en & room;

  assign can_receive = room;
  assign o_data = {o_e1, o_e0};
  assign o_wsalt = o_wsalt_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      o_e0 <= 96'd0;
      o_e1 <= 96'd0;
      o_wsalt_q <= 2'd0;
    end else begin
      o_e0 <= ((push & (!o_widx)) ? data_write_in : o_e0);
      o_e1 <= ((push & o_widx) ? data_write_in : o_e1);
      o_wsalt_q <= (push ? (o_wsalt_q ^ (o_widx ? 2'd2 : 2'd1)) : o_wsalt_q);
    end
  end

endmodule

module ddl_salt_to_rport_128 (
    input          clk,
    input          rst_n,
    input  [1:0]   i_wsalt,
    output [1:0]   i_rsalt,
    input  [255:0] i_data,
    output         has_data,
    input          drop_item,
    output [127:0] data_read_out
);

  reg [1:0] rsalt_q;

  wire i_empty = i_wsalt == rsalt_q;
  wire has_item = !i_empty;
  wire i_ridx = rsalt_q[0] ^ rsalt_q[1];
  wire [127:0] i_item = i_ridx ? i_data[255:128] : i_data[127:0];
  wire take = has_item & drop_item;

  assign data_read_out = i_item;
  assign has_data = has_item;
  assign i_rsalt = rsalt_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      rsalt_q <= 2'd0;
    end else begin
      rsalt_q <= (take ? (rsalt_q ^ (i_ridx ? 2'd2 : 2'd1)) : rsalt_q);
    end
  end

endmodule

module rf_lvt (
    input          clk,
    input          rst_n,
    output         req_can_receive,
    input          req_receive_en,
    input  [95:0]  req_data_write_in,
    output         resp_has_data,
    input          resp_drop_item,
    output [127:0] resp_data_read_out
);

  wire [1:0] req_wsalt;
  wire [1:0] req_rsalt;
  wire [191:0] req_data;
  wire [1:0] resp_wsalt;
  wire [1:0] resp_rsalt;
  wire [255:0] resp_data;

  ddl_wport_to_salt_96 u_req_adapt (
    .clk           (clk),
    .rst_n         (rst_n),
    .o_wsalt       (req_wsalt),
    .o_rsalt       (req_rsalt),
    .o_data        (req_data),
    .can_receive   (req_can_receive),
    .receive_en    (req_receive_en),
    .data_write_in (req_data_write_in)
  );

  ddl_salt_to_rport_128 u_resp_adapt (
    .clk           (clk),
    .rst_n         (rst_n),
    .i_wsalt       (resp_wsalt),
    .i_rsalt       (resp_rsalt),
    .i_data        (resp_data),
    .has_data      (resp_has_data),
    .drop_item     (resp_drop_item),
    .data_read_out (resp_data_read_out)
  );

  rf_lvt_core u_rf_lvt_core (
    .clk        (clk),
    .rst_n      (rst_n),
    .req_wsalt  (req_wsalt),
    .req_rsalt  (req_rsalt),
    .req_data   (req_data),
    .resp_wsalt (resp_wsalt),
    .resp_rsalt (resp_rsalt),
    .resp_data  (resp_data)
  );

endmodule
