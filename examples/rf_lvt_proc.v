// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/rf_lvt_proc.ddl -o examples/rf_lvt_proc.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module rf_lvt_proc_core (
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

module ddl_salt_to_rport_64 (
    input          clk,
    input          rst_n,
    input  [1:0]   i_wsalt,
    output [1:0]   i_rsalt,
    input  [127:0] i_data,
    output         has_data,
    input          drop_item,
    output [63:0]  data_read_out
);

  reg [1:0] rsalt_q;

  wire i_empty = i_wsalt == rsalt_q;
  wire has_item = !i_empty;
  wire i_ridx = rsalt_q[0] ^ rsalt_q[1];
  wire [63:0] i_item = i_ridx ? i_data[127:64] : i_data[63:0];
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

module rf_lvt_proc (
    input         clk,
    input         rst_n,
    output        cmd_can_receive,
    input         cmd_receive_en,
    input  [95:0] cmd_data_write_in,
    output        rd_has_data,
    input         rd_drop_item,
    output [63:0] rd_data_read_out
);

  wire [1:0] cmd_wsalt;
  wire [1:0] cmd_rsalt;
  wire [191:0] cmd_data;
  wire [1:0] rd_wsalt;
  wire [1:0] rd_rsalt;
  wire [127:0] rd_data;

  ddl_wport_to_salt_96 u_cmd_adapt (
    .clk           (clk),
    .rst_n         (rst_n),
    .o_wsalt       (cmd_wsalt),
    .o_rsalt       (cmd_rsalt),
    .o_data        (cmd_data),
    .can_receive   (cmd_can_receive),
    .receive_en    (cmd_receive_en),
    .data_write_in (cmd_data_write_in)
  );

  ddl_salt_to_rport_64 u_rd_adapt (
    .clk           (clk),
    .rst_n         (rst_n),
    .i_wsalt       (rd_wsalt),
    .i_rsalt       (rd_rsalt),
    .i_data        (rd_data),
    .has_data      (rd_has_data),
    .drop_item     (rd_drop_item),
    .data_read_out (rd_data_read_out)
  );

  rf_lvt_proc_core u_rf_lvt_proc_core (
    .clk       (clk),
    .rst_n     (rst_n),
    .cmd_wsalt (cmd_wsalt),
    .cmd_rsalt (cmd_rsalt),
    .cmd_data  (cmd_data),
    .rd_wsalt  (rd_wsalt),
    .rd_rsalt  (rd_rsalt),
    .rd_data   (rd_data)
  );

endmodule
