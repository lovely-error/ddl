// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/k2g_xstage.ddl -I ../KAMASUTRA2G/rtl -o examples/k2g_xstage.v
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

module rdt_normalize (
    input  [31:0] v,
    input  [2:0]  t,
    output [31:0] o
);

  wire sgn = t[2];
  wire [31:0] as_byte = sgn ? {{24{v[7]}}, v[7:0]} : {24'd0, v[7:0]};
  wire [31:0] as_half = sgn ? {{16{v[15]}}, v[15:0]} : {16'd0, v[15:0]};
  wire [1:0] width = t[1:0];

  assign o = ((width == 2'd0) ? as_byte : ((width == 2'd1) ? as_half : v));

endmodule

module rdt_width_bytes (
    input  [2:0] t,
    output [2:0] w
);

  wire [1:0] low = t[1:0];

  assign w = ((low == 2'd0) ? 3'd1 : ((low == 2'd1) ? 3'd2 : 3'd4));

endmodule

// Built with:
//   mem_bytes : u32 = 32'h800000
//
module k2g_xstage (
    input          clk,
    input          rst_n,
    input  [1:0]   uops_wsalt,
    output [1:0]   uops_rsalt,
    input  [255:0] uops_data,
    output [1:0]   wb_wsalt,
    input  [1:0]   wb_rsalt,
    output [167:0] wb_data
);

  reg [83:0] w;
  reg [1:0] uops_rsalt_q;
  reg [83:0] wb_e0;
  reg [83:0] wb_e1;
  reg [1:0] wb_wsalt_q;

  reg [31:0] values [0:31];
  integer values_ix;
  reg [2:0] tags [0:31];
  integer tags_ix;
  reg overflow [0:31];
  integer overflow_ix;
  reg flagbit [0:31];
  integer flagbit_ix;

  wire uops_ridx = uops_rsalt_q[0] ^ uops_rsalt_q[1];
  wire [127:0] uops_item = uops_ridx ? uops_data[255:128] : uops_data[127:0];
  wire wb_full = wb_wsalt_q == (~wb_rsalt);
  wire wb_room = !wb_full;
  wire uops_empty = uops_wsalt == uops_rsalt_q;
  wire uops_xfer = !uops_empty;
  wire uops_empty_1 = uops_wsalt == uops_rsalt_q;
  wire uops_present = !uops_empty_1;
  wire [4:0] ra = uops_item[122:118];
  wire [4:0] rb = uops_item[117:113];
  wire [4:0] rc = uops_item[76:72];
  wire [31:0] ra_value = values[ra];
  wire [31:0] rb_value = values[rb];
  wire [31:0] rc_value = values[rc];
  wire [2:0] ra_tag = tags[ra];
  wire [2:0] rb_tag = tags[rb];
  wire [2:0] rc_tag = tags[rc];
  wire rb_ovf = overflow[rb];
  wire rc_ovf = overflow[rc];
  wire ra_flg = flagbit[ra];
  wire rb_flg = flagbit[rb];
  wire rc_flg = flagbit[rc];
  wire writing = ((w[83] | w[82]) | w[81]) | w[80];
  wire ma = writing & (w[79:75] == ra);
  wire mb = writing & (w[79:75] == rb);
  wire mc = writing & (w[79:75] == rc);
  wire [31:0] xa_value = (ma & w[83]) ? w[74:43] : ra_value;
  wire [31:0] xb_value = (mb & w[83]) ? w[74:43] : rb_value;
  wire [31:0] xc_value = (mc & w[83]) ? w[74:43] : rc_value;
  wire [2:0] xa_tag = (ma & w[82]) ? w[42:40] : ra_tag;
  wire [2:0] xb_tag = (mb & w[82]) ? w[42:40] : rb_tag;
  wire [2:0] xc_tag = (mc & w[82]) ? w[42:40] : rc_tag;
  wire xb_ovf = (mb & w[81]) ? w[39] : rb_ovf;
  wire xc_ovf = (mc & w[81]) ? w[39] : rc_ovf;
  wire xa_flg = (ma & w[80]) ? w[38] : ra_flg;
  wire xb_flg = (mb & w[80]) ? w[38] : rb_flg;
  wire xc_flg = (mc & w[80]) ? w[38] : rc_flg;
  wire [2:0] n129 = uops_item[79:77];
  reg n139;
  wire unpredicated = uops_item[79:77] == 3'd0;
  wire cond_met = unpredicated ? 1'b1 : (n139 ^ uops_item[71]);
  wire [4:0] n153 = uops_item[127:123];
  reg n168;
  reg n169;
  wire f32_a = n168 & (xa_tag == 3'd3);
  wire f32_b = n169 & (xb_tag == 3'd3);
  wire f32_cond = (!unpredicated) & (xc_tag == 3'd3);
  wire [31:0] alu_b = uops_item[80] ? uops_item[112:81] : xb_value;
  wire [2:0] alu_b_tag = uops_item[80] ? xa_tag : xb_tag;
  wire [1:0] n192 = uops_item[70:69];
  wire [1:0] n194 = uops_item[68:67];
  wire [2:0] n196 = uops_item[64:62];
  wire [1:0] n198 = uops_item[61:60];
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
  wire [31:0] n238 = uops_item[112:81];
  wire [4:0] amount = uops_item[80] ? n238[4:0] : xb_value[4:0];
  wire [4:0] n244 = uops_item[46:42];
  wire [4:0] n245 = uops_item[41:37];
  wire signed [31:0] n256 = xa_value;
  wire signed [31:0] n257 = n256 >>> amount;
  wire [31:0] n258 = n257;
  wire [31:0] shift_result = (uops_item[66:65] == 2'd0) ? (xa_value << amount) : ((uops_item[66:65] == 2'd1) ? (xa_value >> amount) : n258);
  wire [31:0] span_mask = (n245 == 5'd0) ? 32'd0 : ((32'd1 << n245) - 32'd1);
  wire [31:0] bext_result = (xb_value >> n244) & span_mask;
  wire [31:0] placed_mask = span_mask << n244;
  wire [31:0] placed_src = (xb_value & span_mask) << n244;
  wire [31:0] bins_result = (xa_value & (~placed_mask)) | placed_src;
  wire [31:0] load_addr = xb_value + uops_item[112:81];
  wire [31:0] store_addr = xa_value + uops_item[112:81];
  wire is_load = uops_item[127:123] == 5'd4;
  wire [31:0] access_addr = is_load ? load_addr : store_addr;
  wire [2:0] access_tag = is_load ? uops_item[49:47] : xb_tag;
  wire [1:0] n293 = uops_item[68:67];
  reg n297;
  wire [1:0] low = access_tag[1:0];
  wire [2:0] access_width = (low == 2'd0) ? 3'd1 : ((low == 2'd1) ? 3'd2 : 3'd4);
  wire is_store = uops_item[127:123] == 5'd5;
  wire is_access = is_load | is_store;
  wire [31:0] last_byte = access_addr + ({{29{1'b0}}, access_width});
  wire out_of_range = last_byte > 32'h800000;
  wire n333 = uops_present & f32_cond;
  wire n338 = uops_present & cond_met;
  wire n344 = uops_item[127:123] == 5'h12;
  wire n349 = f32_a | f32_b;
  wire n357 = is_access & ((access_width == 3'd1) ? 1'b0 : ((access_width == 3'd2) ? access_addr[0] : (access_addr[1] | access_addr[0])));
  wire n362 = is_access & out_of_range;
  wire n382 = n333 ? 1'b1 : (n338 ? (n344 ? 1'b1 : (n349 ? 1'b1 : (n357 ? 1'b1 : n362))) : 1'b0);
  wire commits = (uops_present & cond_met) & (!n382);
  wire [83:0] n = 84'd0;
  wire [83:0] n392 = {n[83:80], uops_item[122:118], n[74:0]};
  wire [83:0] n397 = {n392[83:43], uops_item[49:47], n392[39:0]};
  wire [83:0] n401 = {n397[83:40], 1'b0, n397[38:0]};
  wire [83:0] n405 = {n401[83:39], 1'b0, n401[37:0]};
  wire [83:0] n408 = {n405[83:38], n382, n405[36:0]};
  wire [83:0] n411 = {n408[83:37], (n333 ? 5'd5 : (n338 ? (n344 ? uops_item[36:32] : (n349 ? 5'd5 : (n357 ? (is_load ? 5'd6 : 5'd7) : (n362 ? 5'd9 : 5'd0)))) : 5'd0)), n408[31:0]};
  wire [83:0] n413 = {n411[83:32], (n333 ? ({{27{1'b0}}, uops_item[76:72]}) : (n338 ? (n344 ? uops_item[112:81] : (n349 ? (f32_a ? ({{27{1'b0}}, uops_item[122:118]}) : ({{27{1'b0}}, uops_item[117:113]})) : (n357 ? access_addr : (n362 ? access_addr : 32'd0)))) : 32'd0))};
  wire [4:0] n415 = uops_item[127:123];
  wire [83:0] n417 = {commits, n413[82:0]};
  wire [83:0] n420 = {n417[83], commits, n417[81:0]};
  wire [83:0] n423 = {n420[83:82], commits, n420[80:0]};
  wire [83:0] n426 = {n423[83:81], commits, n423[79:0]};
  wire [83:0] n436 = {commits, n413[82:0]};
  wire [83:0] n439 = {n436[83], commits, n436[81:0]};
  wire [83:0] n448 = {commits, n413[82:0]};
  wire [83:0] n451 = {n448[83], commits, n448[81:0]};
  wire [83:0] n454 = {n451[83:82], commits, n451[80:0]};
  wire [83:0] n457 = {n454[83:81], commits, n454[79:0]};
  wire [83:0] n460 = {n457[83:43], xb_tag, n457[39:0]};
  wire [83:0] n463 = {n460[83:40], xb_ovf, n460[38:0]};
  wire [83:0] n468 = {commits, n413[82:0]};
  wire [83:0] n471 = {n468[83:82], commits, n468[80:0]};
  wire n475 = uops_item[56];
  wire [83:0] n478 = {n413[83:81], commits, n413[79:0]};
  wire n486 = uops_item[56];
  wire [83:0] n489 = {n413[83:81], commits, n413[79:0]};
  wire [83:0] n501 = {n413[83:81], commits, n413[79:0]};
  wire [83:0] n508 = {commits, n413[82:0]};
  wire [83:0] n511 = {n508[83], commits, n508[81:0]};
  wire [83:0] n514 = {n511[83:82], commits, n511[80:0]};
  wire [83:0] n517 = {n514[83:81], commits, n514[79:0]};
  reg [83:0] n530;
  reg [2:0] n531;
  reg [31:0] n532;
  wire sgn = n531[2];
  wire [31:0] as_byte = sgn ? {{24{n532[7]}}, n532[7:0]} : {24'd0, n532[7:0]};
  wire [31:0] as_half = sgn ? {{16{n532[15]}}, n532[15:0]} : {16'd0, n532[15:0]};
  wire [1:0] width = n531[1:0];
  wire [83:0] n559 = {n530[83:75], ((width == 2'd0) ? as_byte : ((width == 2'd1) ? as_half : n532)), n530[42:0]};
  wire [83:0] n562 = {n559[83:75], access_addr, n559[42:0]};
  wire [83:0] n566 = is_access ? {n562[83:43], access_tag, n562[39:0]} : n559;
  wire n573 = w[82];
  wire sent = wb_room & uops_present;
  wire n602 = uops_present & sent;
  wire consumed = uops_xfer & n602;
  wire uops_take = uops_xfer & n602;
  wire wb_push = wb_room & uops_present;
  wire wb_widx = wb_wsalt_q[0] ^ wb_wsalt_q[1];

  always @* begin
    case (n129)
      3'd1: n139 = xc_ovf;
      3'd2: n139 = xc_flg;
      3'd3: n139 = (xc_value == 32'd0);
      3'd4: n139 = xc_value[31];
      3'd5: n139 = ((xc_value != 32'd0) & (!xc_value[31]));
      default: n139 = 1'b1;
    endcase
  end

  always @* begin
    case (n153)
      5'd3, 5'd4, 5'd6, 5'hB: n168 = 1'b0;
      5'd5, 5'd7, 5'hC: n168 = 1'b1;
      5'h10: n168 = 1'b1;
      5'd8, 5'd9, 5'hA, 5'hD: n168 = 1'b1;
      5'hE: n168 = (uops_item[59:58] != 2'd0);
      default: n168 = 1'b0;
    endcase
  end

  always @* begin
    case (n153)
      5'd3, 5'd4, 5'd6, 5'hB: n169 = 1'b1;
      5'd5, 5'd7, 5'hC: n169 = 1'b1;
      5'd8, 5'd9, 5'hA, 5'hD: n169 = (!uops_item[80]);
      default: n169 = 1'b0;
    endcase
  end

  always @* begin
    case (n192)
      2'd1: arith_ovf = diff[32];
      default: arith_ovf = sum[32];
    endcase
  end

  always @* begin
    case (n192)
      2'd1: arith_result = diff[31:0];
      default: arith_result = sum[31:0];
    endcase
  end

  always @* begin
    case (n194)
      2'd0: logic_result = (xa_value & alu_b);
      2'd1: logic_result = (xa_value | alu_b);
      default: logic_result = (xa_value ^ alu_b);
    endcase
  end

  always @* begin
    case (n198)
      2'd0: unary_result = (~xa_value);
      default: unary_result = ((~xa_value) + 32'd1);
    endcase
  end

  always @* begin
    case (n196)
      3'd0: cmp_result = (a_wide == b_wide);
      3'd1: cmp_result = (a_wide != b_wide);
      3'd2: cmp_result = (a_wide < b_wide);
      3'd3: cmp_result = (a_wide > b_wide);
      3'd4: cmp_result = (a_wide <= b_wide);
      default: cmp_result = (a_wide >= b_wide);
    endcase
  end

  always @* begin
    case (n293)
      2'd0: n297 = (xa_flg & xb_flg);
      2'd1: n297 = (xa_flg | xb_flg);
      default: n297 = (xa_flg ^ xb_flg);
    endcase
  end

  always @* begin
    case (n415)
      5'd1: n530 = {n426[83:43], uops_item[49:47], n426[39:0]};
      5'd2: n530 = {n439[83:43], uops_item[49:47], n439[39:0]};
      5'd3: n530 = {n463[83:39], xb_flg, n463[37:0]};
      5'd8: n530 = {n471[83:40], arith_ovf, n471[38:0]};
      5'd9: n530 = (n475 ? {n478[83:39], n297, n478[37:0]} : {commits, n413[82:0]});
      5'h10: n530 = (n486 ? {n489[83:39], (!xa_flg), n489[37:0]} : {commits, n413[82:0]});
      5'hD: n530 = {n501[83:39], cmp_result, n501[37:0]};
      5'hA: n530 = {commits, n413[82:0]};
      5'hB: n530 = {n517[83:43], 3'd2, n517[39:0]};
      5'hC: n530 = {commits, n413[82:0]};
      5'd4, 5'd5: n530 = {1'b0, n413[82:0]};
      default: n530 = {1'b0, n413[82:0]};
    endcase
  end

  always @* begin
    case (n415)
      5'd1: n531 = uops_item[49:47];
      5'd2: n531 = uops_item[49:47];
      5'h10: n531 = (n486 ? 3'd2 : xa_tag);
      5'hA: n531 = xa_tag;
      default: n531 = 3'd2;
    endcase
  end

  always @* begin
    case (n415)
      5'd1: n532 = uops_item[112:81];
      5'd2: n532 = xa_value;
      5'd3: n532 = xb_value;
      5'd8: n532 = arith_result;
      5'd9: n532 = (n475 ? 32'd0 : logic_result);
      5'h10: n532 = (n486 ? 32'd0 : unary_result);
      5'hA: n532 = shift_result;
      5'hB: n532 = bext_result;
      5'hC: n532 = bins_result;
      default: n532 = 32'd0;
    endcase
  end

  assign uops_rsalt = uops_rsalt_q;
  assign wb_wsalt = wb_wsalt_q;
  assign wb_data = {wb_e1, wb_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      w <= 84'd0;
      uops_rsalt_q <= 2'd0;
      wb_e0 <= 84'd0;
      wb_e1 <= 84'd0;
      wb_wsalt_q <= 2'd0;
    end else begin
      w <= (uops_present ? (sent ? n566 : 84'd0) : 84'd0);
      uops_rsalt_q <= (uops_take ? (uops_rsalt_q ^ (uops_ridx ? 2'd2 : 2'd1)) : uops_rsalt_q);
      wb_e0 <= ((wb_push & (!wb_widx)) ? n566 : wb_e0);
      wb_e1 <= ((wb_push & wb_widx) ? n566 : wb_e1);
      wb_wsalt_q <= (wb_push ? (wb_wsalt_q ^ (wb_widx ? 2'd2 : 2'd1)) : wb_wsalt_q);
    end
  end

  // values [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (values_ix = 0; values_ix < 32; values_ix = values_ix + 1) values[values_ix] <= 32'd0;
    end else if (w[83]) begin
      values[w[79:75]] <= w[74:43];
    end
  end

  // tags [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (tags_ix = 0; tags_ix < 32; tags_ix = tags_ix + 1) tags[tags_ix] <= 3'd2;
    end else if (n573) begin
      tags[w[79:75]] <= w[42:40];
    end
  end

  // overflow [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (overflow_ix = 0; overflow_ix < 32; overflow_ix = overflow_ix + 1) overflow[overflow_ix] <= 1'b0;
    end else if (w[81]) begin
      overflow[w[79:75]] <= w[39];
    end
  end

  // flagbit [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (flagbit_ix = 0; flagbit_ix < 32; flagbit_ix = flagbit_ix + 1) flagbit[flagbit_ix] <= 1'b0;
    end else if (w[80]) begin
      flagbit[w[79:75]] <= w[38];
    end
  end

`ifdef SIMULATION
  always @(posedge clk) begin
    if (rst_n) begin
      if (!(((!n573) | (w[42:40] != 3'd7)))) $error("%m: the reserved tag encoding was written");
      if (!(((!(uops_present & sent)) | consumed))) $error("%m: @assert failed");
    end
  end
`endif

endmodule
