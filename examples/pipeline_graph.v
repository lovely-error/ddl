// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/pipeline_graph.ddl -o examples/pipeline_graph.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module mul3 (
    input         clk,
    input         rst_n,
    input  [1:0]  src_wsalt,
    output [1:0]  src_rsalt,
    input  [31:0] src_data,
    output [1:0]  dst_wsalt,
    input  [1:0]  dst_rsalt,
    output [63:0] dst_data
);

  reg v0;
  reg v1;
  reg [1:0] src_rsalt_q;
  reg [31:0] dst_e0;
  reg [31:0] dst_e1;
  reg [1:0] dst_wsalt_q;
  reg [15:0] doubled_s1;
  reg [31:0] wide_s2;

  wire dst_full = dst_wsalt_q == (~dst_rsalt);
  wire shift2 = !dst_full;
  wire shift1 = (!v1) | shift2;
  wire shift0 = (!v0) | shift1;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [15:0] src_item = src_ridx ? src_data[31:16] : src_data[15:0];
  wire [15:0] doubled = src_item + src_item;
  wire [31:0] wide = {{16{1'b0}}, doubled_s1};
  wire [31:0] scaled = wide_s2 + wide_s2;
  wire src_empty = src_wsalt == src_rsalt_q;
  wire src_present = !src_empty;
  wire push = shift2 & v1;
  wire dst_widx = dst_wsalt_q[0] ^ dst_wsalt_q[1];
  wire take = src_present & shift0;

  assign dst_data = {dst_e1, dst_e0};
  assign dst_wsalt = dst_wsalt_q;
  assign src_rsalt = src_rsalt_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      src_rsalt_q <= 2'd0;
      dst_e0 <= 32'd0;
      dst_e1 <= 32'd0;
      dst_wsalt_q <= 2'd0;
      doubled_s1 <= 16'd0;
      wide_s2 <= 32'd0;
    end else begin
      v0 <= (shift0 ? src_present : v0);
      v1 <= (shift1 ? v0 : v1);
      src_rsalt_q <= (take ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      dst_e0 <= ((push & (!dst_widx)) ? scaled : dst_e0);
      dst_e1 <= ((push & dst_widx) ? scaled : dst_e1);
      dst_wsalt_q <= (push ? (dst_wsalt_q ^ (dst_widx ? 2'd2 : 2'd1)) : dst_wsalt_q);
      doubled_s1 <= (shift0 ? doubled : doubled_s1);
      wide_s2 <= (shift1 ? wide : wide_s2);
    end
  end

endmodule

module add_one (
    input         clk,
    input         rst_n,
    input  [1:0]  src_wsalt,
    output [1:0]  src_rsalt,
    input  [31:0] src_data,
    output [1:0]  dst_wsalt,
    input  [1:0]  dst_rsalt,
    output [31:0] dst_data
);

  reg v0;
  reg [1:0] src_rsalt_q;
  reg [15:0] dst_e0;
  reg [15:0] dst_e1;
  reg [1:0] dst_wsalt_q;
  reg [15:0] a_s1;

  wire dst_full = dst_wsalt_q == (~dst_rsalt);
  wire shift1 = !dst_full;
  wire shift0 = (!v0) | shift1;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [15:0] src_item = src_ridx ? src_data[31:16] : src_data[15:0];
  wire [15:0] b = a_s1 + 16'd1;
  wire src_empty = src_wsalt == src_rsalt_q;
  wire src_present = !src_empty;
  wire push = shift1 & v0;
  wire dst_widx = dst_wsalt_q[0] ^ dst_wsalt_q[1];
  wire take = src_present & shift0;

  assign dst_data = {dst_e1, dst_e0};
  assign dst_wsalt = dst_wsalt_q;
  assign src_rsalt = src_rsalt_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      src_rsalt_q <= 2'd0;
      dst_e0 <= 16'd0;
      dst_e1 <= 16'd0;
      dst_wsalt_q <= 2'd0;
      a_s1 <= 16'd0;
    end else begin
      v0 <= (shift0 ? src_present : v0);
      src_rsalt_q <= (take ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      dst_e0 <= ((push & (!dst_widx)) ? b : dst_e0);
      dst_e1 <= ((push & dst_widx) ? b : dst_e1);
      dst_wsalt_q <= (push ? (dst_wsalt_q ^ (dst_widx ? 2'd2 : 2'd1)) : dst_wsalt_q);
      a_s1 <= (shift0 ? src_item : a_s1);
    end
  end

endmodule

module saturate (
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
  reg [31:0] dst_e0;
  reg [31:0] dst_e1;
  reg [1:0] dst_wsalt_q;
  reg [31:0] a_s1;

  wire dst_full = dst_wsalt_q == (~dst_rsalt);
  wire shift1 = !dst_full;
  wire shift0 = (!v0) | shift1;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [31:0] src_item = src_ridx ? src_data[63:32] : src_data[31:0];
  wire too_big = a_s1 > 32'hFFFF;
  wire [31:0] b = too_big ? 32'hFFFF : a_s1;
  wire src_empty = src_wsalt == src_rsalt_q;
  wire src_present = !src_empty;
  wire push = shift1 & v0;
  wire dst_widx = dst_wsalt_q[0] ^ dst_wsalt_q[1];
  wire take = src_present & shift0;

  assign dst_data = {dst_e1, dst_e0};
  assign dst_wsalt = dst_wsalt_q;
  assign src_rsalt = src_rsalt_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      src_rsalt_q <= 2'd0;
      dst_e0 <= 32'd0;
      dst_e1 <= 32'd0;
      dst_wsalt_q <= 2'd0;
      a_s1 <= 32'd0;
    end else begin
      v0 <= (shift0 ? src_present : v0);
      src_rsalt_q <= (take ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      dst_e0 <= ((push & (!dst_widx)) ? b : dst_e0);
      dst_e1 <= ((push & dst_widx) ? b : dst_e1);
      dst_wsalt_q <= (push ? (dst_wsalt_q ^ (dst_widx ? 2'd2 : 2'd1)) : dst_wsalt_q);
      a_s1 <= (shift0 ? src_item : a_s1);
    end
  end

endmodule

module scaler_core (
    input         clk,
    input         rst_n,
    input  [1:0]  src_wsalt,
    output [1:0]  src_rsalt,
    input  [31:0] src_data,
    output [1:0]  dst_wsalt,
    input  [1:0]  dst_rsalt,
    output [63:0] dst_data
);

  wire [1:0] bumped_wsalt;
  wire [1:0] bumped_rsalt;
  wire [31:0] bumped_data;
  wire [1:0] scaled_wsalt;
  wire [1:0] scaled_rsalt;
  wire [63:0] scaled_data;

  add_one u_add_one (
    .clk       (clk),
    .rst_n     (rst_n),
    .src_wsalt (src_wsalt),
    .src_rsalt (src_rsalt),
    .src_data  (src_data),
    .dst_wsalt (bumped_wsalt),
    .dst_rsalt (bumped_rsalt),
    .dst_data  (bumped_data)
  );

  mul3 u_mul3 (
    .clk       (clk),
    .rst_n     (rst_n),
    .src_wsalt (bumped_wsalt),
    .src_rsalt (bumped_rsalt),
    .src_data  (bumped_data),
    .dst_wsalt (scaled_wsalt),
    .dst_rsalt (scaled_rsalt),
    .dst_data  (scaled_data)
  );

  saturate u_saturate (
    .clk       (clk),
    .rst_n     (rst_n),
    .src_wsalt (scaled_wsalt),
    .src_rsalt (scaled_rsalt),
    .src_data  (scaled_data),
    .dst_wsalt (dst_wsalt),
    .dst_rsalt (dst_rsalt),
    .dst_data  (dst_data)
  );

endmodule

module ddl_wport_to_salt_16 (
    input         clk,
    input         rst_n,
    output [1:0]  o_wsalt,
    input  [1:0]  o_rsalt,
    output [31:0] o_data,
    output        can_receive,
    input         receive_en,
    input  [15:0] data_write_in
);

  reg [15:0] o_e0;
  reg [15:0] o_e1;
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
      o_e0 <= 16'd0;
      o_e1 <= 16'd0;
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

module scaler (
    input         clk,
    input         rst_n,
    output        src_can_receive,
    input         src_receive_en,
    input  [15:0] src_data_write_in,
    output        dst_has_data,
    input         dst_drop_item,
    output [31:0] dst_data_read_out
);

  wire [1:0] src_wsalt;
  wire [1:0] src_rsalt;
  wire [31:0] src_data;
  wire [1:0] dst_wsalt;
  wire [1:0] dst_rsalt;
  wire [63:0] dst_data;

  ddl_wport_to_salt_16 u_src_adapt (
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

  scaler_core u_scaler_core (
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
