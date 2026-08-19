// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/pipeline_graph.ddl -o examples/pipeline_graph.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module mul3 (
    input         clk,
    input         rst_n,
    input         src_valid,
    output        src_ready,
    input  [15:0] src_data,
    output        dst_valid,
    input         dst_ready,
    output [31:0] dst_data
);

  reg v0;
  reg v1;
  reg v2;
  reg [15:0] doubled_s1;
  reg [31:0] wide_s2;
  reg [31:0] out_hold;

  wire shift = (!v2) | dst_ready;
  wire [15:0] doubled = src_data + src_data;
  wire [31:0] wide = {{16{1'b0}}, doubled_s1};
  wire [31:0] scaled = wide_s2 + wide_s2;

  assign src_ready = shift;
  assign dst_valid = v2;
  assign dst_data = out_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      v2 <= 1'b0;
      doubled_s1 <= 16'd0;
      wide_s2 <= 32'd0;
      out_hold <= 32'd0;
    end else begin
      v0 <= (shift ? src_valid : v0);
      v1 <= (shift ? v0 : v1);
      v2 <= (shift ? v1 : v2);
      doubled_s1 <= (shift ? doubled : doubled_s1);
      wide_s2 <= (shift ? wide : wide_s2);
      out_hold <= (shift ? scaled : out_hold);
    end
  end

endmodule

module add_one (
    input         clk,
    input         rst_n,
    input         src_valid,
    output        src_ready,
    input  [15:0] src_data,
    output        dst_valid,
    input         dst_ready,
    output [15:0] dst_data
);

  reg v0;
  reg v1;
  reg [15:0] a_s1;
  reg [15:0] out_hold;

  wire shift = (!v1) | dst_ready;
  wire [15:0] b = a_s1 + 16'd1;

  assign src_ready = shift;
  assign dst_valid = v1;
  assign dst_data = out_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      a_s1 <= 16'd0;
      out_hold <= 16'd0;
    end else begin
      v0 <= (shift ? src_valid : v0);
      v1 <= (shift ? v0 : v1);
      a_s1 <= (shift ? src_data : a_s1);
      out_hold <= (shift ? b : out_hold);
    end
  end

endmodule

module saturate (
    input         clk,
    input         rst_n,
    input         src_valid,
    output        src_ready,
    input  [31:0] src_data,
    output        dst_valid,
    input         dst_ready,
    output [31:0] dst_data
);

  reg v0;
  reg v1;
  reg [31:0] a_s1;
  reg [31:0] out_hold;

  wire shift = (!v1) | dst_ready;
  wire too_big = a_s1 > 32'hFFFF;
  wire [31:0] b = too_big ? 32'hFFFF : a_s1;

  assign src_ready = shift;
  assign dst_valid = v1;
  assign dst_data = out_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      a_s1 <= 32'd0;
      out_hold <= 32'd0;
    end else begin
      v0 <= (shift ? src_valid : v0);
      v1 <= (shift ? v0 : v1);
      a_s1 <= (shift ? src_data : a_s1);
      out_hold <= (shift ? b : out_hold);
    end
  end

endmodule

module scaler (
    input         clk,
    input         rst_n,
    input         src_valid,
    output        src_ready,
    input  [15:0] src_data,
    output        dst_valid,
    input         dst_ready,
    output [31:0] dst_data
);

  wire bumped_valid;
  wire bumped_ready;
  wire [15:0] bumped_data;
  wire scaled_valid;
  wire scaled_ready;
  wire [31:0] scaled_data;

  add_one u_add_one (
    .clk       (clk),
    .rst_n     (rst_n),
    .src_valid (src_valid),
    .src_ready (src_ready),
    .src_data  (src_data),
    .dst_valid (bumped_valid),
    .dst_ready (bumped_ready),
    .dst_data  (bumped_data)
  );

  mul3 u_mul3 (
    .clk       (clk),
    .rst_n     (rst_n),
    .src_valid (bumped_valid),
    .src_ready (bumped_ready),
    .src_data  (bumped_data),
    .dst_valid (scaled_valid),
    .dst_ready (scaled_ready),
    .dst_data  (scaled_data)
  );

  saturate u_saturate (
    .clk       (clk),
    .rst_n     (rst_n),
    .src_valid (scaled_valid),
    .src_ready (scaled_ready),
    .src_data  (scaled_data),
    .dst_valid (dst_valid),
    .dst_ready (dst_ready),
    .dst_data  (dst_data)
  );

endmodule
