// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/k2g_shift.ddl -I ../KAMASUTRA2G/rtl -o examples/k2g_shift.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module uop_nop (
    output [127:0] u
);

  assign u = 128'd0;

endmodule

module uop_fault (
    input  [4:0]   cause,
    input  [15:0]  cp,
    output [127:0] u
);

  wire [127:0] f = 128'd0;
  wire [127:0] n5 = {5'h12, f[122:0]};
  wire [127:0] n8 = {n5[127:37], cause, n5[31:0]};

  assign u = {n8[127:113], {16'd0, cp}, n8[80:0]};

endmodule

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

  wire signed [31:0] n16 = value;
  wire signed [31:0] n17 = n16 >>> amount;
  wire [31:0] n18 = n17;
  wire [31:0] span_mask = (bm_span == 5'd0) ? 32'd0 : ((32'd1 << bm_span) - 32'd1);
  wire [31:0] placed_mask = span_mask << bm_start;
  wire [31:0] placed_src = (src & span_mask) << bm_start;

  assign shift_result = ((shift_op == 2'd0) ? (value << amount) : ((shift_op == 2'd1) ? (value >> amount) : n18));
  assign bext_result = ((src >> bm_start) & span_mask);
  assign bins_result = ((value & (~placed_mask)) | placed_src);

endmodule
