// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/k2g_decode.ddl -I ../KAMASUTRA2G/rtl -o examples/k2g_decode.v
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

module k2g_decode (
    input          clk,
    input          rst_n,
    input  [1:0]   cps_wsalt,
    output [1:0]   cps_rsalt,
    input  [33:0]  cps_data,
    output [1:0]   uop_wsalt,
    input  [1:0]   uop_rsalt,
    output [253:0] uop_data
);

  reg [102:0] pfx;
  reg [1:0] state;
  reg [4:0] llc_dst;
  reg [2:0] llc_kind;
  reg [15:0] llc_hi;
  reg [1:0] cps_rsalt_q;
  reg [126:0] uop_e0;
  reg [126:0] uop_e1;
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
  wire pfx_icinvr = is_ep2 & (arg1 == 5'h18);
  wire is_prefix = (((((((((pfx_xi | pfx_bmx) | pfx_xc) | pfx_uto) | pfx_order) | pfx_esp) | pfx_flag) | pfx_csp) | pfx_mpi) | pfx_mpd) | pfx_icinvr;
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
  wire [126:0] n194 = 127'd0;
  wire [126:0] n201 = {n194[126:79], pfx[68:66], n194[75:0]};
  wire [126:0] n205 = {n201[126:76], pfx[65:61], n201[70:0]};
  wire [126:0] n209 = {n205[126:71], pfx[60], n205[69:0]};
  wire [126:0] n213 = {n209[126:51], pfx[59], n209[49:0]};
  wire [126:0] n217 = {n213[126:56], pfx[58:54], n213[50:0]};
  wire [126:0] n221 = {n217[126:57], pfx[42], n217[55:0]};
  wire [126:0] n225 = {n221[126:47], pfx[52:48], n221[41:0]};
  wire [126:0] n229 = {n225[126:42], pfx[47:43], n225[36:0]};
  wire [126:0] n232 = {n229[126:122], arg1, n229[116:0]};
  wire [126:0] n235 = {n232[126:117], arg2, n232[111:0]};
  wire [126:0] n238 = {5'd1, n235[121:0]};
  wire [126:0] n241 = {n238[126:112], imm_alu, n238[79:0]};
  wire [126:0] f = 127'd0;
  wire [126:0] n269 = {5'h12, f[121:0]};
  wire [126:0] n272 = {n269[126:37], 5'd1, n269[31:0]};
  wire [126:0] n280 = {5'd2, n235[121:0]};
  wire [126:0] n311 = {n235[126:50], ((lb == 6'h3A) ? 3'd0 : ((lb == 6'h39) ? 3'd1 : ((lb == 6'h38) ? 3'd2 : 3'd3))), n235[46:0]};
  wire [126:0] f_1 = 127'd0;
  wire [126:0] n320 = {5'h12, f_1[121:0]};
  wire [126:0] n323 = {n320[126:37], 5'd5, n320[31:0]};
  wire [126:0] n335 = {5'd4, n311[121:0]};
  wire [126:0] n355 = {5'd5, n235[121:0]};
  wire [126:0] f_2 = 127'd0;
  wire [126:0] n370 = {5'h12, f_2[121:0]};
  wire [126:0] n373 = {n370[126:37], 5'd4, n370[31:0]};
  wire [126:0] n381 = {5'd8, n235[121:0]};
  wire [126:0] n397 = {n381[126:70], ((lb == 6'h35) ? 2'd0 : ((lb == 6'h34) ? 2'd1 : 2'd2)), n381[67:0]};
  wire [126:0] n401 = {n397[126:80], pfx[102], n397[78:0]};
  wire [126:0] n408 = {5'd9, n235[121:0]};
  wire [126:0] n424 = {n408[126:68], ((lb == 6'h31) ? 2'd0 : ((lb == 6'h30) ? 2'd1 : 2'd2)), n408[65:0]};
  wire [126:0] n428 = {n424[126:80], pfx[102], n424[78:0]};
  wire [126:0] f_3 = 127'd0;
  wire [126:0] n439 = {5'h12, f_3[121:0]};
  wire [126:0] n442 = {n439[126:37], 5'd2, n439[31:0]};
  wire [126:0] f_4 = 127'd0;
  wire [126:0] n464 = {5'h12, f_4[121:0]};
  wire [126:0] n467 = {n464[126:37], 5'hB, n464[31:0]};
  wire [126:0] f_5 = 127'd0;
  wire [126:0] n485 = {5'h12, f_5[121:0]};
  wire [126:0] n488 = {n485[126:37], 5'd2, n485[31:0]};
  wire [126:0] n498 = {5'hA, n235[121:0]};
  wire [126:0] n518 = {5'hA, n235[121:0]};
  wire [126:0] n534 = {n518[126:66], ((lb == 6'h2B) ? 2'd0 : ((lb == 6'h2A) ? 2'd1 : 2'd2)), n518[63:0]};
  wire [126:0] n538 = {n534[126:80], 1'b1, n534[78:0]};
  wire [126:0] n552 = {5'hD, n235[121:0]};
  wire [126:0] n556 = {n552[126:80], pfx[102], n552[78:0]};
  wire [126:0] n559 = {n556[126:112], imm_alu, n556[79:0]};
  wire [126:0] n597 = {5'hE, n235[121:0]};
  wire [126:0] f_6 = 127'd0;
  wire [126:0] n616 = {5'h12, f_6[121:0]};
  wire [126:0] n619 = {n616[126:37], 5'd1, n616[31:0]};
  wire [126:0] n629 = {5'hE, n235[121:0]};
  wire [126:0] n633 = {n629[126:59], 2'd0, n629[56:0]};
  wire [126:0] f_7 = 127'd0;
  wire [126:0] n662 = {5'h12, f_7[121:0]};
  wire [126:0] n665 = {n662[126:37], 5'd1, n662[31:0]};
  wire n682 = (arg2 == 5'h1C) | (arg2 == 5'h1B);
  wire [126:0] n685 = {5'h10, n235[121:0]};
  wire [126:0] f_8 = 127'd0;
  wire [126:0] n706 = {5'h12, f_8[121:0]};
  wire [126:0] n709 = {n706[126:37], 5'd2, n706[31:0]};
  wire n724 = (arg2 == 5'h14) | (arg2 == 5'h13);
  wire n736 = arg2 == 5'h12;
  wire [126:0] f_9 = 127'd0;
  wire [126:0] n747 = {5'h12, f_9[121:0]};
  wire [126:0] n750 = {n747[126:37], 5'd5, n747[31:0]};
  wire [126:0] f_10 = 127'd0;
  wire [126:0] n760 = {5'h12, f_10[121:0]};
  wire [126:0] n763 = {n760[126:37], 5'd1, n760[31:0]};
  wire [126:0] f_11 = 127'd0;
  wire [126:0] n785 = {5'h12, f_11[121:0]};
  wire [126:0] n788 = {n785[126:37], 5'd1, n785[31:0]};
  reg n794;
  reg [2:0] n795;
  reg [126:0] n796;
  wire [126:0] n799 = 127'd0;
  wire [102:0] n805 = {pfx[102:32], (pfx[31:0] + 32'd2)};
  wire n809 = state == 2'd0;
  wire n812 = n805[35:32] >= 4'd7;
  wire [126:0] f_12 = 127'd0;
  wire [126:0] n817 = {5'h12, f_12[121:0]};
  wire [126:0] n820 = {n817[126:37], 5'd3, n817[31:0]};
  wire [102:0] n832 = {n805[102:36], (n805[35:32] + 4'd1), n805[31:0]};
  wire [102:0] n835 = {1'b1, n832[101:0]};
  wire [102:0] n842 = {n835[102], (lb == 6'h1D), n835[100:0]};
  wire [102:0] n846 = pfx_xi ? {n842[102:101], xi_ext, n842[68:0]} : n832;
  wire [102:0] n850 = {n846[102:54], 1'b1, n846[52:0]};
  wire [102:0] n853 = {n850[102:53], arg1, n850[47:0]};
  wire [102:0] n857 = pfx_bmx ? {n853[102:48], arg2, n853[42:0]} : n846;
  wire [102:0] n860 = {n857[102:69], n152, n857[65:0]};
  wire [102:0] n863 = {n860[102:66], arg1, n860[60:0]};
  wire [126:0] f_13 = 127'd0;
  wire [126:0] n875 = {5'h12, f_13[121:0]};
  wire [126:0] n878 = {n875[126:37], 5'd2, n875[31:0]};
  wire [102:0] n890 = pfx_xc ? (n153 ? {n863[102:61], (lb == 6'h20), n863[59:0]} : n857) : n857;
  wire [102:0] n894 = {n890[102:60], 1'b1, n890[58:0]};
  wire [102:0] n898 = pfx_uto ? {n894[102:59], arg1, n894[53:0]} : n890;
  wire [102:0] n903 = pfx_flag ? {n898[102:43], 1'b1, n898[41:0]} : n898;
  wire [102:0] n908 = pfx_csp ? {n903[102:42], 1'b1, n903[40:0]} : n903;
  wire [102:0] n913 = pfx_icinvr ? {n908[102:41], 1'b1, n908[39:0]} : n908;
  wire [102:0] n918 = pfx_esp ? {n913[102:40], 1'b1, n913[38:0]} : n913;
  wire [102:0] n923 = pfx_mpd ? {n918[102:39], 1'b1, n918[37:0]} : n918;
  wire [102:0] n928 = pfx_mpi ? {n923[102:38], 1'b1, n923[36:0]} : n923;
  wire n959 = state == 2'd1;
  wire [126:0] n963 = {5'd1, n799[121:0]};
  wire [126:0] n966 = {n963[126:122], llc_dst, n963[116:0]};
  wire [126:0] n969 = {n966[126:50], llc_kind, n966[46:0]};
  wire [126:0] n980 = {n969[126:112], ((llc_kind == 3'd2) ? {llc_hi, cp} : {16'd0, cp}), n969[79:0]};
  wire [126:0] n985 = {n980[126:79], n805[68:66], n980[75:0]};
  wire [126:0] n989 = {n985[126:76], n805[65:61], n985[70:0]};
  wire n1007 = cps_take ? (n809 ? (is_prefix ? (n812 ? 1'b1 : (pfx_xc ? (n153 ? 1'b0 : 1'b1) : 1'b0)) : (n794 ? 1'b0 : 1'b1)) : (n959 ? 1'b0 : 1'b1)) : 1'b0;
  wire [126:0] n1011 = cps_take ? (n809 ? (is_prefix ? (n812 ? {n820[126:112], {16'd0, cp}, n820[79:0]} : (pfx_xc ? (n153 ? n799 : {n878[126:112], {16'd0, cp}, n878[79:0]}) : n799)) : (n794 ? n799 : n796)) : (n959 ? n799 : {n989[126:71], n805[60], n989[69:0]})) : n799;
  wire [126:0] n1017 = {n1011[126:32], (bytes_held + 32'd2)};
  wire n1023 = !cps_take;
  wire uop_push = n1007 & uop_room;
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
      6'h3F: n794 = (is_ep2 ? 1'b0 : (n682 ? 1'b0 : (n724 ? 1'b1 : n736)));
      default: n794 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n795 = (is_ep2 ? 3'd2 : (n682 ? 3'd2 : (n724 ? ((arg2 == 5'h14) ? 3'd0 : 3'd1) : (n736 ? 3'd2 : 3'd2))));
      default: n795 = 3'd2;
    endcase
  end

  always @* begin
    case (lb)
      6'h3E, 6'h3D, 6'h3C: n796 = {n241[126:50], ((lb == 6'h3E) ? 3'd0 : ((lb == 6'h3D) ? 3'd1 : 3'd2)), n241[46:0]};
      6'h1B: n796 = (((arg2[2:0] == 3'd7) | (arg2[4:3] != 2'd0)) ? {n272[126:112], {16'd0, cp}, n272[79:0]} : {n280[126:50], arg2[2:0], n280[46:0]});
      6'h3B: n796 = {5'd3, n235[121:0]};
      6'h3A, 6'h39, 6'h38, 6'h37: n796 = ((lb == 6'h37) ? {n323[126:112], {16'd0, cp}, n323[79:0]} : (pfx[41] ? {5'd6, n311[121:0]} : {n335[126:112], imm_mem, n335[79:0]}));
      6'h36: n796 = (pfx[40] ? {5'h13, n235[121:0]} : (pfx[39] ? {5'd0, n235[121:0]} : (pfx[41] ? {5'd7, n235[121:0]} : {n355[126:112], imm_mem, n355[79:0]})));
      6'h35, 6'h34, 6'h33, 6'h32: n796 = ((lb == 6'h32) ? {n373[126:112], {16'd0, cp}, n373[79:0]} : {n401[126:112], imm_alu, n401[79:0]});
      6'h31, 6'h30, 6'h2F: n796 = ((pfx[42] & pfx[102]) ? {n442[126:112], {16'd0, cp}, n442[79:0]} : {n428[126:112], imm_alu, n428[79:0]});
      6'h2E, 6'h2D, 6'h2C: n796 = (pfx[53] ? ((lb == 6'h2E) ? ((pfx[47:43] == 5'd0) ? {n467[126:112], {16'd0, cp}, n467[79:0]} : {5'hB, n235[121:0]}) : ((lb == 6'h2D) ? {5'hC, n235[121:0]} : {n488[126:112], {16'd0, cp}, n488[79:0]})) : {n498[126:66], ((lb == 6'h2E) ? 2'd0 : ((lb == 6'h2D) ? 2'd1 : 2'd2)), n498[63:0]});
      6'h2B, 6'h2A, 6'h29: n796 = {n538[126:112], {27'd0, arg2}, n538[79:0]};
      6'h28, 6'h27, 6'h26, 6'h25, 6'h24, 6'h23: n796 = ((pfx[38] | pfx[37]) ? {5'd0, n235[121:0]} : {n559[126:64], ((lb == 6'h28) ? 3'd0 : ((lb == 6'h27) ? 3'd1 : ((lb == 6'h26) ? 3'd2 : ((lb == 6'h25) ? 3'd3 : ((lb == 6'h24) ? 3'd4 : 3'd5))))), n559[60:0]});
      6'h22: n796 = ((arg2 == 5'h1A) ? {n597[126:59], 2'd1, n597[56:0]} : ((arg2 == 5'h19) ? {n597[126:59], 2'd2, n597[56:0]} : {n619[126:112], {16'd0, cp}, n619[79:0]}));
      6'h21: n796 = {n633[126:112], disp_bytes, n633[79:0]};
      6'h3F: n796 = (is_ep2 ? ((arg1 == 5'h19) ? {5'h11, n235[121:0]} : ((arg1 == 5'h1C) ? {5'd0, n235[121:0]} : ((arg1 == 5'h1D) ? {5'hF, n235[121:0]} : {n665[126:112], {16'd0, cp}, n665[79:0]}))) : (n682 ? ((pfx[42] & (arg2 == 5'h1B)) ? {n709[126:112], {16'd0, cp}, n709[79:0]} : {n685[126:61], ((arg2 == 5'h1C) ? 2'd0 : 2'd1), n685[58:0]}) : (n724 ? n235 : (n736 ? n235 : ((arg2 == 5'h11) ? {n750[126:112], {16'd0, cp}, n750[79:0]} : {n763[126:112], {16'd0, cp}, n763[79:0]})))));
      default: n796 = {n788[126:112], {16'd0, cp}, n788[79:0]};
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
      uop_e0 <= 127'd0;
      uop_e1 <= 127'd0;
      uop_wsalt_q <= 2'd0;
    end else begin
      pfx <= (flushing ? 103'd0 : (n1023 ? pfx : (n1007 ? 103'd0 : (cps_take ? (n809 ? (is_prefix ? (n812 ? n805 : (pfx_order ? {n928[102:37], 1'b1, n928[35:0]} : n928)) : n805) : n805) : pfx))));
      state <= (flushing ? 2'd0 : (n1023 ? state : (n1007 ? 2'd0 : (cps_take ? (n809 ? (is_prefix ? state : (n794 ? ((n795 == 3'd2) ? 2'd1 : 2'd2) : state)) : (n959 ? 2'd2 : 2'd0)) : state))));
      llc_dst <= (flushing ? 5'd0 : (n1023 ? llc_dst : (cps_take ? (n809 ? (is_prefix ? llc_dst : (n794 ? arg1 : llc_dst)) : llc_dst) : llc_dst)));
      llc_kind <= (flushing ? 3'd2 : (n1023 ? llc_kind : (cps_take ? (n809 ? (is_prefix ? llc_kind : (n794 ? n795 : llc_kind)) : llc_kind) : llc_kind)));
      llc_hi <= (flushing ? 16'd0 : (n1023 ? llc_hi : (cps_take ? (n809 ? llc_hi : (n959 ? cp : llc_hi)) : llc_hi)));
      cps_rsalt_q <= (cps_take ? (cps_rsalt_q ^ (cps_ridx ? 2'd2 : 2'd1)) : cps_rsalt_q);
      uop_e0 <= ((uop_push & (!uop_widx)) ? n1017 : uop_e0);
      uop_e1 <= ((uop_push & uop_widx) ? n1017 : uop_e1);
      uop_wsalt_q <= (uop_push ? (uop_wsalt_q ^ (uop_widx ? 2'd2 : 2'd1)) : uop_wsalt_q);
    end
  end

endmodule
