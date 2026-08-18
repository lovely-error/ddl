// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build E:/Code/ddl/target/verify/k2g_xstage/src.ddl -o E:/Code/ddl/examples/k2g_xstage.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module uop_nop (
    output [126:0] u
);

  assign u = 127'd0;

endmodule

module uop_fault (
    input  [4:0]   cause,
    input  [15:0]  cp,
    output [126:0] u
);

  wire [126:0] f = 127'd0;
  wire [126:0] n5 = {5'h12, f[121:0]};
  wire [126:0] n8 = {n5[126:37], cause, n5[31:0]};

  assign u = {n8[126:112], {16'd0, cp}, n8[79:0]};

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

module k2g_xstage (
    input          clk,
    input          rst_n,
    input          uops_valid,
    output         uops_ready,
    input  [126:0] uops_data,
    output         wb_valid,
    input          wb_ready,
    output [45:0]  wb_data
);

  reg [45:0] w;
  reg wb_busy;
  reg [45:0] wb_hold;

  reg [31:0] values [0:31];
  integer values_ix;
  reg [2:0] tags [0:31];
  integer tags_ix;
  reg overflow [0:31];
  integer overflow_ix;
  reg flagbit [0:31];
  integer flagbit_ix;

  wire n25 = (!wb_busy) | wb_ready;
  wire uops_xfer = uops_valid & n25;
  wire [4:0] ra = uops_data[121:117];
  wire [4:0] rb = uops_data[116:112];
  wire [31:0] ra_value = values[ra];
  wire [31:0] rb_value = values[rb];
  wire [2:0] ra_tag = tags[ra];
  wire [2:0] rb_tag = tags[rb];
  wire rb_ovf = overflow[rb];
  wire rb_flg = flagbit[rb];
  wire writing = ((w[45] | w[44]) | w[43]) | w[42];
  wire ma = writing & (w[41:37] == ra);
  wire mb = writing & (w[41:37] == rb);
  wire [31:0] xa_value = (ma & w[45]) ? w[36:5] : ra_value;
  wire [31:0] xb_value = (mb & w[45]) ? w[36:5] : rb_value;
  wire [2:0] xa_tag = (ma & w[44]) ? w[4:2] : ra_tag;
  wire [2:0] xb_tag = (mb & w[44]) ? w[4:2] : rb_tag;
  wire xb_ovf = (mb & w[43]) ? w[1] : rb_ovf;
  wire xb_flg = (mb & w[42]) ? w[0] : rb_flg;
  wire [31:0] alu_b = uops_data[79] ? uops_data[111:80] : xb_value;
  wire [2:0] alu_b_tag = uops_data[79] ? xa_tag : xb_tag;
  wire [1:0] n117 = uops_data[69:68];
  wire [1:0] n119 = uops_data[67:66];
  wire [2:0] n121 = uops_data[63:61];
  wire [1:0] n123 = uops_data[60:59];
  wire [32:0] a33 = {1'b0, xa_value};
  wire [32:0] b33 = {1'b0, alu_b};
  wire [32:0] sum = a33 + b33;
  wire [32:0] diff = a33 - b33;
  reg arith_ovf;
  reg [31:0] arith_result;
  reg [31:0] logic_result;
  reg [31:0] unary_result;
  wire a_signed = xa_tag[2];
  wire b_signed = alu_b_tag[2];
  wire a_top = a_signed & xa_value[31];
  wire b_top = b_signed & alu_b[31];
  wire signed [32:0] a_wide = {a_top, xa_value};
  wire signed [32:0] b_wide = {b_top, alu_b};
  reg cmp_result;
  wire [31:0] n163 = uops_data[111:80];
  wire [4:0] amount = uops_data[79] ? n163[4:0] : xb_value[4:0];
  wire [4:0] n169 = uops_data[46:42];
  wire [4:0] n170 = uops_data[41:37];
  wire signed [31:0] n181 = xa_value;
  wire signed [31:0] n182 = n181 >>> amount;
  wire [31:0] n183 = n182;
  wire [31:0] shift_result = (uops_data[65:64] == 2'd0) ? (xa_value << amount) : ((uops_data[65:64] == 2'd1) ? (xa_value >> amount) : n183);
  wire [31:0] span_mask = (n170 == 5'd0) ? 32'd0 : ((32'd1 << n170) - 32'd1);
  wire [31:0] bext_result = (xb_value >> n169) & span_mask;
  wire [31:0] placed_mask = span_mask << n169;
  wire [31:0] placed_src = (xb_value & span_mask) << n169;
  wire [31:0] bins_result = (xa_value & (~placed_mask)) | placed_src;
  wire [31:0] load_addr = xb_value + uops_data[111:80];
  wire [31:0] store_addr = xa_value + uops_data[111:80];
  wire is_load = uops_data[126:122] == 5'd4;
  wire [31:0] access_addr = is_load ? load_addr : store_addr;
  wire [2:0] access_tag = is_load ? uops_data[49:47] : xb_tag;
  wire [45:0] n = 46'd0;
  wire [45:0] n220 = {n[45:42], uops_data[121:117], n[36:0]};
  wire [45:0] n224 = {n220[45:37], 32'd0, n220[4:0]};
  wire [45:0] n227 = {n224[45:5], xa_tag, n224[1:0]};
  wire [45:0] n231 = {n227[45:2], 1'b0, n227[0]};
  wire [45:0] n234 = {n231[45:1], 1'b0};
  wire [4:0] n236 = uops_data[126:122];
  wire [45:0] n238 = {uops_xfer, n234[44:0]};
  wire [45:0] n241 = {n238[45], uops_xfer, n238[43:0]};
  wire [45:0] n245 = {n241[45:37], uops_data[111:80], n241[4:0]};
  wire [45:0] n253 = {n234[45], uops_xfer, n234[43:0]};
  wire [45:0] n260 = {uops_xfer, n234[44:0]};
  wire [45:0] n263 = {n260[45], uops_xfer, n260[43:0]};
  wire [45:0] n266 = {n263[45:44], uops_xfer, n263[42:0]};
  wire [45:0] n269 = {n266[45:43], uops_xfer, n266[41:0]};
  wire [45:0] n272 = {n269[45:37], xb_value, n269[4:0]};
  wire [45:0] n275 = {n272[45:5], xb_tag, n272[1:0]};
  wire [45:0] n278 = {n275[45:2], xb_ovf, n275[0]};
  wire [45:0] n282 = {uops_xfer, n234[44:0]};
  wire [45:0] n285 = {n282[45:44], uops_xfer, n282[42:0]};
  wire [45:0] n288 = {n285[45:37], arith_result, n285[4:0]};
  wire [45:0] n293 = {uops_xfer, n234[44:0]};
  wire [45:0] n298 = {uops_xfer, n234[44:0]};
  wire [45:0] n304 = {n234[45:43], uops_xfer, n234[41:0]};
  wire [45:0] n308 = {uops_xfer, n234[44:0]};
  wire [45:0] n313 = {uops_xfer, n234[44:0]};
  wire [45:0] n318 = {uops_xfer, n234[44:0]};
  wire [45:0] n324 = {1'b0, n234[44:0]};
  wire [45:0] n327 = {n324[45:37], access_addr, n324[4:0]};
  reg [45:0] n334;
  wire n341 = w[44];

  always @* begin
    case (n117)
      2'd1: arith_ovf = diff[32];
      default: arith_ovf = sum[32];
    endcase
  end

  always @* begin
    case (n117)
      2'd1: arith_result = diff[31:0];
      default: arith_result = sum[31:0];
    endcase
  end

  always @* begin
    case (n119)
      2'd0: logic_result = (xa_value & alu_b);
      2'd1: logic_result = (xa_value | alu_b);
      default: logic_result = (xa_value ^ alu_b);
    endcase
  end

  always @* begin
    case (n123)
      2'd0: unary_result = (~xa_value);
      default: unary_result = ((~xa_value) + 32'd1);
    endcase
  end

  always @* begin
    case (n121)
      3'd0: cmp_result = (a_wide == b_wide);
      3'd1: cmp_result = (a_wide != b_wide);
      3'd2: cmp_result = (a_wide < b_wide);
      3'd3: cmp_result = (a_wide > b_wide);
      3'd4: cmp_result = (a_wide <= b_wide);
      default: cmp_result = (a_wide >= b_wide);
    endcase
  end

  always @* begin
    case (n236)
      5'd1: n334 = {n245[45:5], uops_data[49:47], n245[1:0]};
      5'd2: n334 = {n253[45:5], uops_data[49:47], n253[1:0]};
      5'd3: n334 = {n278[45:1], xb_flg};
      5'd8: n334 = {n288[45:2], arith_ovf, n288[0]};
      5'd9: n334 = {n293[45:37], logic_result, n293[4:0]};
      5'h10: n334 = {n298[45:37], unary_result, n298[4:0]};
      5'hD: n334 = {n304[45:1], cmp_result};
      5'hA: n334 = {n308[45:37], shift_result, n308[4:0]};
      5'hB: n334 = {n313[45:37], bext_result, n313[4:0]};
      5'hC: n334 = {n318[45:37], bins_result, n318[4:0]};
      5'd4, 5'd5: n334 = {n327[45:5], access_tag, n327[1:0]};
      default: n334 = {1'b0, n234[44:0]};
    endcase
  end

  assign uops_ready = n25;
  assign wb_valid = wb_busy;
  assign wb_data = wb_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      w <= 46'd0;
      wb_busy <= 1'b0;
      wb_hold <= 46'd0;
    end else begin
      w <= n334;
      wb_busy <= (uops_xfer ? 1'b1 : (wb_ready ? 1'b0 : wb_busy));
      wb_hold <= (uops_xfer ? n334 : wb_hold);
    end
  end

  // values [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (values_ix = 0; values_ix < 32; values_ix = values_ix + 1) values[values_ix] <= 32'd0;
    end else if (w[45]) begin
      values[w[41:37]] <= w[36:5];
    end
  end

  // tags [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (tags_ix = 0; tags_ix < 32; tags_ix = tags_ix + 1) tags[tags_ix] <= 3'd2;
    end else if (n341) begin
      tags[w[41:37]] <= w[4:2];
    end
  end

  // overflow [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (overflow_ix = 0; overflow_ix < 32; overflow_ix = overflow_ix + 1) overflow[overflow_ix] <= 1'b0;
    end else if (w[43]) begin
      overflow[w[41:37]] <= w[1];
    end
  end

  // flagbit [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (flagbit_ix = 0; flagbit_ix < 32; flagbit_ix = flagbit_ix + 1) flagbit[flagbit_ix] <= 1'b0;
    end else if (w[42]) begin
      flagbit[w[41:37]] <= w[0];
    end
  end

`ifdef SIMULATION
  always @(posedge clk) begin
    if (rst_n) begin
      if (!(((!n341) | (w[4:2] != 3'd7)))) $error("%m: the reserved tag encoding was written");
    end
  end
`endif

endmodule
