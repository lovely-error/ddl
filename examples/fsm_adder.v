// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/fsm_adder.ddl -o examples/fsm_adder.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module fsm_adder_core (
    input         clk,
    input         rst_n,
    input  [1:0]  src_wsalt,
    output [1:0]  src_rsalt,
    input  [63:0] src_data,
    output [1:0]  dst_wsalt,
    input  [1:0]  dst_rsalt,
    output [63:0] dst_data
);

  reg [1:0] state;
  reg [1:0] src_rsalt_q;
  reg [31:0] dst_e0;
  reg [31:0] dst_e1;
  reg [1:0] dst_wsalt_q;
  reg [31:0] a_r;
  reg [31:0] b_r;

  wire src_empty = src_wsalt == src_rsalt_q;
  wire n7 = !src_empty;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [31:0] src_item = src_ridx ? src_data[63:32] : src_data[31:0];
  wire dst_full = dst_wsalt_q == (~dst_rsalt);
  wire dst_widx = dst_wsalt_q[0] ^ dst_wsalt_q[1];
  wire in_s0 = state == 2'd0;
  wire in_s1 = state == 2'd1;
  wire in_s2 = state == 2'd2;
  wire fire_s0 = in_s0 & n7;
  wire fire_s1 = in_s1 & n7;
  wire fire_s2 = in_s2 & (!dst_full);
  wire [31:0] n39 = a_r + b_r;
  wire src_take = fire_s0 | fire_s1;

  assign src_rsalt = src_rsalt_q;
  assign dst_wsalt = dst_wsalt_q;
  assign dst_data = {dst_e1, dst_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      state <= 2'd0;
      src_rsalt_q <= 2'd0;
      dst_e0 <= 32'd0;
      dst_e1 <= 32'd0;
      dst_wsalt_q <= 2'd0;
      a_r <= 32'd0;
      b_r <= 32'd0;
    end else begin
      state <= (((fire_s0 | fire_s1) | fire_s2) ? (fire_s0 ? 2'd1 : (fire_s1 ? 2'd2 : (fire_s2 ? 2'd0 : 2'd0))) : state);
      src_rsalt_q <= (src_take ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      dst_e0 <= ((fire_s2 & (!dst_widx)) ? n39 : dst_e0);
      dst_e1 <= ((fire_s2 & dst_widx) ? n39 : dst_e1);
      dst_wsalt_q <= (fire_s2 ? (dst_wsalt_q ^ (dst_widx ? 2'd2 : 2'd1)) : dst_wsalt_q);
      a_r <= (fire_s0 ? src_item : a_r);
      b_r <= (fire_s1 ? src_item : b_r);
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

module fsm_adder (
    input         clk,
    input         rst_n,
    output        src_can_receive,
    input         src_receive_en,
    input  [31:0] src_data_write_in,
    output        dst_has_data,
    input         dst_drop_item,
    output [31:0] dst_data_read_out
);

  wire [1:0] src_wsalt;
  wire [1:0] src_rsalt;
  wire [63:0] src_data;
  wire [1:0] dst_wsalt;
  wire [1:0] dst_rsalt;
  wire [63:0] dst_data;

  ddl_wport_to_salt_32 u_src_adapt (
    .clk           (clk),
    .rst_n         (rst_n),
    .o_wsalt       (src_wsalt),
    .o_rsalt       (src_rsalt),
    .o_data        (src_data),
    .can_receive   (src_can_receive),
    .receive_en    (src_receive_en),
    .data_write_in (src_data_write_in)
  );

  ddl_salt_to_rport_32 u_dst_adapt (
    .clk           (clk),
    .rst_n         (rst_n),
    .i_wsalt       (dst_wsalt),
    .i_rsalt       (dst_rsalt),
    .i_data        (dst_data),
    .has_data      (dst_has_data),
    .drop_item     (dst_drop_item),
    .data_read_out (dst_data_read_out)
  );

  fsm_adder_core u_fsm_adder_core (
    .clk       (clk),
    .rst_n     (rst_n),
    .src_wsalt (src_wsalt),
    .src_rsalt (src_rsalt),
    .src_data  (src_data),
    .dst_wsalt (dst_wsalt),
    .dst_rsalt (dst_rsalt),
    .dst_data  (dst_data)
  );

endmodule
