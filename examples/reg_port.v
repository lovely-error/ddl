// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/reg_port.ddl -o examples/reg_port.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module reg_port_core (
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

module ddl_wport_to_salt_8 (
    input         clk,
    input         rst_n,
    output [1:0]  o_wsalt,
    input  [1:0]  o_rsalt,
    output [15:0] o_data,
    output        can_receive,
    input         receive_en,
    input  [7:0]  data_write_in
);

  reg [7:0] o_e0;
  reg [7:0] o_e1;
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
      o_e0 <= 8'd0;
      o_e1 <= 8'd0;
      o_wsalt_q <= 2'd0;
    end else begin
      o_e0 <= ((push & (!o_widx)) ? data_write_in : o_e0);
      o_e1 <= ((push & o_widx) ? data_write_in : o_e1);
      o_wsalt_q <= (push ? (o_wsalt_q ^ (o_widx ? 2'd2 : 2'd1)) : o_wsalt_q);
    end
  end

endmodule

module ddl_wport_to_salt_32 (
    input         clk,
    input         rst_n,
    output [1:0]  o_wsalt,
    input  [1:0]  o_rsalt,
    output [63:0] o_data,
    output        can_receive,
    input         receive_en,
    input  [31:0] data_write_in
);

  reg [31:0] o_e0;
  reg [31:0] o_e1;
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
      o_e0 <= 32'd0;
      o_e1 <= 32'd0;
      o_wsalt_q <= 2'd0;
    end else begin
      o_e0 <= ((push & (!o_widx)) ? data_write_in : o_e0);
      o_e1 <= ((push & o_widx) ? data_write_in : o_e1);
      o_wsalt_q <= (push ? (o_wsalt_q ^ (o_widx ? 2'd2 : 2'd1)) : o_wsalt_q);
    end
  end

endmodule

module ddl_salt_to_rport_32 (
    input         clk,
    input         rst_n,
    input  [1:0]  i_wsalt,
    output [1:0]  i_rsalt,
    input  [63:0] i_data,
    output        has_data,
    input         drop_item,
    output [31:0] data_read_out
);

  reg [1:0] rsalt_q;

  wire i_empty = i_wsalt == rsalt_q;
  wire has_item = !i_empty;
  wire i_ridx = rsalt_q[0] ^ rsalt_q[1];
  wire [31:0] i_item = i_ridx ? i_data[63:32] : i_data[31:0];
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

module reg_port (
    input         clk,
    input         rst_n,
    output        cmd_can_receive,
    input         cmd_receive_en,
    input  [7:0]  cmd_data_write_in,
    output        din_can_receive,
    input         din_receive_en,
    input  [31:0] din_data_write_in,
    output        dout_has_data,
    input         dout_drop_item,
    output [31:0] dout_data_read_out
);

  wire [1:0] cmd_wsalt;
  wire [1:0] cmd_rsalt;
  wire [15:0] cmd_data;
  wire [1:0] din_wsalt;
  wire [1:0] din_rsalt;
  wire [63:0] din_data;
  wire [1:0] dout_wsalt;
  wire [1:0] dout_rsalt;
  wire [63:0] dout_data;

  ddl_wport_to_salt_8 u_cmd_adapt (
    .clk           (clk),
    .rst_n         (rst_n),
    .o_wsalt       (cmd_wsalt),
    .o_rsalt       (cmd_rsalt),
    .o_data        (cmd_data),
    .can_receive   (cmd_can_receive),
    .receive_en    (cmd_receive_en),
    .data_write_in (cmd_data_write_in)
  );

  ddl_wport_to_salt_32 u_din_adapt (
    .clk           (clk),
    .rst_n         (rst_n),
    .o_wsalt       (din_wsalt),
    .o_rsalt       (din_rsalt),
    .o_data        (din_data),
    .can_receive   (din_can_receive),
    .receive_en    (din_receive_en),
    .data_write_in (din_data_write_in)
  );

  ddl_salt_to_rport_32 u_dout_adapt (
    .clk           (clk),
    .rst_n         (rst_n),
    .i_wsalt       (dout_wsalt),
    .i_rsalt       (dout_rsalt),
    .i_data        (dout_data),
    .has_data      (dout_has_data),
    .drop_item     (dout_drop_item),
    .data_read_out (dout_data_read_out)
  );

  reg_port_core u_reg_port_core (
    .clk        (clk),
    .rst_n      (rst_n),
    .cmd_wsalt  (cmd_wsalt),
    .cmd_rsalt  (cmd_rsalt),
    .cmd_data   (cmd_data),
    .din_wsalt  (din_wsalt),
    .din_rsalt  (din_rsalt),
    .din_data   (din_data),
    .dout_wsalt (dout_wsalt),
    .dout_rsalt (dout_rsalt),
    .dout_data  (dout_data)
  );

endmodule
