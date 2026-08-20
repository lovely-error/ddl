// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/pipeline_graph.ddl -I ../KAMASUTRA2G/rtl -o examples/pipeline_graph.v
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
  reg out_skid_busy;
  reg [31:0] out_skid;
  reg [15:0] doubled_s1;
  reg [31:0] wide_s2;
  reg [31:0] out_hold;

  wire shift = !out_skid_busy;
  wire [15:0] doubled = src_data + src_data;
  wire [31:0] wide = {{16{1'b0}}, doubled_s1};
  wire [31:0] scaled = wide_s2 + wide_s2;
  wire n19 = v2 & dst_ready;
  wire n20 = !n19;
  wire n21 = shift & v1;
  wire n24 = n21 & ((!v2) | n19);
  wire n25 = n19 & out_skid_busy;
  wire n26 = v2 & n20;
  wire n31 = n21 & n26;

  assign src_ready = shift;
  assign dst_valid = v2;
  assign dst_data = out_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      v2 <= 1'b0;
      out_skid_busy <= 1'b0;
      out_skid <= 32'd0;
      doubled_s1 <= 16'd0;
      wide_s2 <= 32'd0;
      out_hold <= 32'd0;
    end else begin
      v0 <= (shift ? src_valid : v0);
      v1 <= (shift ? v0 : v1);
      v2 <= ((n26 | n25) | n24);
      out_skid_busy <= ((out_skid_busy & n20) | n31);
      out_skid <= (n31 ? scaled : out_skid);
      doubled_s1 <= (shift ? doubled : doubled_s1);
      wide_s2 <= (shift ? wide : wide_s2);
      out_hold <= (n25 ? out_skid : (n24 ? scaled : out_hold));
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
  reg out_skid_busy;
  reg [15:0] out_skid;
  reg [15:0] a_s1;
  reg [15:0] out_hold;

  wire shift = !out_skid_busy;
  wire [15:0] b = a_s1 + 16'd1;
  wire n16 = v1 & dst_ready;
  wire n17 = !n16;
  wire n18 = shift & v0;
  wire n21 = n18 & ((!v1) | n16);
  wire n22 = n16 & out_skid_busy;
  wire n23 = v1 & n17;
  wire n28 = n18 & n23;

  assign src_ready = shift;
  assign dst_valid = v1;
  assign dst_data = out_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      out_skid_busy <= 1'b0;
      out_skid <= 16'd0;
      a_s1 <= 16'd0;
      out_hold <= 16'd0;
    end else begin
      v0 <= (shift ? src_valid : v0);
      v1 <= ((n23 | n22) | n21);
      out_skid_busy <= ((out_skid_busy & n17) | n28);
      out_skid <= (n28 ? b : out_skid);
      a_s1 <= (shift ? src_data : a_s1);
      out_hold <= (n22 ? out_skid : (n21 ? b : out_hold));
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
  reg out_skid_busy;
  reg [31:0] out_skid;
  reg [31:0] a_s1;
  reg [31:0] out_hold;

  wire shift = !out_skid_busy;
  wire too_big = a_s1 > 32'hFFFF;
  wire [31:0] b = too_big ? 32'hFFFF : a_s1;
  wire n18 = v1 & dst_ready;
  wire n19 = !n18;
  wire n20 = shift & v0;
  wire n23 = n20 & ((!v1) | n18);
  wire n24 = n18 & out_skid_busy;
  wire n25 = v1 & n19;
  wire n30 = n20 & n25;

  assign src_ready = shift;
  assign dst_valid = v1;
  assign dst_data = out_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0;
      v1 <= 1'b0;
      out_skid_busy <= 1'b0;
      out_skid <= 32'd0;
      a_s1 <= 32'd0;
      out_hold <= 32'd0;
    end else begin
      v0 <= (shift ? src_valid : v0);
      v1 <= ((n25 | n24) | n23);
      out_skid_busy <= ((out_skid_busy & n19) | n30);
      out_skid <= (n30 ? b : out_skid);
      a_s1 <= (shift ? src_data : a_s1);
      out_hold <= (n24 ? out_skid : (n23 ? b : out_hold));
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
