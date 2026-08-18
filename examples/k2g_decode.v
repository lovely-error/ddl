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
    input  [15:0]  cp,
    input          cp_valid,
    input          flush,
    input          hold,
    output         accept,
    output [126:0] uop,
    output         uop_valid
);

  reg [102:0] pfx;
  reg [1:0] state;
  reg [4:0] llc_dst;
  reg [2:0] llc_kind;
  reg [15:0] llc_hi;

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
  reg [2:0] n133;
  reg n134;
  wire [31:0] imm10_sext = {{22{imm10[9]}}, imm10};
  wire [31:0] imm10_zext = {22'd0, imm10};
  wire [31:0] xi_ext = (lb == 6'h1E) ? imm10_sext : imm10_zext;
  wire [31:0] n146 = pfx[100:69];
  wire [31:0] imm_alu = pfx[102] ? {n146[26:0], arg2} : {{27{arg2[4]}}, arg2};
  wire [31:0] imm_mem = pfx[102] ? pfx[100:69] : 32'd0;
  wire [31:0] n157 = pfx[100:69];
  wire [19:0] disp20 = {n157[9:0], imm10};
  wire [31:0] disp_from_xi = pfx[101] ? {12'd0, disp20} : {{12{disp20[19]}}, disp20};
  wire [31:0] disp_ext = pfx[102] ? disp_from_xi : imm10_sext;
  wire [31:0] disp_bytes = {disp_ext[30:0], 1'b0};
  wire [126:0] n175 = 127'd0;
  wire [126:0] n182 = {n175[126:79], pfx[68:66], n175[75:0]};
  wire [126:0] n186 = {n182[126:76], pfx[65:61], n182[70:0]};
  wire [126:0] n190 = {n186[126:71], pfx[60], n186[69:0]};
  wire [126:0] n194 = {n190[126:51], pfx[59], n190[49:0]};
  wire [126:0] n198 = {n194[126:56], pfx[58:54], n194[50:0]};
  wire [126:0] n202 = {n198[126:57], pfx[42], n198[55:0]};
  wire [126:0] n206 = {n202[126:47], pfx[52:48], n202[41:0]};
  wire [126:0] n210 = {n206[126:42], pfx[47:43], n206[36:0]};
  wire [126:0] n213 = {n210[126:122], arg1, n210[116:0]};
  wire [126:0] n216 = {n213[126:117], arg2, n213[111:0]};
  wire [126:0] n219 = {5'd1, n216[121:0]};
  wire [126:0] n222 = {n219[126:112], imm_alu, n219[79:0]};
  wire [126:0] f = 127'd0;
  wire [126:0] n250 = {5'h12, f[121:0]};
  wire [126:0] n253 = {n250[126:37], 5'd1, n250[31:0]};
  wire [126:0] n261 = {5'd2, n216[121:0]};
  wire [126:0] n292 = {n216[126:50], ((lb == 6'h3A) ? 3'd0 : ((lb == 6'h39) ? 3'd1 : ((lb == 6'h38) ? 3'd2 : 3'd3))), n216[46:0]};
  wire [126:0] f_1 = 127'd0;
  wire [126:0] n301 = {5'h12, f_1[121:0]};
  wire [126:0] n304 = {n301[126:37], 5'd5, n301[31:0]};
  wire [126:0] n316 = {5'd4, n292[121:0]};
  wire [126:0] n336 = {5'd5, n216[121:0]};
  wire [126:0] f_2 = 127'd0;
  wire [126:0] n351 = {5'h12, f_2[121:0]};
  wire [126:0] n354 = {n351[126:37], 5'd4, n351[31:0]};
  wire [126:0] n362 = {5'd8, n216[121:0]};
  wire [126:0] n378 = {n362[126:70], ((lb == 6'h35) ? 2'd0 : ((lb == 6'h34) ? 2'd1 : 2'd2)), n362[67:0]};
  wire [126:0] n382 = {n378[126:80], pfx[102], n378[78:0]};
  wire [126:0] n389 = {5'd9, n216[121:0]};
  wire [126:0] n405 = {n389[126:68], ((lb == 6'h31) ? 2'd0 : ((lb == 6'h30) ? 2'd1 : 2'd2)), n389[65:0]};
  wire [126:0] n409 = {n405[126:80], pfx[102], n405[78:0]};
  wire [126:0] f_3 = 127'd0;
  wire [126:0] n420 = {5'h12, f_3[121:0]};
  wire [126:0] n423 = {n420[126:37], 5'd2, n420[31:0]};
  wire [126:0] f_4 = 127'd0;
  wire [126:0] n445 = {5'h12, f_4[121:0]};
  wire [126:0] n448 = {n445[126:37], 5'hB, n445[31:0]};
  wire [126:0] f_5 = 127'd0;
  wire [126:0] n466 = {5'h12, f_5[121:0]};
  wire [126:0] n469 = {n466[126:37], 5'd2, n466[31:0]};
  wire [126:0] n479 = {5'hA, n216[121:0]};
  wire [126:0] n499 = {5'hA, n216[121:0]};
  wire [126:0] n515 = {n499[126:66], ((lb == 6'h2B) ? 2'd0 : ((lb == 6'h2A) ? 2'd1 : 2'd2)), n499[63:0]};
  wire [126:0] n519 = {n515[126:80], 1'b1, n515[78:0]};
  wire [126:0] n533 = {5'hD, n216[121:0]};
  wire [126:0] n537 = {n533[126:80], pfx[102], n533[78:0]};
  wire [126:0] n540 = {n537[126:112], imm_alu, n537[79:0]};
  wire [126:0] n578 = {5'hE, n216[121:0]};
  wire [126:0] f_6 = 127'd0;
  wire [126:0] n597 = {5'h12, f_6[121:0]};
  wire [126:0] n600 = {n597[126:37], 5'd1, n597[31:0]};
  wire [126:0] n610 = {5'hE, n216[121:0]};
  wire [126:0] n614 = {n610[126:59], 2'd0, n610[56:0]};
  wire [126:0] f_7 = 127'd0;
  wire [126:0] n643 = {5'h12, f_7[121:0]};
  wire [126:0] n646 = {n643[126:37], 5'd1, n643[31:0]};
  wire n663 = (arg2 == 5'h1C) | (arg2 == 5'h1B);
  wire [126:0] n666 = {5'h10, n216[121:0]};
  wire [126:0] f_8 = 127'd0;
  wire [126:0] n687 = {5'h12, f_8[121:0]};
  wire [126:0] n690 = {n687[126:37], 5'd2, n687[31:0]};
  wire n705 = (arg2 == 5'h14) | (arg2 == 5'h13);
  wire n717 = arg2 == 5'h12;
  wire [126:0] f_9 = 127'd0;
  wire [126:0] n728 = {5'h12, f_9[121:0]};
  wire [126:0] n731 = {n728[126:37], 5'd5, n728[31:0]};
  wire [126:0] f_10 = 127'd0;
  wire [126:0] n741 = {5'h12, f_10[121:0]};
  wire [126:0] n744 = {n741[126:37], 5'd1, n741[31:0]};
  wire [126:0] f_11 = 127'd0;
  wire [126:0] n767 = {5'h12, f_11[121:0]};
  wire [126:0] n770 = {n767[126:37], 5'd1, n767[31:0]};
  reg n776;
  reg [2:0] n777;
  reg [126:0] n778;
  wire [126:0] n781 = 127'd0;
  wire [102:0] n787 = {pfx[102:32], (pfx[31:0] + 32'd2)};
  wire n791 = state == 2'd0;
  wire n794 = n787[35:32] >= 4'd7;
  wire [126:0] f_12 = 127'd0;
  wire [126:0] n799 = {5'h12, f_12[121:0]};
  wire [126:0] n802 = {n799[126:37], 5'd3, n799[31:0]};
  wire [102:0] n814 = {n787[102:36], (n787[35:32] + 4'd1), n787[31:0]};
  wire [102:0] n817 = {1'b1, n814[101:0]};
  wire [102:0] n824 = {n817[102], (lb == 6'h1D), n817[100:0]};
  wire [102:0] n828 = pfx_xi ? {n824[102:101], xi_ext, n824[68:0]} : n814;
  wire [102:0] n832 = {n828[102:54], 1'b1, n828[52:0]};
  wire [102:0] n835 = {n832[102:53], arg1, n832[47:0]};
  wire [102:0] n839 = pfx_bmx ? {n835[102:48], arg2, n835[42:0]} : n828;
  wire [102:0] n842 = {n839[102:69], n133, n839[65:0]};
  wire [102:0] n845 = {n842[102:66], arg1, n842[60:0]};
  wire [126:0] f_13 = 127'd0;
  wire [126:0] n857 = {5'h12, f_13[121:0]};
  wire [126:0] n860 = {n857[126:37], 5'd2, n857[31:0]};
  wire [102:0] n872 = pfx_xc ? (n134 ? {n845[102:61], (lb == 6'h20), n845[59:0]} : n839) : n839;
  wire [102:0] n876 = {n872[102:60], 1'b1, n872[58:0]};
  wire [102:0] n880 = pfx_uto ? {n876[102:59], arg1, n876[53:0]} : n872;
  wire [102:0] n885 = pfx_flag ? {n880[102:43], 1'b1, n880[41:0]} : n880;
  wire [102:0] n890 = pfx_csp ? {n885[102:42], 1'b1, n885[40:0]} : n885;
  wire [102:0] n895 = pfx_icinvr ? {n890[102:41], 1'b1, n890[39:0]} : n890;
  wire [102:0] n900 = pfx_esp ? {n895[102:40], 1'b1, n895[38:0]} : n895;
  wire [102:0] n905 = pfx_mpd ? {n900[102:39], 1'b1, n900[37:0]} : n900;
  wire [102:0] n910 = pfx_mpi ? {n905[102:38], 1'b1, n905[36:0]} : n905;
  wire n941 = state == 2'd1;
  wire [126:0] n945 = {5'd1, n781[121:0]};
  wire [126:0] n948 = {n945[126:122], llc_dst, n945[116:0]};
  wire [126:0] n951 = {n948[126:50], llc_kind, n948[46:0]};
  wire [126:0] n962 = {n951[126:112], ((llc_kind == 3'd2) ? {llc_hi, cp} : {16'd0, cp}), n951[79:0]};
  wire [126:0] n967 = {n962[126:79], n787[68:66], n962[75:0]};
  wire [126:0] n971 = {n967[126:76], n787[65:61], n967[70:0]};
  wire n989 = cp_valid ? (n791 ? (is_prefix ? (n794 ? 1'b1 : (pfx_xc ? (n134 ? 1'b0 : 1'b1) : 1'b0)) : (n776 ? 1'b0 : 1'b1)) : (n941 ? 1'b0 : 1'b1)) : 1'b0;
  wire [126:0] n991 = cp_valid ? (n791 ? (is_prefix ? (n794 ? {n802[126:112], {16'd0, cp}, n802[79:0]} : (pfx_xc ? (n134 ? n781 : {n860[126:112], {16'd0, cp}, n860[79:0]}) : n781)) : (n776 ? n781 : n778)) : (n941 ? n781 : {n971[126:71], n787[60], n971[69:0]})) : n781;
  wire update = cp_valid & (!hold);
  wire n1007 = !update;

  always @* begin
    case (cc)
      5'h1F: n133 = 3'd1;
      5'h1E: n133 = 3'd2;
      5'h1D: n133 = 3'd3;
      5'h1C: n133 = 3'd4;
      5'h1B: n133 = 3'd5;
      default: n133 = 3'd0;
    endcase
  end

  always @* begin
    case (cc)
      5'h1F: n134 = 1'b1;
      5'h1E: n134 = 1'b1;
      5'h1D: n134 = 1'b1;
      5'h1C: n134 = 1'b1;
      5'h1B: n134 = 1'b1;
      default: n134 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n776 = (is_ep2 ? 1'b0 : (n663 ? 1'b0 : (n705 ? 1'b1 : (n717 ? 1'b1 : 1'b0))));
      default: n776 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n777 = (is_ep2 ? 3'd2 : (n663 ? 3'd2 : (n705 ? ((arg2 == 5'h14) ? 3'd0 : 3'd1) : (n717 ? 3'd2 : 3'd2))));
      default: n777 = 3'd2;
    endcase
  end

  always @* begin
    case (lb)
      6'h3E, 6'h3D, 6'h3C: n778 = {n222[126:50], ((lb == 6'h3E) ? 3'd0 : ((lb == 6'h3D) ? 3'd1 : 3'd2)), n222[46:0]};
      6'h1B: n778 = (((arg2[2:0] == 3'd7) | (arg2[4:3] != 2'd0)) ? {n253[126:112], {16'd0, cp}, n253[79:0]} : {n261[126:50], arg2[2:0], n261[46:0]});
      6'h3B: n778 = {5'd3, n216[121:0]};
      6'h3A, 6'h39, 6'h38, 6'h37: n778 = ((lb == 6'h37) ? {n304[126:112], {16'd0, cp}, n304[79:0]} : (pfx[41] ? {5'd6, n292[121:0]} : {n316[126:112], imm_mem, n316[79:0]}));
      6'h36: n778 = (pfx[40] ? {5'h13, n216[121:0]} : (pfx[39] ? {5'd0, n216[121:0]} : (pfx[41] ? {5'd7, n216[121:0]} : {n336[126:112], imm_mem, n336[79:0]})));
      6'h35, 6'h34, 6'h33, 6'h32: n778 = ((lb == 6'h32) ? {n354[126:112], {16'd0, cp}, n354[79:0]} : {n382[126:112], imm_alu, n382[79:0]});
      6'h31, 6'h30, 6'h2F: n778 = ((pfx[42] & pfx[102]) ? {n423[126:112], {16'd0, cp}, n423[79:0]} : {n409[126:112], imm_alu, n409[79:0]});
      6'h2E, 6'h2D, 6'h2C: n778 = (pfx[53] ? ((lb == 6'h2E) ? ((pfx[47:43] == 5'd0) ? {n448[126:112], {16'd0, cp}, n448[79:0]} : {5'hB, n216[121:0]}) : ((lb == 6'h2D) ? {5'hC, n216[121:0]} : {n469[126:112], {16'd0, cp}, n469[79:0]})) : {n479[126:66], ((lb == 6'h2E) ? 2'd0 : ((lb == 6'h2D) ? 2'd1 : 2'd2)), n479[63:0]});
      6'h2B, 6'h2A, 6'h29: n778 = {n519[126:112], {27'd0, arg2}, n519[79:0]};
      6'h28, 6'h27, 6'h26, 6'h25, 6'h24, 6'h23: n778 = ((pfx[38] | pfx[37]) ? {5'd0, n216[121:0]} : {n540[126:64], ((lb == 6'h28) ? 3'd0 : ((lb == 6'h27) ? 3'd1 : ((lb == 6'h26) ? 3'd2 : ((lb == 6'h25) ? 3'd3 : ((lb == 6'h24) ? 3'd4 : 3'd5))))), n540[60:0]});
      6'h22: n778 = ((arg2 == 5'h1A) ? {n578[126:59], 2'd1, n578[56:0]} : ((arg2 == 5'h19) ? {n578[126:59], 2'd2, n578[56:0]} : {n600[126:112], {16'd0, cp}, n600[79:0]}));
      6'h21: n778 = {n614[126:112], disp_bytes, n614[79:0]};
      6'h3F: n778 = (is_ep2 ? ((arg1 == 5'h19) ? {5'h11, n216[121:0]} : ((arg1 == 5'h1C) ? {5'd0, n216[121:0]} : ((arg1 == 5'h1D) ? {5'hF, n216[121:0]} : {n646[126:112], {16'd0, cp}, n646[79:0]}))) : (n663 ? ((pfx[42] & (arg2 == 5'h1B)) ? {n690[126:112], {16'd0, cp}, n690[79:0]} : {n666[126:61], ((arg2 == 5'h1C) ? 2'd0 : 2'd1), n666[58:0]}) : (n705 ? n216 : (n717 ? n216 : ((arg2 == 5'h11) ? {n731[126:112], {16'd0, cp}, n731[79:0]} : {n744[126:112], {16'd0, cp}, n744[79:0]})))));
      default: n778 = {n770[126:112], {16'd0, cp}, n770[79:0]};
    endcase
  end

  assign accept = cp_valid;
  assign uop = {n991[126:32], (bytes_held + 32'd2)};
  assign uop_valid = n989;

  always @(posedge clk) begin
    if (!rst_n) begin
      pfx <= 103'd0;
      state <= 2'd0;
      llc_dst <= 5'd0;
      llc_kind <= 3'd2;
      llc_hi <= 16'd0;
    end else begin
      pfx <= (flush ? 103'd0 : (n1007 ? pfx : (n989 ? 103'd0 : (cp_valid ? (n791 ? (is_prefix ? (n794 ? n787 : (pfx_order ? {n910[102:37], 1'b1, n910[35:0]} : n910)) : n787) : n787) : pfx))));
      state <= (flush ? 2'd0 : (n1007 ? state : (n989 ? 2'd0 : (cp_valid ? (n791 ? (is_prefix ? state : (n776 ? ((n777 == 3'd2) ? 2'd1 : 2'd2) : state)) : (n941 ? 2'd2 : 2'd0)) : state))));
      llc_dst <= (flush ? 5'd0 : (n1007 ? llc_dst : (cp_valid ? (n791 ? (is_prefix ? llc_dst : (n776 ? arg1 : llc_dst)) : llc_dst) : llc_dst)));
      llc_kind <= (flush ? 3'd2 : (n1007 ? llc_kind : (cp_valid ? (n791 ? (is_prefix ? llc_kind : (n776 ? n777 : llc_kind)) : llc_kind) : llc_kind)));
      llc_hi <= (flush ? 16'd0 : (n1007 ? llc_hi : (cp_valid ? (n791 ? llc_hi : (n941 ? cp : llc_hi)) : llc_hi)));
    end
  end

endmodule
