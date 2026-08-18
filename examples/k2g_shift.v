// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build E:/Code/ddl/examples/k2g_shift.ddl -o E:/Code/ddl/examples/k2g_shift.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module k2g_shift (
    input  [31:0] value,
    input  [31:0] src,
    input  [4:0]  amount,
    input  [1:0]  shift_op,
    input  [4:0]  bm_start,
    input  [4:0]  bm_span,
    output [31:0] shift_result,
    output [31:0] bext_result,
    output [31:0] bins_result
);

  wire signed [31:0] n12 = value;
  wire signed [31:0] n13 = n12 >>> amount;
  wire [31:0] n14 = n13;
  wire [31:0] n26 = (bm_span == 5'd0) ? 32'd0 : ((32'd1 << bm_span) - 32'd1);
  wire [31:0] placed_mask = n26 << bm_start;
  wire [31:0] placed_src = (src & n26) << bm_start;

  assign shift_result = ((shift_op == 2'd0) ? (value << amount) : ((shift_op == 2'd1) ? (value >> amount) : n14));
  assign bext_result = ((src >> bm_start) & n26);
  assign bins_result = ((value & (~placed_mask)) | placed_src);

endmodule
