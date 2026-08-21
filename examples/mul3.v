// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/mul3.ddl -o examples/mul3.v
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
