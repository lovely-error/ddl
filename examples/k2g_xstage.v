// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/k2g_xstage.ddl -I ../KAMASUTRA2G/rtl -o examples/k2g_xstage.v
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
//   mem_bytes : i32 = 32'h800000
//
module k2g_xstage (
    input          clk,
    input          rst_n,
    input          uops_valid,
    output         uops_ready,
    input  [126:0] uops_data,
    output         wb_valid,
    input          wb_ready,
    output [83:0]  wb_data
);

  reg [83:0] w;
  reg wb_busy;
  reg [83:0] wb_hold;
  reg wb_skid_busy;
  reg [83:0] wb_skid;

  reg [31:0] values [0:31];
  integer values_ix;
  reg [2:0] tags [0:31];
  integer tags_ix;
  reg overflow [0:31];
  integer overflow_ix;
  reg flagbit [0:31];
  integer flagbit_ix;

  wire wb_room = !wb_skid_busy;
  wire uops_xfer = uops_valid & wb_room;
  wire [4:0] ra = uops_data[121:117];
  wire [4:0] rb = uops_data[116:112];
  wire [4:0] rc = uops_data[75:71];
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
  wire [2:0] n115 = uops_data[78:76];
  reg n125;
  wire unpredicated = uops_data[78:76] == 3'd0;
  wire cond_met = unpredicated ? 1'b1 : (n125 ^ uops_data[70]);
  wire [4:0] n139 = uops_data[126:122];
  reg n154;
  reg n155;
  wire f32_a = n154 & (xa_tag == 3'd3);
  wire f32_b = n155 & (xb_tag == 3'd3);
  wire f32_cond = (!unpredicated) & (xc_tag == 3'd3);
  wire [31:0] alu_b = uops_data[79] ? uops_data[111:80] : xb_value;
  wire [2:0] alu_b_tag = uops_data[79] ? xa_tag : xb_tag;
  wire [1:0] n178 = uops_data[69:68];
  wire [1:0] n180 = uops_data[67:66];
  wire [2:0] n182 = uops_data[63:61];
  wire [1:0] n184 = uops_data[60:59];
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
  wire [31:0] n224 = uops_data[111:80];
  wire [4:0] amount = uops_data[79] ? n224[4:0] : xb_value[4:0];
  wire [4:0] n230 = uops_data[46:42];
  wire [4:0] n231 = uops_data[41:37];
  wire signed [31:0] n242 = xa_value;
  wire signed [31:0] n243 = n242 >>> amount;
  wire [31:0] n244 = n243;
  wire [31:0] shift_result = (uops_data[65:64] == 2'd0) ? (xa_value << amount) : ((uops_data[65:64] == 2'd1) ? (xa_value >> amount) : n244);
  wire [31:0] span_mask = (n231 == 5'd0) ? 32'd0 : ((32'd1 << n231) - 32'd1);
  wire [31:0] bext_result = (xb_value >> n230) & span_mask;
  wire [31:0] placed_mask = span_mask << n230;
  wire [31:0] placed_src = (xb_value & span_mask) << n230;
  wire [31:0] bins_result = (xa_value & (~placed_mask)) | placed_src;
  wire [31:0] load_addr = xb_value + uops_data[111:80];
  wire [31:0] store_addr = xa_value + uops_data[111:80];
  wire is_load = uops_data[126:122] == 5'd4;
  wire [31:0] access_addr = is_load ? load_addr : store_addr;
  wire [2:0] access_tag = is_load ? uops_data[49:47] : xb_tag;
  wire [1:0] n279 = uops_data[67:66];
  reg n283;
  wire [1:0] low = access_tag[1:0];
  wire [2:0] access_width = (low == 2'd0) ? 3'd1 : ((low == 2'd1) ? 3'd2 : 3'd4);
  wire is_store = uops_data[126:122] == 5'd5;
  wire is_access = is_load | is_store;
  wire [31:0] last_byte = access_addr + ({{29{1'b0}}, access_width});
  wire out_of_range = last_byte > 32'h800000;
  wire n319 = uops_xfer & f32_cond;
  wire n324 = uops_xfer & cond_met;
  wire n330 = uops_data[126:122] == 5'h12;
  wire n335 = f32_a | f32_b;
  wire n343 = is_access & ((access_width == 3'd1) ? 1'b0 : ((access_width == 3'd2) ? access_addr[0] : (access_addr[1] | access_addr[0])));
  wire n348 = is_access & out_of_range;
  wire n368 = n319 ? 1'b1 : (n324 ? (n330 ? 1'b1 : (n335 ? 1'b1 : (n343 ? 1'b1 : n348))) : 1'b0);
  wire commits = (uops_xfer & cond_met) & (!n368);
  wire [83:0] n = 84'd0;
  wire [83:0] n378 = {n[83:80], uops_data[121:117], n[74:0]};
  wire [83:0] n383 = {n378[83:43], uops_data[49:47], n378[39:0]};
  wire [83:0] n387 = {n383[83:40], 1'b0, n383[38:0]};
  wire [83:0] n391 = {n387[83:39], 1'b0, n387[37:0]};
  wire [83:0] n394 = {n391[83:38], n368, n391[36:0]};
  wire [83:0] n397 = {n394[83:37], (n319 ? 5'd5 : (n324 ? (n330 ? uops_data[36:32] : (n335 ? 5'd5 : (n343 ? (is_load ? 5'd6 : 5'd7) : (n348 ? 5'd9 : 5'd0)))) : 5'd0)), n394[31:0]};
  wire [83:0] n399 = {n397[83:32], (n319 ? ({{27{1'b0}}, uops_data[75:71]}) : (n324 ? (n330 ? uops_data[111:80] : (n335 ? (f32_a ? ({{27{1'b0}}, uops_data[121:117]}) : ({{27{1'b0}}, uops_data[116:112]})) : (n343 ? access_addr : (n348 ? access_addr : 32'd0)))) : 32'd0))};
  wire [4:0] n401 = uops_data[126:122];
  wire [83:0] n403 = {commits, n399[82:0]};
  wire [83:0] n406 = {n403[83], commits, n403[81:0]};
  wire [83:0] n409 = {n406[83:82], commits, n406[80:0]};
  wire [83:0] n412 = {n409[83:81], commits, n409[79:0]};
  wire [83:0] n422 = {commits, n399[82:0]};
  wire [83:0] n425 = {n422[83], commits, n422[81:0]};
  wire [83:0] n434 = {commits, n399[82:0]};
  wire [83:0] n437 = {n434[83], commits, n434[81:0]};
  wire [83:0] n440 = {n437[83:82], commits, n437[80:0]};
  wire [83:0] n443 = {n440[83:81], commits, n440[79:0]};
  wire [83:0] n446 = {n443[83:43], xb_tag, n443[39:0]};
  wire [83:0] n449 = {n446[83:40], xb_ovf, n446[38:0]};
  wire [83:0] n454 = {commits, n399[82:0]};
  wire [83:0] n457 = {n454[83:82], commits, n454[80:0]};
  wire n461 = uops_data[56];
  wire [83:0] n464 = {n399[83:81], commits, n399[79:0]};
  wire n472 = uops_data[56];
  wire [83:0] n475 = {n399[83:81], commits, n399[79:0]};
  wire [83:0] n487 = {n399[83:81], commits, n399[79:0]};
  wire [83:0] n494 = {commits, n399[82:0]};
  wire [83:0] n497 = {n494[83], commits, n494[81:0]};
  wire [83:0] n500 = {n497[83:82], commits, n497[80:0]};
  wire [83:0] n503 = {n500[83:81], commits, n500[79:0]};
  reg [83:0] n516;
  reg [2:0] n517;
  reg [31:0] n518;
  wire sgn = n517[2];
  wire [31:0] as_byte = sgn ? {{24{n518[7]}}, n518[7:0]} : {24'd0, n518[7:0]};
  wire [31:0] as_half = sgn ? {{16{n518[15]}}, n518[15:0]} : {16'd0, n518[15:0]};
  wire [1:0] width = n517[1:0];
  wire [83:0] n545 = {n516[83:75], ((width == 2'd0) ? as_byte : ((width == 2'd1) ? as_half : n518)), n516[42:0]};
  wire [83:0] n548 = {n545[83:75], access_addr, n545[42:0]};
  wire [83:0] n552 = is_access ? {n548[83:43], access_tag, n548[39:0]} : n545;
  wire n559 = w[82];
  wire wb_pop = wb_busy & wb_ready;
  wire n592 = !wb_pop;
  wire n595 = uops_xfer & ((!wb_busy) | wb_pop);
  wire n596 = wb_pop & wb_skid_busy;
  wire n603 = uops_xfer & (wb_busy & n592);

  always @* begin
    case (n115)
      3'd1: n125 = xc_ovf;
      3'd2: n125 = xc_flg;
      3'd3: n125 = (xc_value == 32'd0);
      3'd4: n125 = xc_value[31];
      3'd5: n125 = ((xc_value != 32'd0) & (!xc_value[31]));
      default: n125 = 1'b1;
    endcase
  end

  always @* begin
    case (n139)
      5'd3, 5'd4, 5'd6, 5'hB: n154 = 1'b0;
      5'd5, 5'd7, 5'hC: n154 = 1'b1;
      5'h10: n154 = 1'b1;
      5'd8, 5'd9, 5'hA, 5'hD: n154 = 1'b1;
      5'hE: n154 = (uops_data[58:57] != 2'd0);
      default: n154 = 1'b0;
    endcase
  end

  always @* begin
    case (n139)
      5'd3, 5'd4, 5'd6, 5'hB: n155 = 1'b1;
      5'd5, 5'd7, 5'hC: n155 = 1'b1;
      5'd8, 5'd9, 5'hA, 5'hD: n155 = (!uops_data[79]);
      default: n155 = 1'b0;
    endcase
  end

  always @* begin
    case (n178)
      2'd1: arith_ovf = diff[32];
      default: arith_ovf = sum[32];
    endcase
  end

  always @* begin
    case (n178)
      2'd1: arith_result = diff[31:0];
      default: arith_result = sum[31:0];
    endcase
  end

  always @* begin
    case (n180)
      2'd0: logic_result = (xa_value & alu_b);
      2'd1: logic_result = (xa_value | alu_b);
      default: logic_result = (xa_value ^ alu_b);
    endcase
  end

  always @* begin
    case (n184)
      2'd0: unary_result = (~xa_value);
      default: unary_result = ((~xa_value) + 32'd1);
    endcase
  end

  always @* begin
    case (n182)
      3'd0: cmp_result = (a_wide == b_wide);
      3'd1: cmp_result = (a_wide != b_wide);
      3'd2: cmp_result = (a_wide < b_wide);
      3'd3: cmp_result = (a_wide > b_wide);
      3'd4: cmp_result = (a_wide <= b_wide);
      default: cmp_result = (a_wide >= b_wide);
    endcase
  end

  always @* begin
    case (n279)
      2'd0: n283 = (xa_flg & xb_flg);
      2'd1: n283 = (xa_flg | xb_flg);
      default: n283 = (xa_flg ^ xb_flg);
    endcase
  end

  always @* begin
    case (n401)
      5'd1: n516 = {n412[83:43], uops_data[49:47], n412[39:0]};
      5'd2: n516 = {n425[83:43], uops_data[49:47], n425[39:0]};
      5'd3: n516 = {n449[83:39], xb_flg, n449[37:0]};
      5'd8: n516 = {n457[83:40], arith_ovf, n457[38:0]};
      5'd9: n516 = (n461 ? {n464[83:39], n283, n464[37:0]} : {commits, n399[82:0]});
      5'h10: n516 = (n472 ? {n475[83:39], (!xa_flg), n475[37:0]} : {commits, n399[82:0]});
      5'hD: n516 = {n487[83:39], cmp_result, n487[37:0]};
      5'hA: n516 = {commits, n399[82:0]};
      5'hB: n516 = {n503[83:43], 3'd2, n503[39:0]};
      5'hC: n516 = {commits, n399[82:0]};
      5'd4, 5'd5: n516 = {1'b0, n399[82:0]};
      default: n516 = {1'b0, n399[82:0]};
    endcase
  end

  always @* begin
    case (n401)
      5'd1: n517 = uops_data[49:47];
      5'd2: n517 = uops_data[49:47];
      5'h10: n517 = (n472 ? 3'd2 : xa_tag);
      5'hA: n517 = xa_tag;
      default: n517 = 3'd2;
    endcase
  end

  always @* begin
    case (n401)
      5'd1: n518 = uops_data[111:80];
      5'd2: n518 = xa_value;
      5'd3: n518 = xb_value;
      5'd8: n518 = arith_result;
      5'd9: n518 = (n461 ? 32'd0 : logic_result);
      5'h10: n518 = (n472 ? 32'd0 : unary_result);
      5'hA: n518 = shift_result;
      5'hB: n518 = bext_result;
      5'hC: n518 = bins_result;
      default: n518 = 32'd0;
    endcase
  end

  assign uops_ready = wb_room;
  assign wb_valid = wb_busy;
  assign wb_data = wb_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      w <= 84'd0;
      wb_busy <= 1'b0;
      wb_hold <= 84'd0;
      wb_skid_busy <= 1'b0;
      wb_skid <= 84'd0;
    end else begin
      w <= n552;
      wb_busy <= (((wb_busy & n592) | n596) | n595);
      wb_hold <= (n596 ? wb_skid : (n595 ? n552 : wb_hold));
      wb_skid_busy <= ((wb_skid_busy & n592) | n603);
      wb_skid <= (n603 ? n552 : wb_skid);
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
    end else if (n559) begin
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
      if (!(((!n559) | (w[42:40] != 3'd7)))) $error("%m: the reserved tag encoding was written");
    end
  end
`endif

endmodule
