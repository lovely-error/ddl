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
  wire cps_xfer = !cps_empty;
  wire cps_empty_1 = cps_wsalt == cps_rsalt_q;
  wire cps_present = !cps_empty_1;
  wire [15:0] cp = cps_item[16:1];
  wire flushing = cps_present & cps_item[0];
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
  reg [2:0] n154;
  reg n155;
  wire [31:0] imm10_sext = {{22{imm10[9]}}, imm10};
  wire [31:0] imm10_zext = {22'd0, imm10};
  wire [31:0] xi_ext = (lb == 6'h1E) ? imm10_sext : imm10_zext;
  wire [31:0] n167 = pfx[100:69];
  wire [31:0] imm_alu = pfx[102] ? {n167[26:0], arg2} : {{27{arg2[4]}}, arg2};
  wire [31:0] imm_mem = pfx[102] ? pfx[100:69] : 32'd0;
  wire [31:0] n178 = pfx[100:69];
  wire [19:0] disp20 = {n178[9:0], imm10};
  wire [31:0] disp_from_xi = pfx[101] ? {12'd0, disp20} : {{12{disp20[19]}}, disp20};
  wire [31:0] disp_ext = pfx[102] ? disp_from_xi : imm10_sext;
  wire [31:0] disp_bytes = {disp_ext[30:0], 1'b0};
  wire [127:0] n196 = 128'd0;
  wire [127:0] n203 = {n196[127:80], pfx[68:66], n196[76:0]};
  wire [127:0] n207 = {n203[127:77], pfx[65:61], n203[71:0]};
  wire [127:0] n211 = {n207[127:72], pfx[60], n207[70:0]};
  wire [127:0] n215 = {n211[127:51], pfx[59], n211[49:0]};
  wire [127:0] n219 = {n215[127:56], pfx[58:54], n215[50:0]};
  wire [127:0] n223 = {n219[127:57], pfx[42], n219[55:0]};
  wire [127:0] n227 = {n223[127:47], pfx[52:48], n223[41:0]};
  wire [127:0] n231 = {n227[127:42], pfx[47:43], n227[36:0]};
  wire [127:0] n234 = {n231[127:123], arg1, n231[117:0]};
  wire [127:0] n237 = {n234[127:118], arg2, n234[112:0]};
  wire [127:0] n240 = {5'd1, n237[122:0]};
  wire [127:0] n243 = {n240[127:113], imm_alu, n240[80:0]};
  wire [127:0] f = 128'd0;
  wire [127:0] n271 = {5'h12, f[122:0]};
  wire [127:0] n274 = {n271[127:37], 5'd1, n271[31:0]};
  wire [127:0] n282 = {5'd2, n237[122:0]};
  wire [127:0] n313 = {n237[127:50], ((lb == 6'h3A) ? 3'd0 : ((lb == 6'h39) ? 3'd1 : ((lb == 6'h38) ? 3'd2 : 3'd3))), n237[46:0]};
  wire [127:0] f_1 = 128'd0;
  wire [127:0] n322 = {5'h12, f_1[122:0]};
  wire [127:0] n325 = {n322[127:37], 5'd5, n322[31:0]};
  wire [127:0] n337 = {5'd4, n313[122:0]};
  wire [127:0] n353 = {5'd5, n237[122:0]};
  wire [127:0] n356 = {n353[127:113], imm_mem, n353[80:0]};
  wire [127:0] f_2 = 128'd0;
  wire [127:0] n371 = {5'h12, f_2[122:0]};
  wire [127:0] n374 = {n371[127:37], 5'd4, n371[31:0]};
  wire [127:0] n382 = {5'd8, n237[122:0]};
  wire [127:0] n398 = {n382[127:71], ((lb == 6'h35) ? 2'd0 : ((lb == 6'h34) ? 2'd1 : 2'd2)), n382[68:0]};
  wire [127:0] n402 = {n398[127:81], pfx[102], n398[79:0]};
  wire [127:0] n409 = {5'd9, n237[122:0]};
  wire [127:0] n425 = {n409[127:69], ((lb == 6'h31) ? 2'd0 : ((lb == 6'h30) ? 2'd1 : 2'd2)), n409[66:0]};
  wire [127:0] n429 = {n425[127:81], pfx[102], n425[79:0]};
  wire [127:0] f_3 = 128'd0;
  wire [127:0] n440 = {5'h12, f_3[122:0]};
  wire [127:0] n443 = {n440[127:37], 5'd2, n440[31:0]};
  wire [127:0] f_4 = 128'd0;
  wire [127:0] n465 = {5'h12, f_4[122:0]};
  wire [127:0] n468 = {n465[127:37], 5'hB, n465[31:0]};
  wire [127:0] f_5 = 128'd0;
  wire [127:0] n486 = {5'h12, f_5[122:0]};
  wire [127:0] n489 = {n486[127:37], 5'd2, n486[31:0]};
  wire [127:0] n499 = {5'hA, n237[122:0]};
  wire [127:0] n519 = {5'hA, n237[122:0]};
  wire [127:0] n535 = {n519[127:67], ((lb == 6'h2B) ? 2'd0 : ((lb == 6'h2A) ? 2'd1 : 2'd2)), n519[64:0]};
  wire [127:0] n539 = {n535[127:81], 1'b1, n535[79:0]};
  wire [127:0] n553 = {5'hD, n237[122:0]};
  wire [127:0] n557 = {n553[127:81], pfx[102], n553[79:0]};
  wire [127:0] n560 = {n557[127:113], imm_alu, n557[80:0]};
  wire [127:0] n598 = {5'hE, n237[122:0]};
  wire [127:0] f_6 = 128'd0;
  wire [127:0] n617 = {5'h12, f_6[122:0]};
  wire [127:0] n620 = {n617[127:37], 5'd1, n617[31:0]};
  wire [127:0] n630 = {5'hE, n237[122:0]};
  wire [127:0] n634 = {n630[127:60], 2'd0, n630[57:0]};
  wire [127:0] f_7 = 128'd0;
  wire [127:0] n666 = {5'h12, f_7[122:0]};
  wire [127:0] n669 = {n666[127:37], 5'd2, n666[31:0]};
  wire [127:0] f_8 = 128'd0;
  wire [127:0] n683 = {5'h12, f_8[122:0]};
  wire [127:0] n686 = {n683[127:37], 5'd1, n683[31:0]};
  wire n703 = (arg2 == 5'h1C) | (arg2 == 5'h1B);
  wire [127:0] n706 = {5'h10, n237[122:0]};
  wire [127:0] f_9 = 128'd0;
  wire [127:0] n727 = {5'h12, f_9[122:0]};
  wire [127:0] n730 = {n727[127:37], 5'd2, n727[31:0]};
  wire n745 = (arg2 == 5'h14) | (arg2 == 5'h13);
  wire n757 = arg2 == 5'h12;
  wire [127:0] f_10 = 128'd0;
  wire [127:0] n768 = {5'h12, f_10[122:0]};
  wire [127:0] n771 = {n768[127:37], 5'd5, n768[31:0]};
  wire [127:0] f_11 = 128'd0;
  wire [127:0] n781 = {5'h12, f_11[122:0]};
  wire [127:0] n784 = {n781[127:37], 5'd1, n781[31:0]};
  wire [127:0] f_12 = 128'd0;
  wire [127:0] n806 = {5'h12, f_12[122:0]};
  wire [127:0] n809 = {n806[127:37], 5'd1, n806[31:0]};
  reg n815;
  reg [2:0] n816;
  reg [127:0] n817;
  wire [127:0] n820 = 128'd0;
  wire [102:0] n826 = {pfx[102:32], (pfx[31:0] + 32'd2)};
  wire n830 = state == 2'd0;
  wire n833 = n826[35:32] >= 4'd7;
  wire [127:0] f_13 = 128'd0;
  wire [127:0] n838 = {5'h12, f_13[122:0]};
  wire [127:0] n841 = {n838[127:37], 5'd3, n838[31:0]};
  wire [102:0] n853 = {n826[102:36], (n826[35:32] + 4'd1), n826[31:0]};
  wire [102:0] n856 = {1'b1, n853[101:0]};
  wire [102:0] n863 = {n856[102], (lb == 6'h1D), n856[100:0]};
  wire [102:0] n867 = pfx_xi ? {n863[102:101], xi_ext, n863[68:0]} : n853;
  wire [102:0] n871 = {n867[102:54], 1'b1, n867[52:0]};
  wire [102:0] n874 = {n871[102:53], arg1, n871[47:0]};
  wire [102:0] n878 = pfx_bmx ? {n874[102:48], arg2, n874[42:0]} : n867;
  wire [102:0] n881 = {n878[102:69], n154, n878[65:0]};
  wire [102:0] n884 = {n881[102:66], arg1, n881[60:0]};
  wire [127:0] f_14 = 128'd0;
  wire [127:0] n896 = {5'h12, f_14[122:0]};
  wire [127:0] n899 = {n896[127:37], 5'd2, n896[31:0]};
  wire [102:0] n911 = pfx_xc ? (n155 ? {n884[102:61], (lb == 6'h20), n884[59:0]} : n878) : n878;
  wire [102:0] n915 = {n911[102:60], 1'b1, n911[58:0]};
  wire [102:0] n919 = pfx_uto ? {n915[102:59], arg1, n915[53:0]} : n911;
  wire [102:0] n924 = pfx_flag ? {n919[102:43], 1'b1, n919[41:0]} : n919;
  wire [102:0] n929 = pfx_csp ? {n924[102:42], 1'b1, n924[40:0]} : n924;
  wire [102:0] n934 = pfx_istore ? {n929[102:41], 1'b1, n929[39:0]} : n929;
  wire [102:0] n939 = pfx_esp ? {n934[102:40], 1'b1, n934[38:0]} : n934;
  wire [102:0] n944 = pfx_mpd ? {n939[102:39], 1'b1, n939[37:0]} : n939;
  wire [102:0] n949 = pfx_mpi ? {n944[102:38], 1'b1, n944[36:0]} : n944;
  wire n980 = state == 2'd1;
  wire [127:0] n984 = {5'd1, n820[122:0]};
  wire [127:0] n987 = {n984[127:123], llc_dst, n984[117:0]};
  wire [127:0] n990 = {n987[127:50], llc_kind, n987[46:0]};
  wire [127:0] n1001 = {n990[127:113], ((llc_kind == 3'd2) ? {llc_hi, cp} : {16'd0, cp}), n990[80:0]};
  wire [127:0] n1006 = {n1001[127:80], n826[68:66], n1001[76:0]};
  wire [127:0] n1010 = {n1006[127:77], n826[65:61], n1006[71:0]};
  wire n1028 = cps_present ? (n830 ? (is_prefix ? (n833 ? 1'b1 : (pfx_xc ? (n155 ? 1'b0 : 1'b1) : 1'b0)) : (n815 ? 1'b0 : 1'b1)) : (n980 ? 1'b0 : 1'b1)) : 1'b0;
  wire [127:0] n1032 = cps_present ? (n830 ? (is_prefix ? (n833 ? {n841[127:113], {16'd0, cp}, n841[80:0]} : (pfx_xc ? (n155 ? n820 : {n899[127:113], {16'd0, cp}, n899[80:0]}) : n820)) : (n815 ? n820 : n817)) : (n980 ? n820 : {n1010[127:72], n826[60], n1010[70:0]})) : n820;
  wire [127:0] n1038 = {n1032[127:32], (bytes_held + 32'd2)};
  wire update = cps_present & ((!n1028) | (n1028 ? (uop_room & n1028) : 1'b0));
  wire consumed = cps_xfer & update;
  wire n1048 = flushing & update;
  wire n1054 = !update;
  wire cps_take = cps_xfer & update;
  wire uop_push = uop_room & n1028;
  wire uop_widx = uop_wsalt_q[0] ^ uop_wsalt_q[1];

  always @* begin
    case (cc)
      5'h1F: n154 = 3'd1;
      5'h1E: n154 = 3'd2;
      5'h1D: n154 = 3'd3;
      5'h1C: n154 = 3'd4;
      5'h1B: n154 = 3'd5;
      default: n154 = 3'd0;
    endcase
  end

  always @* begin
    case (cc)
      5'h1F: n155 = 1'b1;
      5'h1E: n155 = 1'b1;
      5'h1D: n155 = 1'b1;
      5'h1C: n155 = 1'b1;
      5'h1B: n155 = 1'b1;
      default: n155 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n815 = (is_ep2 ? 1'b0 : (n703 ? 1'b0 : (n745 ? 1'b1 : n757)));
      default: n815 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n816 = (is_ep2 ? 3'd2 : (n703 ? 3'd2 : (n745 ? ((arg2 == 5'h14) ? 3'd0 : 3'd1) : (n757 ? 3'd2 : 3'd2))));
      default: n816 = 3'd2;
    endcase
  end

  always @* begin
    case (lb)
      6'h3E, 6'h3D, 6'h3C: n817 = {n243[127:50], ((lb == 6'h3E) ? 3'd0 : ((lb == 6'h3D) ? 3'd1 : 3'd2)), n243[46:0]};
      6'h1B: n817 = (((arg2[2:0] == 3'd7) | (arg2[4:3] != 2'd0)) ? {n274[127:113], {16'd0, cp}, n274[80:0]} : {n282[127:50], arg2[2:0], n282[46:0]});
      6'h3B: n817 = {5'd3, n237[122:0]};
      6'h3A, 6'h39, 6'h38, 6'h37: n817 = ((lb == 6'h37) ? {n325[127:113], {16'd0, cp}, n325[80:0]} : (pfx[41] ? {5'd6, n313[122:0]} : {n337[127:113], imm_mem, n337[80:0]}));
      6'h36: n817 = (pfx[39] ? {5'd0, n237[122:0]} : (pfx[41] ? {5'd7, n237[122:0]} : {n356[127:58], pfx[40], n356[56:0]}));
      6'h35, 6'h34, 6'h33, 6'h32: n817 = ((lb == 6'h32) ? {n374[127:113], {16'd0, cp}, n374[80:0]} : {n402[127:113], imm_alu, n402[80:0]});
      6'h31, 6'h30, 6'h2F: n817 = ((pfx[42] & pfx[102]) ? {n443[127:113], {16'd0, cp}, n443[80:0]} : {n429[127:113], imm_alu, n429[80:0]});
      6'h2E, 6'h2D, 6'h2C: n817 = (pfx[53] ? ((lb == 6'h2E) ? ((pfx[47:43] == 5'd0) ? {n468[127:113], {16'd0, cp}, n468[80:0]} : {5'hB, n237[122:0]}) : ((lb == 6'h2D) ? {5'hC, n237[122:0]} : {n489[127:113], {16'd0, cp}, n489[80:0]})) : {n499[127:67], ((lb == 6'h2E) ? 2'd0 : ((lb == 6'h2D) ? 2'd1 : 2'd2)), n499[64:0]});
      6'h2B, 6'h2A, 6'h29: n817 = {n539[127:113], {27'd0, arg2}, n539[80:0]};
      6'h28, 6'h27, 6'h26, 6'h25, 6'h24, 6'h23: n817 = ((pfx[38] | pfx[37]) ? {5'd0, n237[122:0]} : {n560[127:65], ((lb == 6'h28) ? 3'd0 : ((lb == 6'h27) ? 3'd1 : ((lb == 6'h26) ? 3'd2 : ((lb == 6'h25) ? 3'd3 : ((lb == 6'h24) ? 3'd4 : 3'd5))))), n560[61:0]});
      6'h22: n817 = ((arg2 == 5'h1A) ? {n598[127:60], 2'd1, n598[57:0]} : ((arg2 == 5'h19) ? {n598[127:60], 2'd2, n598[57:0]} : {n620[127:113], {16'd0, cp}, n620[80:0]}));
      6'h21: n817 = {n634[127:113], disp_bytes, n634[80:0]};
      6'h3F: n817 = (is_ep2 ? ((arg1 == 5'h19) ? {5'h11, n237[122:0]} : ((arg1 == 5'h1C) ? {5'd0, n237[122:0]} : ((arg1 == 5'h1D) ? ((pfx[68:66] != 3'd0) ? {n669[127:113], {16'd0, cp}, n669[80:0]} : {5'hF, n237[122:0]}) : {n686[127:113], {16'd0, cp}, n686[80:0]}))) : (n703 ? ((pfx[42] & (arg2 == 5'h1B)) ? {n730[127:113], {16'd0, cp}, n730[80:0]} : {n706[127:62], ((arg2 == 5'h1C) ? 2'd0 : 2'd1), n706[59:0]}) : (n745 ? n237 : (n757 ? n237 : ((arg2 == 5'h11) ? {n771[127:113], {16'd0, cp}, n771[80:0]} : {n784[127:113], {16'd0, cp}, n784[80:0]})))));
      default: n817 = {n809[127:113], {16'd0, cp}, n809[80:0]};
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
      pfx <= (n1048 ? 103'd0 : (n1054 ? pfx : (n1028 ? 103'd0 : (cps_present ? (n830 ? (is_prefix ? (n833 ? n826 : (pfx_order ? {n949[102:37], 1'b1, n949[35:0]} : n949)) : n826) : n826) : pfx))));
      state <= (n1048 ? 2'd0 : (n1054 ? state : (n1028 ? 2'd0 : (cps_present ? (n830 ? (is_prefix ? state : (n815 ? ((n816 == 3'd2) ? 2'd1 : 2'd2) : state)) : (n980 ? 2'd2 : 2'd0)) : state))));
      llc_dst <= (n1048 ? 5'd0 : (n1054 ? llc_dst : (cps_present ? (n830 ? (is_prefix ? llc_dst : (n815 ? arg1 : llc_dst)) : llc_dst) : llc_dst)));
      llc_kind <= (n1048 ? 3'd2 : (n1054 ? llc_kind : (cps_present ? (n830 ? (is_prefix ? llc_kind : (n815 ? n816 : llc_kind)) : llc_kind) : llc_kind)));
      llc_hi <= (n1048 ? 16'd0 : (n1054 ? llc_hi : (cps_present ? (n830 ? llc_hi : (n980 ? cp : llc_hi)) : llc_hi)));
      cps_rsalt_q <= (cps_take ? (cps_rsalt_q ^ (cps_ridx ? 2'd2 : 2'd1)) : cps_rsalt_q);
      uop_e0 <= ((uop_push & (!uop_widx)) ? n1038 : uop_e0);
      uop_e1 <= ((uop_push & uop_widx) ? n1038 : uop_e1);
      uop_wsalt_q <= (uop_push ? (uop_wsalt_q ^ (uop_widx ? 2'd2 : 2'd1)) : uop_wsalt_q);
    end
  end

`ifdef SIMULATION
  always @(posedge clk) begin
    if (rst_n) begin
      if (!(((!update) | consumed))) $error("%m: @assert failed");
    end
  end
`endif

endmodule
