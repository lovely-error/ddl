// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build E:/Code/ddl/target/verify/k2g_decode/src.ddl -o E:/Code/ddl/examples/k2g_decode.v
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
    input          cps_valid,
    output         cps_ready,
    input  [16:0]  cps_data,
    output         uop_valid,
    input          uop_ready,
    output [126:0] uop_data
);

  reg [102:0] pfx;
  reg [1:0] state;
  reg [4:0] llc_dst;
  reg [2:0] llc_kind;
  reg [15:0] llc_hi;
  reg uop_busy;
  reg [126:0] uop_hold;

  wire n17 = (!uop_busy) | uop_ready;
  wire cps_xfer = cps_valid & n17;
  wire [15:0] cp = cps_data[16:1];
  wire flushing = cps_xfer & cps_data[0];
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
  reg [2:0] n140;
  reg n141;
  wire [31:0] imm10_sext = {{22{imm10[9]}}, imm10};
  wire [31:0] imm10_zext = {22'd0, imm10};
  wire [31:0] xi_ext = (lb == 6'h1E) ? imm10_sext : imm10_zext;
  wire [31:0] n153 = pfx[100:69];
  wire [31:0] imm_alu = pfx[102] ? {n153[26:0], arg2} : {{27{arg2[4]}}, arg2};
  wire [31:0] imm_mem = pfx[102] ? pfx[100:69] : 32'd0;
  wire [31:0] n164 = pfx[100:69];
  wire [19:0] disp20 = {n164[9:0], imm10};
  wire [31:0] disp_from_xi = pfx[101] ? {12'd0, disp20} : {{12{disp20[19]}}, disp20};
  wire [31:0] disp_ext = pfx[102] ? disp_from_xi : imm10_sext;
  wire [31:0] disp_bytes = {disp_ext[30:0], 1'b0};
  wire [126:0] n182 = 127'd0;
  wire [126:0] n189 = {n182[126:79], pfx[68:66], n182[75:0]};
  wire [126:0] n193 = {n189[126:76], pfx[65:61], n189[70:0]};
  wire [126:0] n197 = {n193[126:71], pfx[60], n193[69:0]};
  wire [126:0] n201 = {n197[126:51], pfx[59], n197[49:0]};
  wire [126:0] n205 = {n201[126:56], pfx[58:54], n201[50:0]};
  wire [126:0] n209 = {n205[126:57], pfx[42], n205[55:0]};
  wire [126:0] n213 = {n209[126:47], pfx[52:48], n209[41:0]};
  wire [126:0] n217 = {n213[126:42], pfx[47:43], n213[36:0]};
  wire [126:0] n220 = {n217[126:122], arg1, n217[116:0]};
  wire [126:0] n223 = {n220[126:117], arg2, n220[111:0]};
  wire [126:0] n226 = {5'd1, n223[121:0]};
  wire [126:0] n229 = {n226[126:112], imm_alu, n226[79:0]};
  wire [126:0] f = 127'd0;
  wire [126:0] n257 = {5'h12, f[121:0]};
  wire [126:0] n260 = {n257[126:37], 5'd1, n257[31:0]};
  wire [126:0] n268 = {5'd2, n223[121:0]};
  wire [126:0] n299 = {n223[126:50], ((lb == 6'h3A) ? 3'd0 : ((lb == 6'h39) ? 3'd1 : ((lb == 6'h38) ? 3'd2 : 3'd3))), n223[46:0]};
  wire [126:0] f_1 = 127'd0;
  wire [126:0] n308 = {5'h12, f_1[121:0]};
  wire [126:0] n311 = {n308[126:37], 5'd5, n308[31:0]};
  wire [126:0] n323 = {5'd4, n299[121:0]};
  wire [126:0] n343 = {5'd5, n223[121:0]};
  wire [126:0] f_2 = 127'd0;
  wire [126:0] n358 = {5'h12, f_2[121:0]};
  wire [126:0] n361 = {n358[126:37], 5'd4, n358[31:0]};
  wire [126:0] n369 = {5'd8, n223[121:0]};
  wire [126:0] n385 = {n369[126:70], ((lb == 6'h35) ? 2'd0 : ((lb == 6'h34) ? 2'd1 : 2'd2)), n369[67:0]};
  wire [126:0] n389 = {n385[126:80], pfx[102], n385[78:0]};
  wire [126:0] n396 = {5'd9, n223[121:0]};
  wire [126:0] n412 = {n396[126:68], ((lb == 6'h31) ? 2'd0 : ((lb == 6'h30) ? 2'd1 : 2'd2)), n396[65:0]};
  wire [126:0] n416 = {n412[126:80], pfx[102], n412[78:0]};
  wire [126:0] f_3 = 127'd0;
  wire [126:0] n427 = {5'h12, f_3[121:0]};
  wire [126:0] n430 = {n427[126:37], 5'd2, n427[31:0]};
  wire [126:0] f_4 = 127'd0;
  wire [126:0] n452 = {5'h12, f_4[121:0]};
  wire [126:0] n455 = {n452[126:37], 5'hB, n452[31:0]};
  wire [126:0] f_5 = 127'd0;
  wire [126:0] n473 = {5'h12, f_5[121:0]};
  wire [126:0] n476 = {n473[126:37], 5'd2, n473[31:0]};
  wire [126:0] n486 = {5'hA, n223[121:0]};
  wire [126:0] n506 = {5'hA, n223[121:0]};
  wire [126:0] n522 = {n506[126:66], ((lb == 6'h2B) ? 2'd0 : ((lb == 6'h2A) ? 2'd1 : 2'd2)), n506[63:0]};
  wire [126:0] n526 = {n522[126:80], 1'b1, n522[78:0]};
  wire [126:0] n540 = {5'hD, n223[121:0]};
  wire [126:0] n544 = {n540[126:80], pfx[102], n540[78:0]};
  wire [126:0] n547 = {n544[126:112], imm_alu, n544[79:0]};
  wire [126:0] n585 = {5'hE, n223[121:0]};
  wire [126:0] f_6 = 127'd0;
  wire [126:0] n604 = {5'h12, f_6[121:0]};
  wire [126:0] n607 = {n604[126:37], 5'd1, n604[31:0]};
  wire [126:0] n617 = {5'hE, n223[121:0]};
  wire [126:0] n621 = {n617[126:59], 2'd0, n617[56:0]};
  wire [126:0] f_7 = 127'd0;
  wire [126:0] n650 = {5'h12, f_7[121:0]};
  wire [126:0] n653 = {n650[126:37], 5'd1, n650[31:0]};
  wire n670 = (arg2 == 5'h1C) | (arg2 == 5'h1B);
  wire [126:0] n673 = {5'h10, n223[121:0]};
  wire [126:0] f_8 = 127'd0;
  wire [126:0] n694 = {5'h12, f_8[121:0]};
  wire [126:0] n697 = {n694[126:37], 5'd2, n694[31:0]};
  wire n712 = (arg2 == 5'h14) | (arg2 == 5'h13);
  wire n724 = arg2 == 5'h12;
  wire [126:0] f_9 = 127'd0;
  wire [126:0] n735 = {5'h12, f_9[121:0]};
  wire [126:0] n738 = {n735[126:37], 5'd5, n735[31:0]};
  wire [126:0] f_10 = 127'd0;
  wire [126:0] n748 = {5'h12, f_10[121:0]};
  wire [126:0] n751 = {n748[126:37], 5'd1, n748[31:0]};
  wire [126:0] f_11 = 127'd0;
  wire [126:0] n773 = {5'h12, f_11[121:0]};
  wire [126:0] n776 = {n773[126:37], 5'd1, n773[31:0]};
  reg n782;
  reg [2:0] n783;
  reg [126:0] n784;
  wire [126:0] n787 = 127'd0;
  wire [102:0] n793 = {pfx[102:32], (pfx[31:0] + 32'd2)};
  wire n797 = state == 2'd0;
  wire n800 = n793[35:32] >= 4'd7;
  wire [126:0] f_12 = 127'd0;
  wire [126:0] n805 = {5'h12, f_12[121:0]};
  wire [126:0] n808 = {n805[126:37], 5'd3, n805[31:0]};
  wire [102:0] n820 = {n793[102:36], (n793[35:32] + 4'd1), n793[31:0]};
  wire [102:0] n823 = {1'b1, n820[101:0]};
  wire [102:0] n830 = {n823[102], (lb == 6'h1D), n823[100:0]};
  wire [102:0] n834 = pfx_xi ? {n830[102:101], xi_ext, n830[68:0]} : n820;
  wire [102:0] n838 = {n834[102:54], 1'b1, n834[52:0]};
  wire [102:0] n841 = {n838[102:53], arg1, n838[47:0]};
  wire [102:0] n845 = pfx_bmx ? {n841[102:48], arg2, n841[42:0]} : n834;
  wire [102:0] n848 = {n845[102:69], n140, n845[65:0]};
  wire [102:0] n851 = {n848[102:66], arg1, n848[60:0]};
  wire [126:0] f_13 = 127'd0;
  wire [126:0] n863 = {5'h12, f_13[121:0]};
  wire [126:0] n866 = {n863[126:37], 5'd2, n863[31:0]};
  wire [102:0] n878 = pfx_xc ? (n141 ? {n851[102:61], (lb == 6'h20), n851[59:0]} : n845) : n845;
  wire [102:0] n882 = {n878[102:60], 1'b1, n878[58:0]};
  wire [102:0] n886 = pfx_uto ? {n882[102:59], arg1, n882[53:0]} : n878;
  wire [102:0] n891 = pfx_flag ? {n886[102:43], 1'b1, n886[41:0]} : n886;
  wire [102:0] n896 = pfx_csp ? {n891[102:42], 1'b1, n891[40:0]} : n891;
  wire [102:0] n901 = pfx_icinvr ? {n896[102:41], 1'b1, n896[39:0]} : n896;
  wire [102:0] n906 = pfx_esp ? {n901[102:40], 1'b1, n901[38:0]} : n901;
  wire [102:0] n911 = pfx_mpd ? {n906[102:39], 1'b1, n906[37:0]} : n906;
  wire [102:0] n916 = pfx_mpi ? {n911[102:38], 1'b1, n911[36:0]} : n911;
  wire n947 = state == 2'd1;
  wire [126:0] n951 = {5'd1, n787[121:0]};
  wire [126:0] n954 = {n951[126:122], llc_dst, n951[116:0]};
  wire [126:0] n957 = {n954[126:50], llc_kind, n954[46:0]};
  wire [126:0] n968 = {n957[126:112], ((llc_kind == 3'd2) ? {llc_hi, cp} : {16'd0, cp}), n957[79:0]};
  wire [126:0] n973 = {n968[126:79], n793[68:66], n968[75:0]};
  wire [126:0] n977 = {n973[126:76], n793[65:61], n973[70:0]};
  wire n995 = cps_xfer ? (n797 ? (is_prefix ? (n800 ? 1'b1 : (pfx_xc ? (n141 ? 1'b0 : 1'b1) : 1'b0)) : (n782 ? 1'b0 : 1'b1)) : (n947 ? 1'b0 : 1'b1)) : 1'b0;
  wire [126:0] n999 = cps_xfer ? (n797 ? (is_prefix ? (n800 ? {n808[126:112], {16'd0, cp}, n808[79:0]} : (pfx_xc ? (n141 ? n787 : {n866[126:112], {16'd0, cp}, n866[79:0]}) : n787)) : (n782 ? n787 : n784)) : (n947 ? n787 : {n977[126:71], n793[60], n977[69:0]})) : n787;
  wire n1011 = !cps_xfer;
  wire n1028 = cps_xfer & n995;

  always @* begin
    case (cc)
      5'h1F: n140 = 3'd1;
      5'h1E: n140 = 3'd2;
      5'h1D: n140 = 3'd3;
      5'h1C: n140 = 3'd4;
      5'h1B: n140 = 3'd5;
      default: n140 = 3'd0;
    endcase
  end

  always @* begin
    case (cc)
      5'h1F: n141 = 1'b1;
      5'h1E: n141 = 1'b1;
      5'h1D: n141 = 1'b1;
      5'h1C: n141 = 1'b1;
      5'h1B: n141 = 1'b1;
      default: n141 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n782 = (is_ep2 ? 1'b0 : (n670 ? 1'b0 : (n712 ? 1'b1 : n724)));
      default: n782 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n783 = (is_ep2 ? 3'd2 : (n670 ? 3'd2 : (n712 ? ((arg2 == 5'h14) ? 3'd0 : 3'd1) : (n724 ? 3'd2 : 3'd2))));
      default: n783 = 3'd2;
    endcase
  end

  always @* begin
    case (lb)
      6'h3E, 6'h3D, 6'h3C: n784 = {n229[126:50], ((lb == 6'h3E) ? 3'd0 : ((lb == 6'h3D) ? 3'd1 : 3'd2)), n229[46:0]};
      6'h1B: n784 = (((arg2[2:0] == 3'd7) | (arg2[4:3] != 2'd0)) ? {n260[126:112], {16'd0, cp}, n260[79:0]} : {n268[126:50], arg2[2:0], n268[46:0]});
      6'h3B: n784 = {5'd3, n223[121:0]};
      6'h3A, 6'h39, 6'h38, 6'h37: n784 = ((lb == 6'h37) ? {n311[126:112], {16'd0, cp}, n311[79:0]} : (pfx[41] ? {5'd6, n299[121:0]} : {n323[126:112], imm_mem, n323[79:0]}));
      6'h36: n784 = (pfx[40] ? {5'h13, n223[121:0]} : (pfx[39] ? {5'd0, n223[121:0]} : (pfx[41] ? {5'd7, n223[121:0]} : {n343[126:112], imm_mem, n343[79:0]})));
      6'h35, 6'h34, 6'h33, 6'h32: n784 = ((lb == 6'h32) ? {n361[126:112], {16'd0, cp}, n361[79:0]} : {n389[126:112], imm_alu, n389[79:0]});
      6'h31, 6'h30, 6'h2F: n784 = ((pfx[42] & pfx[102]) ? {n430[126:112], {16'd0, cp}, n430[79:0]} : {n416[126:112], imm_alu, n416[79:0]});
      6'h2E, 6'h2D, 6'h2C: n784 = (pfx[53] ? ((lb == 6'h2E) ? ((pfx[47:43] == 5'd0) ? {n455[126:112], {16'd0, cp}, n455[79:0]} : {5'hB, n223[121:0]}) : ((lb == 6'h2D) ? {5'hC, n223[121:0]} : {n476[126:112], {16'd0, cp}, n476[79:0]})) : {n486[126:66], ((lb == 6'h2E) ? 2'd0 : ((lb == 6'h2D) ? 2'd1 : 2'd2)), n486[63:0]});
      6'h2B, 6'h2A, 6'h29: n784 = {n526[126:112], {27'd0, arg2}, n526[79:0]};
      6'h28, 6'h27, 6'h26, 6'h25, 6'h24, 6'h23: n784 = ((pfx[38] | pfx[37]) ? {5'd0, n223[121:0]} : {n547[126:64], ((lb == 6'h28) ? 3'd0 : ((lb == 6'h27) ? 3'd1 : ((lb == 6'h26) ? 3'd2 : ((lb == 6'h25) ? 3'd3 : ((lb == 6'h24) ? 3'd4 : 3'd5))))), n547[60:0]});
      6'h22: n784 = ((arg2 == 5'h1A) ? {n585[126:59], 2'd1, n585[56:0]} : ((arg2 == 5'h19) ? {n585[126:59], 2'd2, n585[56:0]} : {n607[126:112], {16'd0, cp}, n607[79:0]}));
      6'h21: n784 = {n621[126:112], disp_bytes, n621[79:0]};
      6'h3F: n784 = (is_ep2 ? ((arg1 == 5'h19) ? {5'h11, n223[121:0]} : ((arg1 == 5'h1C) ? {5'd0, n223[121:0]} : ((arg1 == 5'h1D) ? {5'hF, n223[121:0]} : {n653[126:112], {16'd0, cp}, n653[79:0]}))) : (n670 ? ((pfx[42] & (arg2 == 5'h1B)) ? {n697[126:112], {16'd0, cp}, n697[79:0]} : {n673[126:61], ((arg2 == 5'h1C) ? 2'd0 : 2'd1), n673[58:0]}) : (n712 ? n223 : (n724 ? n223 : ((arg2 == 5'h11) ? {n738[126:112], {16'd0, cp}, n738[79:0]} : {n751[126:112], {16'd0, cp}, n751[79:0]})))));
      default: n784 = {n776[126:112], {16'd0, cp}, n776[79:0]};
    endcase
  end

  assign cps_ready = n17;
  assign uop_valid = uop_busy;
  assign uop_data = uop_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      pfx <= 103'd0;
      state <= 2'd0;
      llc_dst <= 5'd0;
      llc_kind <= 3'd2;
      llc_hi <= 16'd0;
      uop_busy <= 1'b0;
      uop_hold <= 127'd0;
    end else begin
      pfx <= (flushing ? 103'd0 : (n1011 ? pfx : (n995 ? 103'd0 : (cps_xfer ? (n797 ? (is_prefix ? (n800 ? n793 : (pfx_order ? {n916[102:37], 1'b1, n916[35:0]} : n916)) : n793) : n793) : pfx))));
      state <= (flushing ? 2'd0 : (n1011 ? state : (n995 ? 2'd0 : (cps_xfer ? (n797 ? (is_prefix ? state : (n782 ? ((n783 == 3'd2) ? 2'd1 : 2'd2) : state)) : (n947 ? 2'd2 : 2'd0)) : state))));
      llc_dst <= (flushing ? 5'd0 : (n1011 ? llc_dst : (cps_xfer ? (n797 ? (is_prefix ? llc_dst : (n782 ? arg1 : llc_dst)) : llc_dst) : llc_dst)));
      llc_kind <= (flushing ? 3'd2 : (n1011 ? llc_kind : (cps_xfer ? (n797 ? (is_prefix ? llc_kind : (n782 ? n783 : llc_kind)) : llc_kind) : llc_kind)));
      llc_hi <= (flushing ? 16'd0 : (n1011 ? llc_hi : (cps_xfer ? (n797 ? llc_hi : (n947 ? cp : llc_hi)) : llc_hi)));
      uop_busy <= (n1028 ? 1'b1 : (uop_ready ? 1'b0 : uop_busy));
      uop_hold <= (n1028 ? {n999[126:32], (bytes_held + 32'd2)} : uop_hold);
    end
  end

endmodule
