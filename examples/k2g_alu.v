// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/k2g_alu.ddl -I ../KAMASUTRA2G/rtl -o examples/k2g_alu.v
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

module rdt_is_signed (
    input  [2:0] t,
    output       signed_
);

  assign signed_ = t[2];

endmodule

module k2g_alu (
    input  [31:0] a,
    input  [31:0] b,
    input  [2:0]  a_tag,
    input  [2:0]  b_tag,
    input  [1:0]  arith_op,
    input  [1:0]  logic_op,
    input  [2:0]  cmp_op,
    input  [1:0]  unary_op,
    output [31:0] arith_result,
    output        arith_overflow,
    output [31:0] logic_result,
    output        cmp_result,
    output [31:0] unary_result
);

  wire [32:0] a33 = {1'b0, a};
  wire [32:0] b33 = {1'b0, b};
  wire [32:0] sum = a33 + b33;
  wire [32:0] diff = a33 - b33;
  reg n18;
  reg [31:0] n19;
  reg [31:0] n23;
  reg [31:0] n28;
  wire a_signed = a_tag[2];
  wire b_signed = b_tag[2];
  wire a_top = a_signed & a[31];
  wire b_top = b_signed & b[31];
  wire signed [32:0] a_wide = {a_top, a};
  wire signed [32:0] b_wide = {b_top, b};
  reg n45;

  always @* begin
    case (arith_op)
      2'd1: n18 = diff[32];
      default: n18 = sum[32];
    endcase
  end

  always @* begin
    case (arith_op)
      2'd1: n19 = diff[31:0];
      default: n19 = sum[31:0];
    endcase
  end

  always @* begin
    case (logic_op)
      2'd0: n23 = (a & b);
      2'd1: n23 = (a | b);
      default: n23 = (a ^ b);
    endcase
  end

  always @* begin
    case (unary_op)
      2'd0: n28 = (~a);
      default: n28 = ((~a) + 32'd1);
    endcase
  end

  always @* begin
    case (cmp_op)
      3'd0: n45 = (a_wide == b_wide);
      3'd1: n45 = (a_wide != b_wide);
      3'd2: n45 = (a_wide < b_wide);
      3'd3: n45 = (a_wide > b_wide);
      3'd4: n45 = (a_wide <= b_wide);
      default: n45 = (a_wide >= b_wide);
    endcase
  end

  assign arith_result = n19;
  assign arith_overflow = n18;
  assign logic_result = n23;
  assign cmp_result = n45;
  assign unary_result = n28;

endmodule
