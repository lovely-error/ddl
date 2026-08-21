// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/pipeline_graph.ddl -I ../KAMASUTRA2G/rtl -o examples/pipeline_graph.v
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
  reg [31:0] out_e0;
  reg [31:0] out_e1;
  reg [1:0] out_wsalt_q;
  reg [15:0] doubled_s1;
  reg [31:0] wide_s2;

  wire dst_full = out_wsalt_q == (~dst_rsalt);
  wire shift = !dst_full;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [15:0] src_item = src_ridx ? src_data[31:16] : src_data[15:0];
  wire [15:0] doubled = src_item + src_item;
  wire [31:0] wide = {{16{1'b0}}, doubled_s1};
  wire [31:0] scaled = wide_s2 + wide_s2;
  wire src_empty = src_wsalt == src_rsalt_q;
  wire n24 = !src_empty;
  wire out_push = shift & v1;
  wire out_widx = out_wsalt_q[0] ^ out_wsalt_q[1];
  wire src_take = n24 & shift;

  assign src_rsalt = src_rsalt_q;
  assign dst_wsalt = out_wsalt_q;
  assign dst_data = {out_e1, out_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      src_rsalt_q <= 2'd0;
      out_e0 <= 32'd0;
      out_e1 <= 32'd0;
      out_wsalt_q <= 2'd0;
      doubled_s1 <= 16'd0;
      wide_s2 <= 32'd0;
    end else begin
      v0 <= (shift ? n24 : v0);
      v1 <= (shift ? v0 : v1);
      src_rsalt_q <= (src_take ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      out_e0 <= ((out_push & (!out_widx)) ? scaled : out_e0);
      out_e1 <= ((out_push & out_widx) ? scaled : out_e1);
      out_wsalt_q <= (out_push ? (out_wsalt_q ^ (out_widx ? 2'd2 : 2'd1)) : out_wsalt_q);
      doubled_s1 <= (shift ? doubled : doubled_s1);
      wide_s2 <= (shift ? wide : wide_s2);
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
  reg [15:0] out_e0;
  reg [15:0] out_e1;
  reg [1:0] out_wsalt_q;
  reg [15:0] a_s1;

  wire dst_full = out_wsalt_q == (~dst_rsalt);
  wire shift = !dst_full;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [15:0] src_item = src_ridx ? src_data[31:16] : src_data[15:0];
  wire [15:0] b = a_s1 + 16'd1;
  wire src_empty = src_wsalt == src_rsalt_q;
  wire n21 = !src_empty;
  wire out_push = shift & v0;
  wire out_widx = out_wsalt_q[0] ^ out_wsalt_q[1];
  wire src_take = n21 & shift;

  assign src_rsalt = src_rsalt_q;
  assign dst_wsalt = out_wsalt_q;
  assign dst_data = {out_e1, out_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      src_rsalt_q <= 2'd0;
      out_e0 <= 16'd0;
      out_e1 <= 16'd0;
      out_wsalt_q <= 2'd0;
      a_s1 <= 16'd0;
    end else begin
      v0 <= (shift ? n21 : v0);
      src_rsalt_q <= (src_take ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      out_e0 <= ((out_push & (!out_widx)) ? b : out_e0);
      out_e1 <= ((out_push & out_widx) ? b : out_e1);
      out_wsalt_q <= (out_push ? (out_wsalt_q ^ (out_widx ? 2'd2 : 2'd1)) : out_wsalt_q);
      a_s1 <= (shift ? src_item : a_s1);
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
  reg [31:0] out_e0;
  reg [31:0] out_e1;
  reg [1:0] out_wsalt_q;
  reg [31:0] a_s1;

  wire dst_full = out_wsalt_q == (~dst_rsalt);
  wire shift = !dst_full;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [31:0] src_item = src_ridx ? src_data[63:32] : src_data[31:0];
  wire too_big = a_s1 > 32'hFFFF;
  wire [31:0] b = too_big ? 32'hFFFF : a_s1;
  wire src_empty = src_wsalt == src_rsalt_q;
  wire n23 = !src_empty;
  wire out_push = shift & v0;
  wire out_widx = out_wsalt_q[0] ^ out_wsalt_q[1];
  wire src_take = n23 & shift;

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
      a_s1 <= 32'd0;
    end else begin
      v0 <= (shift ? n23 : v0);
      src_rsalt_q <= (src_take ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      out_e0 <= ((out_push & (!out_widx)) ? b : out_e0);
      out_e1 <= ((out_push & out_widx) ? b : out_e1);
      out_wsalt_q <= (out_push ? (out_wsalt_q ^ (out_widx ? 2'd2 : 2'd1)) : out_wsalt_q);
      a_s1 <= (shift ? src_item : a_s1);
    end
  end

endmodule

module scaler (
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
