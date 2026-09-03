// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/fanout.ddl -o examples/fanout.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module scale (
    input         clk,
    input         rst_n,
    input  [1:0]  src_wsalt,
    output [1:0]  src_rsalt,
    input  [63:0] src_data,
    output [1:0]  dst_wsalt,
    input  [1:0]  dst_rsalt,
    output [63:0] dst_data
);

  reg v0;
  reg [1:0] src_rsalt_q;
  reg [31:0] out_e0;
  reg [31:0] out_e1;
  reg [1:0] out_wsalt_q;
  reg [31:0] x_s1;

  wire dst_full = out_wsalt_q == (~dst_rsalt);
  wire shift = !dst_full;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [31:0] src_item = src_ridx ? src_data[63:32] : src_data[31:0];
  wire [31:0] doubled = x_s1 + x_s1;
  wire src_empty = src_wsalt == src_rsalt_q;
  wire n20 = !src_empty;
  wire out_push = shift & v0;
  wire out_widx = out_wsalt_q[0] ^ out_wsalt_q[1];
  wire src_take = n20 & shift;

  assign src_rsalt = src_rsalt_q;
  assign dst_wsalt = out_wsalt_q;
  assign dst_data = {out_e1, out_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      src_rsalt_q <= 2'd0;
      out_e0 <= 32'd0;
      out_e1 <= 32'd0;
      out_wsalt_q <= 2'd0;
      x_s1 <= 32'd0;
    end else begin
      v0 <= (shift ? n20 : v0);
      src_rsalt_q <= (src_take ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      out_e0 <= ((out_push & (!out_widx)) ? doubled : out_e0);
      out_e1 <= ((out_push & out_widx) ? doubled : out_e1);
      out_wsalt_q <= (out_push ? (out_wsalt_q ^ (out_widx ? 2'd2 : 2'd1)) : out_wsalt_q);
      x_s1 <= (shift ? src_item : x_s1);
    end
  end

endmodule

module ddl_merge_2x32 (
    input         clk,
    input         rst_n,
    input  [1:0]  i0_wsalt,
    output [1:0]  i0_rsalt,
    input  [63:0] i0_data,
    input  [1:0]  i1_wsalt,
    output [1:0]  i1_rsalt,
    input  [63:0] i1_data,
    output [1:0]  o_wsalt,
    input  [1:0]  o_rsalt,
    output [63:0] o_data
);

  reg [1:0] i0_rsalt_q;
  reg [1:0] i1_rsalt_q;
  reg [31:0] o_e0;
  reg [31:0] o_e1;
  reg [1:0] o_wsalt_q;
  reg turn;

  wire o_widx = o_wsalt_q[0] ^ o_wsalt_q[1];
  wire o_full = o_wsalt_q == (~o_rsalt);
  wire room = !o_full;
  wire i0_empty = i0_wsalt == i0_rsalt_q;
  wire i0_offered = !i0_empty;
  wire i0_ridx = i0_rsalt_q[0] ^ i0_rsalt_q[1];
  wire [31:0] i0_item = i0_ridx ? i0_data[63:32] : i0_data[31:0];
  wire i1_empty = i1_wsalt == i1_rsalt_q;
  wire i1_offered = !i1_empty;
  wire i1_ridx = i1_rsalt_q[0] ^ i1_rsalt_q[1];
  wire [31:0] i1_item = i1_ridx ? i1_data[63:32] : i1_data[31:0];
  wire grant0 = i0_offered & ((turn == 1'b0) | ((turn == 1'b1) & (!i1_offered)));
  wire grant1 = i1_offered & (((turn == 1'b0) & (!i0_offered)) | (turn == 1'b1));
  wire push = (grant0 | grant1) & room;
  wire [31:0] item = grant1 ? i1_item : i0_item;
  wire take0 = grant0 & push;
  wire take1 = grant1 & push;

  assign o_data = {o_e1, o_e0};
  assign o_wsalt = o_wsalt_q;
  assign i0_rsalt = i0_rsalt_q;
  assign i1_rsalt = i1_rsalt_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      i0_rsalt_q <= 2'd0;
      i1_rsalt_q <= 2'd0;
      o_e0 <= 32'd0;
      o_e1 <= 32'd0;
      o_wsalt_q <= 2'd0;
      turn <= 1'b0;
    end else begin
      i0_rsalt_q <= (take0 ? (i0_rsalt_q ^ (i0_ridx ? 2'd2 : 2'd1)) : i0_rsalt_q);
      i1_rsalt_q <= (take1 ? (i1_rsalt_q ^ (i1_ridx ? 2'd2 : 2'd1)) : i1_rsalt_q);
      o_e0 <= ((push & (!o_widx)) ? item : o_e0);
      o_e1 <= ((push & o_widx) ? item : o_e1);
      o_wsalt_q <= (push ? (o_wsalt_q ^ (o_widx ? 2'd2 : 2'd1)) : o_wsalt_q);
      turn <= ((grant1 & room) ? 1'b0 : ((grant0 & room) ? 1'b1 : turn));
    end
  end

endmodule

module ddl_split_2x32 (
    input         clk,
    input         rst_n,
    input  [1:0]  i_wsalt,
    output [1:0]  i_rsalt,
    input  [63:0] i_data,
    output [1:0]  o0_wsalt,
    input  [1:0]  o0_rsalt,
    output [63:0] o0_data,
    output [1:0]  o1_wsalt,
    input  [1:0]  o1_rsalt,
    output [63:0] o1_data
);

  reg [1:0] i_rsalt_q;
  reg [31:0] o0_e0;
  reg [31:0] o0_e1;
  reg [1:0] o0_wsalt_q;
  reg [31:0] o1_e0;
  reg [31:0] o1_e1;
  reg [1:0] o1_wsalt_q;

  wire i_empty = i_wsalt == i_rsalt_q;
  wire i_ridx = i_rsalt_q[0] ^ i_rsalt_q[1];
  wire [31:0] i_item = i_ridx ? i_data[63:32] : i_data[31:0];
  wire o0_widx = o0_wsalt_q[0] ^ o0_wsalt_q[1];
  wire o0_full = o0_wsalt_q == (~o0_rsalt);
  wire o1_widx = o1_wsalt_q[0] ^ o1_wsalt_q[1];
  wire o1_full = o1_wsalt_q == (~o1_rsalt);
  wire all_room = (!o0_full) & (!o1_full);
  wire take = (!i_empty) & all_room;

  assign o0_data = {o0_e1, o0_e0};
  assign o0_wsalt = o0_wsalt_q;
  assign o1_data = {o1_e1, o1_e0};
  assign o1_wsalt = o1_wsalt_q;
  assign i_rsalt = i_rsalt_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      i_rsalt_q <= 2'd0;
      o0_e0 <= 32'd0;
      o0_e1 <= 32'd0;
      o0_wsalt_q <= 2'd0;
      o1_e0 <= 32'd0;
      o1_e1 <= 32'd0;
      o1_wsalt_q <= 2'd0;
    end else begin
      i_rsalt_q <= (take ? (i_rsalt_q ^ (i_ridx ? 2'd2 : 2'd1)) : i_rsalt_q);
      o0_e0 <= ((take & (!o0_widx)) ? i_item : o0_e0);
      o0_e1 <= ((take & o0_widx) ? i_item : o0_e1);
      o0_wsalt_q <= (take ? (o0_wsalt_q ^ (o0_widx ? 2'd2 : 2'd1)) : o0_wsalt_q);
      o1_e0 <= ((take & (!o1_widx)) ? i_item : o1_e0);
      o1_e1 <= ((take & o1_widx) ? i_item : o1_e1);
      o1_wsalt_q <= (take ? (o1_wsalt_q ^ (o1_widx ? 2'd2 : 2'd1)) : o1_wsalt_q);
    end
  end

endmodule

module fanout (
    input         clk,
    input         rst_n,
    input  [1:0]  hi_wsalt,
    output [1:0]  hi_rsalt,
    input  [63:0] hi_data,
    input  [1:0]  lo_wsalt,
    output [1:0]  lo_rsalt,
    input  [63:0] lo_data,
    output [1:0]  out_a_wsalt,
    input  [1:0]  out_a_rsalt,
    output [63:0] out_a_data
);

  wire [1:0] picked_wsalt;
  wire [1:0] picked_rsalt;
  wire [63:0] picked_data;
  wire [1:0] scaled_wsalt;
  wire [1:0] scaled_rsalt;
  wire [63:0] scaled_data;
  wire [1:0] copy_wsalt;
  wire [1:0] copy_rsalt;
  wire [63:0] copy_data;

  ddl_merge_2x32 u_ddl_merge_2x32 (
    .clk      (clk),
    .rst_n    (rst_n),
    .i0_wsalt (hi_wsalt),
    .i0_rsalt (hi_rsalt),
    .i0_data  (hi_data),
    .i1_wsalt (lo_wsalt),
    .i1_rsalt (lo_rsalt),
    .i1_data  (lo_data),
    .o_wsalt  (picked_wsalt),
    .o_rsalt  (picked_rsalt),
    .o_data   (picked_data)
  );

  scale u_scale (
    .clk       (clk),
    .rst_n     (rst_n),
    .src_wsalt (picked_wsalt),
    .src_rsalt (picked_rsalt),
    .src_data  (picked_data),
    .dst_wsalt (scaled_wsalt),
    .dst_rsalt (scaled_rsalt),
    .dst_data  (scaled_data)
  );

  ddl_split_2x32 u_ddl_split_2x32 (
    .clk      (clk),
    .rst_n    (rst_n),
    .i_wsalt  (scaled_wsalt),
    .i_rsalt  (scaled_rsalt),
    .i_data   (scaled_data),
    .o0_wsalt (out_a_wsalt),
    .o0_rsalt (out_a_rsalt),
    .o0_data  (out_a_data),
    .o1_wsalt (copy_wsalt),
    .o1_rsalt (copy_rsalt),
    .o1_data  (copy_data)
  );

  sink_ext u_sink_ext (
    .clk     (clk),
    .rst_n   (rst_n),
    .a_wsalt (copy_wsalt),
    .a_rsalt (copy_rsalt),
    .a_data  (copy_data)
  );

endmodule
