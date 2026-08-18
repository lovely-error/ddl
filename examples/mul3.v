// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build E:/Code/ddl/examples/mul3.ddl -o E:/Code/ddl/examples/mul3.v
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
