// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/mul3.ddl -o examples/mul3.v
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
