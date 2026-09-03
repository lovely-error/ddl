// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/k2g_decode.ddl -I ../KAMASUTRA2G/rtl -o examples/k2g_decode.v
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

module k2g_decode (
    input          clk,
    input          rst_n,
    input  [1:0]   cps_wsalt,
    output [1:0]   cps_rsalt,
    input  [33:0]  cps_data,
    output [1:0]   uop_wsalt,
    input  [1:0]   uop_rsalt,
    output [255:0] uop_data
);

  reg [102:0] pfx;
  reg [1:0] state;
  reg [4:0] llc_dst;
  reg [2:0] llc_kind;
  reg [15:0] llc_hi;
  reg [1:0] cps_rsalt_q;
  reg [127:0] uop_e0;
  reg [127:0] uop_e1;
  reg [1:0] uop_wsalt_q;

  wire cps_ridx = cps_rsalt_q[0] ^ cps_rsalt_q[1];
  wire [16:0] cps_item = cps_ridx ? cps_data[33:17] : cps_data[16:0];
  wire uop_full = uop_wsalt_q == (~uop_rsalt);
  wire uop_room = !uop_full;
  wire cps_empty = cps_wsalt == cps_rsalt_q;
  wire cps_take = (!cps_empty) & uop_room;
  wire [15:0] cp = cps_item[16:1];
  wire flushing = cps_take & cps_item[0];
  wire [31:0] bytes_held = pfx[31:0];
  wire [5:0] lb = cp[15:10];
  wire [4:0] arg1 = cp[9:5];
  wire [4:0] arg2 = cp[4:0];
  wire [9:0] imm10 = cp[9:0];
  wire is_ep1 = lb == 6'h3F;
  wire is_ep2 = is_ep1 & (arg2 == 5'h1F);
  wire is_ep1_only = is_ep1 & (!is_ep2);
  wire pfx_xi = (lb == 6'h1E) | (lb == 6'h1D);
  wire pfx_bmx = lb == 6'h1C;
  wire pfx_xc = (lb == 6'h1F) | (lb == 6'h20);
  wire pfx_uto = is_ep1_only & (arg2 == 5'h1E);
  wire pfx_order = is_ep1_only & ((((arg2 == 5'h1D) | (arg2 == 5'h18)) | (arg2 == 5'h15)) | (arg2 == 5'h16));
  wire pfx_esp = is_ep1_only & (arg2 == 5'h17);
  wire pfx_flag = is_ep2 & (arg1 == 5'h1F);
  wire pfx_csp = is_ep2 & (arg1 == 5'h1E);
  wire pfx_mpi = is_ep2 & (arg1 == 5'h1B);
  wire pfx_mpd = is_ep2 & (arg1 == 5'h1A);
  wire pfx_istore = is_ep2 & (arg1 == 5'h17);
  wire is_prefix = (((((((((pfx_xi | pfx_bmx) | pfx_xc) | pfx_uto) | pfx_order) | pfx_esp) | pfx_flag) | pfx_csp) | pfx_mpi) | pfx_mpd) | pfx_istore;
  wire [4:0] cc = arg2;
  reg [2:0] n152;
  reg n153;
  wire [31:0] imm10_sext = {{22{imm10[9]}}, imm10};
  wire [31:0] imm10_zext = {22'd0, imm10};
  wire [31:0] xi_ext = (lb == 6'h1E) ? imm10_sext : imm10_zext;
  wire [31:0] n165 = pfx[100:69];
  wire [31:0] imm_alu = pfx[102] ? {n165[26:0], arg2} : {{27{arg2[4]}}, arg2};
  wire [31:0] imm_mem = pfx[102] ? pfx[100:69] : 32'd0;
  wire [31:0] n176 = pfx[100:69];
  wire [19:0] disp20 = {n176[9:0], imm10};
  wire [31:0] disp_from_xi = pfx[101] ? {12'd0, disp20} : {{12{disp20[19]}}, disp20};
  wire [31:0] disp_ext = pfx[102] ? disp_from_xi : imm10_sext;
  wire [31:0] disp_bytes = {disp_ext[30:0], 1'b0};
  wire [127:0] n194 = 128'd0;
  wire [127:0] n201 = {n194[127:80], pfx[68:66], n194[76:0]};
  wire [127:0] n205 = {n201[127:77], pfx[65:61], n201[71:0]};
  wire [127:0] n209 = {n205[127:72], pfx[60], n205[70:0]};
  wire [127:0] n213 = {n209[127:51], pfx[59], n209[49:0]};
  wire [127:0] n217 = {n213[127:56], pfx[58:54], n213[50:0]};
  wire [127:0] n221 = {n217[127:57], pfx[42], n217[55:0]};
  wire [127:0] n225 = {n221[127:47], pfx[52:48], n221[41:0]};
  wire [127:0] n229 = {n225[127:42], pfx[47:43], n225[36:0]};
  wire [127:0] n232 = {n229[127:123], arg1, n229[117:0]};
  wire [127:0] n235 = {n232[127:118], arg2, n232[112:0]};
  wire [127:0] n238 = {5'd1, n235[122:0]};
  wire [127:0] n241 = {n238[127:113], imm_alu, n238[80:0]};
  wire [127:0] f = 128'd0;
  wire [127:0] n269 = {5'h12, f[122:0]};
  wire [127:0] n272 = {n269[127:37], 5'd1, n269[31:0]};
  wire [127:0] n280 = {5'd2, n235[122:0]};
  wire [127:0] n311 = {n235[127:50], ((lb == 6'h3A) ? 3'd0 : ((lb == 6'h39) ? 3'd1 : ((lb == 6'h38) ? 3'd2 : 3'd3))), n235[46:0]};
  wire [127:0] f_1 = 128'd0;
  wire [127:0] n320 = {5'h12, f_1[122:0]};
  wire [127:0] n323 = {n320[127:37], 5'd5, n320[31:0]};
  wire [127:0] n335 = {5'd4, n311[122:0]};
  wire [127:0] n351 = {5'd5, n235[122:0]};
  wire [127:0] n354 = {n351[127:113], imm_mem, n351[80:0]};
  wire [127:0] f_2 = 128'd0;
  wire [127:0] n369 = {5'h12, f_2[122:0]};
  wire [127:0] n372 = {n369[127:37], 5'd4, n369[31:0]};
  wire [127:0] n380 = {5'd8, n235[122:0]};
  wire [127:0] n396 = {n380[127:71], ((lb == 6'h35) ? 2'd0 : ((lb == 6'h34) ? 2'd1 : 2'd2)), n380[68:0]};
  wire [127:0] n400 = {n396[127:81], pfx[102], n396[79:0]};
  wire [127:0] n407 = {5'd9, n235[122:0]};
  wire [127:0] n423 = {n407[127:69], ((lb == 6'h31) ? 2'd0 : ((lb == 6'h30) ? 2'd1 : 2'd2)), n407[66:0]};
  wire [127:0] n427 = {n423[127:81], pfx[102], n423[79:0]};
  wire [127:0] f_3 = 128'd0;
  wire [127:0] n438 = {5'h12, f_3[122:0]};
  wire [127:0] n441 = {n438[127:37], 5'd2, n438[31:0]};
  wire [127:0] f_4 = 128'd0;
  wire [127:0] n463 = {5'h12, f_4[122:0]};
  wire [127:0] n466 = {n463[127:37], 5'hB, n463[31:0]};
  wire [127:0] f_5 = 128'd0;
  wire [127:0] n484 = {5'h12, f_5[122:0]};
  wire [127:0] n487 = {n484[127:37], 5'd2, n484[31:0]};
  wire [127:0] n497 = {5'hA, n235[122:0]};
  wire [127:0] n517 = {5'hA, n235[122:0]};
  wire [127:0] n533 = {n517[127:67], ((lb == 6'h2B) ? 2'd0 : ((lb == 6'h2A) ? 2'd1 : 2'd2)), n517[64:0]};
  wire [127:0] n537 = {n533[127:81], 1'b1, n533[79:0]};
  wire [127:0] n551 = {5'hD, n235[122:0]};
  wire [127:0] n555 = {n551[127:81], pfx[102], n551[79:0]};
  wire [127:0] n558 = {n555[127:113], imm_alu, n555[80:0]};
  wire [127:0] n596 = {5'hE, n235[122:0]};
  wire [127:0] f_6 = 128'd0;
  wire [127:0] n615 = {5'h12, f_6[122:0]};
  wire [127:0] n618 = {n615[127:37], 5'd1, n615[31:0]};
  wire [127:0] n628 = {5'hE, n235[122:0]};
  wire [127:0] n632 = {n628[127:60], 2'd0, n628[57:0]};
  wire [127:0] f_7 = 128'd0;
  wire [127:0] n664 = {5'h12, f_7[122:0]};
  wire [127:0] n667 = {n664[127:37], 5'd2, n664[31:0]};
  wire [127:0] f_8 = 128'd0;
  wire [127:0] n681 = {5'h12, f_8[122:0]};
  wire [127:0] n684 = {n681[127:37], 5'd1, n681[31:0]};
  wire n701 = (arg2 == 5'h1C) | (arg2 == 5'h1B);
  wire [127:0] n704 = {5'h10, n235[122:0]};
  wire [127:0] f_9 = 128'd0;
  wire [127:0] n725 = {5'h12, f_9[122:0]};
  wire [127:0] n728 = {n725[127:37], 5'd2, n725[31:0]};
  wire n743 = (arg2 == 5'h14) | (arg2 == 5'h13);
  wire n755 = arg2 == 5'h12;
  wire [127:0] f_10 = 128'd0;
  wire [127:0] n766 = {5'h12, f_10[122:0]};
  wire [127:0] n769 = {n766[127:37], 5'd5, n766[31:0]};
  wire [127:0] f_11 = 128'd0;
  wire [127:0] n779 = {5'h12, f_11[122:0]};
  wire [127:0] n782 = {n779[127:37], 5'd1, n779[31:0]};
  wire [127:0] f_12 = 128'd0;
  wire [127:0] n804 = {5'h12, f_12[122:0]};
  wire [127:0] n807 = {n804[127:37], 5'd1, n804[31:0]};
  reg n813;
  reg [2:0] n814;
  reg [127:0] n815;
  wire [127:0] n818 = 128'd0;
  wire [102:0] n824 = {pfx[102:32], (pfx[31:0] + 32'd2)};
  wire n828 = state == 2'd0;
  wire n831 = n824[35:32] >= 4'd7;
  wire [127:0] f_13 = 128'd0;
  wire [127:0] n836 = {5'h12, f_13[122:0]};
  wire [127:0] n839 = {n836[127:37], 5'd3, n836[31:0]};
  wire [102:0] n851 = {n824[102:36], (n824[35:32] + 4'd1), n824[31:0]};
  wire [102:0] n854 = {1'b1, n851[101:0]};
  wire [102:0] n861 = {n854[102], (lb == 6'h1D), n854[100:0]};
  wire [102:0] n865 = pfx_xi ? {n861[102:101], xi_ext, n861[68:0]} : n851;
  wire [102:0] n869 = {n865[102:54], 1'b1, n865[52:0]};
  wire [102:0] n872 = {n869[102:53], arg1, n869[47:0]};
  wire [102:0] n876 = pfx_bmx ? {n872[102:48], arg2, n872[42:0]} : n865;
  wire [102:0] n879 = {n876[102:69], n152, n876[65:0]};
  wire [102:0] n882 = {n879[102:66], arg1, n879[60:0]};
  wire [127:0] f_14 = 128'd0;
  wire [127:0] n894 = {5'h12, f_14[122:0]};
  wire [127:0] n897 = {n894[127:37], 5'd2, n894[31:0]};
  wire [102:0] n909 = pfx_xc ? (n153 ? {n882[102:61], (lb == 6'h20), n882[59:0]} : n876) : n876;
  wire [102:0] n913 = {n909[102:60], 1'b1, n909[58:0]};
  wire [102:0] n917 = pfx_uto ? {n913[102:59], arg1, n913[53:0]} : n909;
  wire [102:0] n922 = pfx_flag ? {n917[102:43], 1'b1, n917[41:0]} : n917;
  wire [102:0] n927 = pfx_csp ? {n922[102:42], 1'b1, n922[40:0]} : n922;
  wire [102:0] n932 = pfx_istore ? {n927[102:41], 1'b1, n927[39:0]} : n927;
  wire [102:0] n937 = pfx_esp ? {n932[102:40], 1'b1, n932[38:0]} : n932;
  wire [102:0] n942 = pfx_mpd ? {n937[102:39], 1'b1, n937[37:0]} : n937;
  wire [102:0] n947 = pfx_mpi ? {n942[102:38], 1'b1, n942[36:0]} : n942;
  wire n978 = state == 2'd1;
  wire [127:0] n982 = {5'd1, n818[122:0]};
  wire [127:0] n985 = {n982[127:123], llc_dst, n982[117:0]};
  wire [127:0] n988 = {n985[127:50], llc_kind, n985[46:0]};
  wire [127:0] n999 = {n988[127:113], ((llc_kind == 3'd2) ? {llc_hi, cp} : {16'd0, cp}), n988[80:0]};
  wire [127:0] n1004 = {n999[127:80], n824[68:66], n999[76:0]};
  wire [127:0] n1008 = {n1004[127:77], n824[65:61], n1004[71:0]};
  wire n1026 = cps_take ? (n828 ? (is_prefix ? (n831 ? 1'b1 : (pfx_xc ? (n153 ? 1'b0 : 1'b1) : 1'b0)) : (n813 ? 1'b0 : 1'b1)) : (n978 ? 1'b0 : 1'b1)) : 1'b0;
  wire [127:0] n1030 = cps_take ? (n828 ? (is_prefix ? (n831 ? {n839[127:113], {16'd0, cp}, n839[80:0]} : (pfx_xc ? (n153 ? n818 : {n897[127:113], {16'd0, cp}, n897[80:0]}) : n818)) : (n813 ? n818 : n815)) : (n978 ? n818 : {n1008[127:72], n824[60], n1008[70:0]})) : n818;
  wire [127:0] n1036 = {n1030[127:32], (bytes_held + 32'd2)};
  wire n1042 = !cps_take;
  wire uop_push = n1026 & uop_room;
  wire uop_widx = uop_wsalt_q[0] ^ uop_wsalt_q[1];

  always @* begin
    case (cc)
      5'h1F: n152 = 3'd1;
      5'h1E: n152 = 3'd2;
      5'h1D: n152 = 3'd3;
      5'h1C: n152 = 3'd4;
      5'h1B: n152 = 3'd5;
      default: n152 = 3'd0;
    endcase
  end

  always @* begin
    case (cc)
      5'h1F: n153 = 1'b1;
      5'h1E: n153 = 1'b1;
      5'h1D: n153 = 1'b1;
      5'h1C: n153 = 1'b1;
      5'h1B: n153 = 1'b1;
      default: n153 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n813 = (is_ep2 ? 1'b0 : (n701 ? 1'b0 : (n743 ? 1'b1 : n755)));
      default: n813 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n814 = (is_ep2 ? 3'd2 : (n701 ? 3'd2 : (n743 ? ((arg2 == 5'h14) ? 3'd0 : 3'd1) : (n755 ? 3'd2 : 3'd2))));
      default: n814 = 3'd2;
    endcase
  end

  always @* begin
    case (lb)
      6'h3E, 6'h3D, 6'h3C: n815 = {n241[127:50], ((lb == 6'h3E) ? 3'd0 : ((lb == 6'h3D) ? 3'd1 : 3'd2)), n241[46:0]};
      6'h1B: n815 = (((arg2[2:0] == 3'd7) | (arg2[4:3] != 2'd0)) ? {n272[127:113], {16'd0, cp}, n272[80:0]} : {n280[127:50], arg2[2:0], n280[46:0]});
      6'h3B: n815 = {5'd3, n235[122:0]};
      6'h3A, 6'h39, 6'h38, 6'h37: n815 = ((lb == 6'h37) ? {n323[127:113], {16'd0, cp}, n323[80:0]} : (pfx[41] ? {5'd6, n311[122:0]} : {n335[127:113], imm_mem, n335[80:0]}));
      6'h36: n815 = (pfx[39] ? {5'd0, n235[122:0]} : (pfx[41] ? {5'd7, n235[122:0]} : {n354[127:58], pfx[40], n354[56:0]}));
      6'h35, 6'h34, 6'h33, 6'h32: n815 = ((lb == 6'h32) ? {n372[127:113], {16'd0, cp}, n372[80:0]} : {n400[127:113], imm_alu, n400[80:0]});
      6'h31, 6'h30, 6'h2F: n815 = ((pfx[42] & pfx[102]) ? {n441[127:113], {16'd0, cp}, n441[80:0]} : {n427[127:113], imm_alu, n427[80:0]});
      6'h2E, 6'h2D, 6'h2C: n815 = (pfx[53] ? ((lb == 6'h2E) ? ((pfx[47:43] == 5'd0) ? {n466[127:113], {16'd0, cp}, n466[80:0]} : {5'hB, n235[122:0]}) : ((lb == 6'h2D) ? {5'hC, n235[122:0]} : {n487[127:113], {16'd0, cp}, n487[80:0]})) : {n497[127:67], ((lb == 6'h2E) ? 2'd0 : ((lb == 6'h2D) ? 2'd1 : 2'd2)), n497[64:0]});
      6'h2B, 6'h2A, 6'h29: n815 = {n537[127:113], {27'd0, arg2}, n537[80:0]};
      6'h28, 6'h27, 6'h26, 6'h25, 6'h24, 6'h23: n815 = ((pfx[38] | pfx[37]) ? {5'd0, n235[122:0]} : {n558[127:65], ((lb == 6'h28) ? 3'd0 : ((lb == 6'h27) ? 3'd1 : ((lb == 6'h26) ? 3'd2 : ((lb == 6'h25) ? 3'd3 : ((lb == 6'h24) ? 3'd4 : 3'd5))))), n558[61:0]});
      6'h22: n815 = ((arg2 == 5'h1A) ? {n596[127:60], 2'd1, n596[57:0]} : ((arg2 == 5'h19) ? {n596[127:60], 2'd2, n596[57:0]} : {n618[127:113], {16'd0, cp}, n618[80:0]}));
      6'h21: n815 = {n632[127:113], disp_bytes, n632[80:0]};
      6'h3F: n815 = (is_ep2 ? ((arg1 == 5'h19) ? {5'h11, n235[122:0]} : ((arg1 == 5'h1C) ? {5'd0, n235[122:0]} : ((arg1 == 5'h1D) ? ((pfx[68:66] != 3'd0) ? {n667[127:113], {16'd0, cp}, n667[80:0]} : {5'hF, n235[122:0]}) : {n684[127:113], {16'd0, cp}, n684[80:0]}))) : (n701 ? ((pfx[42] & (arg2 == 5'h1B)) ? {n728[127:113], {16'd0, cp}, n728[80:0]} : {n704[127:62], ((arg2 == 5'h1C) ? 2'd0 : 2'd1), n704[59:0]}) : (n743 ? n235 : (n755 ? n235 : ((arg2 == 5'h11) ? {n769[127:113], {16'd0, cp}, n769[80:0]} : {n782[127:113], {16'd0, cp}, n782[80:0]})))));
      default: n815 = {n807[127:113], {16'd0, cp}, n807[80:0]};
    endcase
  end

  assign cps_rsalt = cps_rsalt_q;
  assign uop_wsalt = uop_wsalt_q;
  assign uop_data = {uop_e1, uop_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      pfx <= 103'd0;
      state <= 2'd0;
      llc_dst <= 5'd0;
      llc_kind <= 3'd2;
      llc_hi <= 16'd0;
      cps_rsalt_q <= 2'd0;
      uop_e0 <= 128'd0;
      uop_e1 <= 128'd0;
      uop_wsalt_q <= 2'd0;
    end else begin
      pfx <= (flushing ? 103'd0 : (n1042 ? pfx : (n1026 ? 103'd0 : (cps_take ? (n828 ? (is_prefix ? (n831 ? n824 : (pfx_order ? {n947[102:37], 1'b1, n947[35:0]} : n947)) : n824) : n824) : pfx))));
      state <= (flushing ? 2'd0 : (n1042 ? state : (n1026 ? 2'd0 : (cps_take ? (n828 ? (is_prefix ? state : (n813 ? ((n814 == 3'd2) ? 2'd1 : 2'd2) : state)) : (n978 ? 2'd2 : 2'd0)) : state))));
      llc_dst <= (flushing ? 5'd0 : (n1042 ? llc_dst : (cps_take ? (n828 ? (is_prefix ? llc_dst : (n813 ? arg1 : llc_dst)) : llc_dst) : llc_dst)));
      llc_kind <= (flushing ? 3'd2 : (n1042 ? llc_kind : (cps_take ? (n828 ? (is_prefix ? llc_kind : (n813 ? n814 : llc_kind)) : llc_kind) : llc_kind)));
      llc_hi <= (flushing ? 16'd0 : (n1042 ? llc_hi : (cps_take ? (n828 ? llc_hi : (n978 ? cp : llc_hi)) : llc_hi)));
      cps_rsalt_q <= (cps_take ? (cps_rsalt_q ^ (cps_ridx ? 2'd2 : 2'd1)) : cps_rsalt_q);
      uop_e0 <= ((uop_push & (!uop_widx)) ? n1036 : uop_e0);
      uop_e1 <= ((uop_push & uop_widx) ? n1036 : uop_e1);
      uop_wsalt_q <= (uop_push ? (uop_wsalt_q ^ (uop_widx ? 2'd2 : 2'd1)) : uop_wsalt_q);
    end
  end

endmodule
