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
    input  [15:0]  cps_data,
    input          flush_valid,
    input          flush_data,
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

  wire n18 = (!uop_busy) | uop_ready;
  wire cps_xfer = cps_valid & n18;
  wire [31:0] bytes_held = pfx[31:0];
  wire [5:0] lb = cps_data[15:10];
  wire [4:0] arg1 = cps_data[9:5];
  wire [4:0] arg2 = cps_data[4:0];
  wire [9:0] imm10 = cps_data[9:0];
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
  reg [2:0] n139;
  reg n140;
  wire [31:0] imm10_sext = {{22{imm10[9]}}, imm10};
  wire [31:0] imm10_zext = {22'd0, imm10};
  wire [31:0] xi_ext = (lb == 6'h1E) ? imm10_sext : imm10_zext;
  wire [31:0] n152 = pfx[100:69];
  wire [31:0] imm_alu = pfx[102] ? {n152[26:0], arg2} : {{27{arg2[4]}}, arg2};
  wire [31:0] imm_mem = pfx[102] ? pfx[100:69] : 32'd0;
  wire [31:0] n163 = pfx[100:69];
  wire [19:0] disp20 = {n163[9:0], imm10};
  wire [31:0] disp_from_xi = pfx[101] ? {12'd0, disp20} : {{12{disp20[19]}}, disp20};
  wire [31:0] disp_ext = pfx[102] ? disp_from_xi : imm10_sext;
  wire [31:0] disp_bytes = {disp_ext[30:0], 1'b0};
  wire [126:0] n181 = 127'd0;
  wire [126:0] n188 = {n181[126:79], pfx[68:66], n181[75:0]};
  wire [126:0] n192 = {n188[126:76], pfx[65:61], n188[70:0]};
  wire [126:0] n196 = {n192[126:71], pfx[60], n192[69:0]};
  wire [126:0] n200 = {n196[126:51], pfx[59], n196[49:0]};
  wire [126:0] n204 = {n200[126:56], pfx[58:54], n200[50:0]};
  wire [126:0] n208 = {n204[126:57], pfx[42], n204[55:0]};
  wire [126:0] n212 = {n208[126:47], pfx[52:48], n208[41:0]};
  wire [126:0] n216 = {n212[126:42], pfx[47:43], n212[36:0]};
  wire [126:0] n219 = {n216[126:122], arg1, n216[116:0]};
  wire [126:0] n222 = {n219[126:117], arg2, n219[111:0]};
  wire [126:0] n225 = {5'd1, n222[121:0]};
  wire [126:0] n228 = {n225[126:112], imm_alu, n225[79:0]};
  wire [126:0] f = 127'd0;
  wire [126:0] n256 = {5'h12, f[121:0]};
  wire [126:0] n259 = {n256[126:37], 5'd1, n256[31:0]};
  wire [126:0] n267 = {5'd2, n222[121:0]};
  wire [126:0] n298 = {n222[126:50], ((lb == 6'h3A) ? 3'd0 : ((lb == 6'h39) ? 3'd1 : ((lb == 6'h38) ? 3'd2 : 3'd3))), n222[46:0]};
  wire [126:0] f_1 = 127'd0;
  wire [126:0] n307 = {5'h12, f_1[121:0]};
  wire [126:0] n310 = {n307[126:37], 5'd5, n307[31:0]};
  wire [126:0] n322 = {5'd4, n298[121:0]};
  wire [126:0] n342 = {5'd5, n222[121:0]};
  wire [126:0] f_2 = 127'd0;
  wire [126:0] n357 = {5'h12, f_2[121:0]};
  wire [126:0] n360 = {n357[126:37], 5'd4, n357[31:0]};
  wire [126:0] n368 = {5'd8, n222[121:0]};
  wire [126:0] n384 = {n368[126:70], ((lb == 6'h35) ? 2'd0 : ((lb == 6'h34) ? 2'd1 : 2'd2)), n368[67:0]};
  wire [126:0] n388 = {n384[126:80], pfx[102], n384[78:0]};
  wire [126:0] n395 = {5'd9, n222[121:0]};
  wire [126:0] n411 = {n395[126:68], ((lb == 6'h31) ? 2'd0 : ((lb == 6'h30) ? 2'd1 : 2'd2)), n395[65:0]};
  wire [126:0] n415 = {n411[126:80], pfx[102], n411[78:0]};
  wire [126:0] f_3 = 127'd0;
  wire [126:0] n426 = {5'h12, f_3[121:0]};
  wire [126:0] n429 = {n426[126:37], 5'd2, n426[31:0]};
  wire [126:0] f_4 = 127'd0;
  wire [126:0] n451 = {5'h12, f_4[121:0]};
  wire [126:0] n454 = {n451[126:37], 5'hB, n451[31:0]};
  wire [126:0] f_5 = 127'd0;
  wire [126:0] n472 = {5'h12, f_5[121:0]};
  wire [126:0] n475 = {n472[126:37], 5'd2, n472[31:0]};
  wire [126:0] n485 = {5'hA, n222[121:0]};
  wire [126:0] n505 = {5'hA, n222[121:0]};
  wire [126:0] n521 = {n505[126:66], ((lb == 6'h2B) ? 2'd0 : ((lb == 6'h2A) ? 2'd1 : 2'd2)), n505[63:0]};
  wire [126:0] n525 = {n521[126:80], 1'b1, n521[78:0]};
  wire [126:0] n539 = {5'hD, n222[121:0]};
  wire [126:0] n543 = {n539[126:80], pfx[102], n539[78:0]};
  wire [126:0] n546 = {n543[126:112], imm_alu, n543[79:0]};
  wire [126:0] n584 = {5'hE, n222[121:0]};
  wire [126:0] f_6 = 127'd0;
  wire [126:0] n603 = {5'h12, f_6[121:0]};
  wire [126:0] n606 = {n603[126:37], 5'd1, n603[31:0]};
  wire [126:0] n616 = {5'hE, n222[121:0]};
  wire [126:0] n620 = {n616[126:59], 2'd0, n616[56:0]};
  wire [126:0] f_7 = 127'd0;
  wire [126:0] n649 = {5'h12, f_7[121:0]};
  wire [126:0] n652 = {n649[126:37], 5'd1, n649[31:0]};
  wire n669 = (arg2 == 5'h1C) | (arg2 == 5'h1B);
  wire [126:0] n672 = {5'h10, n222[121:0]};
  wire [126:0] f_8 = 127'd0;
  wire [126:0] n693 = {5'h12, f_8[121:0]};
  wire [126:0] n696 = {n693[126:37], 5'd2, n693[31:0]};
  wire n711 = (arg2 == 5'h14) | (arg2 == 5'h13);
  wire n723 = arg2 == 5'h12;
  wire [126:0] f_9 = 127'd0;
  wire [126:0] n734 = {5'h12, f_9[121:0]};
  wire [126:0] n737 = {n734[126:37], 5'd5, n734[31:0]};
  wire [126:0] f_10 = 127'd0;
  wire [126:0] n747 = {5'h12, f_10[121:0]};
  wire [126:0] n750 = {n747[126:37], 5'd1, n747[31:0]};
  wire [126:0] f_11 = 127'd0;
  wire [126:0] n772 = {5'h12, f_11[121:0]};
  wire [126:0] n775 = {n772[126:37], 5'd1, n772[31:0]};
  reg [126:0] n781;
  reg n782;
  reg [2:0] n783;
  wire [126:0] n786 = 127'd0;
  wire [102:0] n792 = {pfx[102:32], (pfx[31:0] + 32'd2)};
  wire n796 = state == 2'd0;
  wire n799 = n792[35:32] >= 4'd7;
  wire [126:0] f_12 = 127'd0;
  wire [126:0] n804 = {5'h12, f_12[121:0]};
  wire [126:0] n807 = {n804[126:37], 5'd3, n804[31:0]};
  wire [102:0] n819 = {n792[102:36], (n792[35:32] + 4'd1), n792[31:0]};
  wire [102:0] n822 = {1'b1, n819[101:0]};
  wire [102:0] n829 = {n822[102], (lb == 6'h1D), n822[100:0]};
  wire [102:0] n833 = pfx_xi ? {n829[102:101], xi_ext, n829[68:0]} : n819;
  wire [102:0] n837 = {n833[102:54], 1'b1, n833[52:0]};
  wire [102:0] n840 = {n837[102:53], arg1, n837[47:0]};
  wire [102:0] n844 = pfx_bmx ? {n840[102:48], arg2, n840[42:0]} : n833;
  wire [102:0] n847 = {n844[102:69], n139, n844[65:0]};
  wire [102:0] n850 = {n847[102:66], arg1, n847[60:0]};
  wire [126:0] f_13 = 127'd0;
  wire [126:0] n862 = {5'h12, f_13[121:0]};
  wire [126:0] n865 = {n862[126:37], 5'd2, n862[31:0]};
  wire [102:0] n877 = pfx_xc ? (n140 ? {n850[102:61], (lb == 6'h20), n850[59:0]} : n844) : n844;
  wire [102:0] n881 = {n877[102:60], 1'b1, n877[58:0]};
  wire [102:0] n885 = pfx_uto ? {n881[102:59], arg1, n881[53:0]} : n877;
  wire [102:0] n890 = pfx_flag ? {n885[102:43], 1'b1, n885[41:0]} : n885;
  wire [102:0] n895 = pfx_csp ? {n890[102:42], 1'b1, n890[40:0]} : n890;
  wire [102:0] n900 = pfx_icinvr ? {n895[102:41], 1'b1, n895[39:0]} : n895;
  wire [102:0] n905 = pfx_esp ? {n900[102:40], 1'b1, n900[38:0]} : n900;
  wire [102:0] n910 = pfx_mpd ? {n905[102:39], 1'b1, n905[37:0]} : n905;
  wire [102:0] n915 = pfx_mpi ? {n910[102:38], 1'b1, n910[36:0]} : n910;
  wire n946 = state == 2'd1;
  wire [126:0] n950 = {5'd1, n786[121:0]};
  wire [126:0] n953 = {n950[126:122], llc_dst, n950[116:0]};
  wire [126:0] n956 = {n953[126:50], llc_kind, n953[46:0]};
  wire [126:0] n967 = {n956[126:112], ((llc_kind == 3'd2) ? {llc_hi, cps_data} : {16'd0, cps_data}), n956[79:0]};
  wire [126:0] n972 = {n967[126:79], n792[68:66], n967[75:0]};
  wire [126:0] n976 = {n972[126:76], n792[65:61], n972[70:0]};
  wire [126:0] n995 = cps_xfer ? (n796 ? (is_prefix ? (n799 ? {n807[126:112], {16'd0, cps_data}, n807[79:0]} : (pfx_xc ? (n140 ? n786 : {n865[126:112], {16'd0, cps_data}, n865[79:0]}) : n786)) : (n782 ? n786 : n781)) : (n946 ? n786 : {n976[126:71], n792[60], n976[69:0]})) : n786;
  wire n996 = cps_xfer ? (n796 ? (is_prefix ? (n799 ? 1'b1 : (pfx_xc ? (n140 ? 1'b0 : 1'b1) : 1'b0)) : (n782 ? 1'b0 : 1'b1)) : (n946 ? 1'b0 : 1'b1)) : 1'b0;
  wire n1010 = !cps_xfer;
  wire n1027 = cps_xfer & n996;

  always @* begin
    case (cc)
      5'h1F: n139 = 3'd1;
      5'h1E: n139 = 3'd2;
      5'h1D: n139 = 3'd3;
      5'h1C: n139 = 3'd4;
      5'h1B: n139 = 3'd5;
      default: n139 = 3'd0;
    endcase
  end

  always @* begin
    case (cc)
      5'h1F: n140 = 1'b1;
      5'h1E: n140 = 1'b1;
      5'h1D: n140 = 1'b1;
      5'h1C: n140 = 1'b1;
      5'h1B: n140 = 1'b1;
      default: n140 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3E, 6'h3D, 6'h3C: n781 = {n228[126:50], ((lb == 6'h3E) ? 3'd0 : ((lb == 6'h3D) ? 3'd1 : 3'd2)), n228[46:0]};
      6'h1B: n781 = (((arg2[2:0] == 3'd7) | (arg2[4:3] != 2'd0)) ? {n259[126:112], {16'd0, cps_data}, n259[79:0]} : {n267[126:50], arg2[2:0], n267[46:0]});
      6'h3B: n781 = {5'd3, n222[121:0]};
      6'h3A, 6'h39, 6'h38, 6'h37: n781 = ((lb == 6'h37) ? {n310[126:112], {16'd0, cps_data}, n310[79:0]} : (pfx[41] ? {5'd6, n298[121:0]} : {n322[126:112], imm_mem, n322[79:0]}));
      6'h36: n781 = (pfx[40] ? {5'h13, n222[121:0]} : (pfx[39] ? {5'd0, n222[121:0]} : (pfx[41] ? {5'd7, n222[121:0]} : {n342[126:112], imm_mem, n342[79:0]})));
      6'h35, 6'h34, 6'h33, 6'h32: n781 = ((lb == 6'h32) ? {n360[126:112], {16'd0, cps_data}, n360[79:0]} : {n388[126:112], imm_alu, n388[79:0]});
      6'h31, 6'h30, 6'h2F: n781 = ((pfx[42] & pfx[102]) ? {n429[126:112], {16'd0, cps_data}, n429[79:0]} : {n415[126:112], imm_alu, n415[79:0]});
      6'h2E, 6'h2D, 6'h2C: n781 = (pfx[53] ? ((lb == 6'h2E) ? ((pfx[47:43] == 5'd0) ? {n454[126:112], {16'd0, cps_data}, n454[79:0]} : {5'hB, n222[121:0]}) : ((lb == 6'h2D) ? {5'hC, n222[121:0]} : {n475[126:112], {16'd0, cps_data}, n475[79:0]})) : {n485[126:66], ((lb == 6'h2E) ? 2'd0 : ((lb == 6'h2D) ? 2'd1 : 2'd2)), n485[63:0]});
      6'h2B, 6'h2A, 6'h29: n781 = {n525[126:112], {27'd0, arg2}, n525[79:0]};
      6'h28, 6'h27, 6'h26, 6'h25, 6'h24, 6'h23: n781 = ((pfx[38] | pfx[37]) ? {5'd0, n222[121:0]} : {n546[126:64], ((lb == 6'h28) ? 3'd0 : ((lb == 6'h27) ? 3'd1 : ((lb == 6'h26) ? 3'd2 : ((lb == 6'h25) ? 3'd3 : ((lb == 6'h24) ? 3'd4 : 3'd5))))), n546[60:0]});
      6'h22: n781 = ((arg2 == 5'h1A) ? {n584[126:59], 2'd1, n584[56:0]} : ((arg2 == 5'h19) ? {n584[126:59], 2'd2, n584[56:0]} : {n606[126:112], {16'd0, cps_data}, n606[79:0]}));
      6'h21: n781 = {n620[126:112], disp_bytes, n620[79:0]};
      6'h3F: n781 = (is_ep2 ? ((arg1 == 5'h19) ? {5'h11, n222[121:0]} : ((arg1 == 5'h1C) ? {5'd0, n222[121:0]} : ((arg1 == 5'h1D) ? {5'hF, n222[121:0]} : {n652[126:112], {16'd0, cps_data}, n652[79:0]}))) : (n669 ? ((pfx[42] & (arg2 == 5'h1B)) ? {n696[126:112], {16'd0, cps_data}, n696[79:0]} : {n672[126:61], ((arg2 == 5'h1C) ? 2'd0 : 2'd1), n672[58:0]}) : (n711 ? n222 : (n723 ? n222 : ((arg2 == 5'h11) ? {n737[126:112], {16'd0, cps_data}, n737[79:0]} : {n750[126:112], {16'd0, cps_data}, n750[79:0]})))));
      default: n781 = {n775[126:112], {16'd0, cps_data}, n775[79:0]};
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n782 = (is_ep2 ? 1'b0 : (n669 ? 1'b0 : (n711 ? 1'b1 : n723)));
      default: n782 = 1'b0;
    endcase
  end

  always @* begin
    case (lb)
      6'h3F: n783 = (is_ep2 ? 3'd2 : (n669 ? 3'd2 : (n711 ? ((arg2 == 5'h14) ? 3'd0 : 3'd1) : (n723 ? 3'd2 : 3'd2))));
      default: n783 = 3'd2;
    endcase
  end

  assign cps_ready = n18;
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
      pfx <= (flush_valid ? 103'd0 : (n1010 ? pfx : (n996 ? 103'd0 : (cps_xfer ? (n796 ? (is_prefix ? (n799 ? n792 : (pfx_order ? {n915[102:37], 1'b1, n915[35:0]} : n915)) : n792) : n792) : pfx))));
      state <= (flush_valid ? 2'd0 : (n1010 ? state : (n996 ? 2'd0 : (cps_xfer ? (n796 ? (is_prefix ? state : (n782 ? ((n783 == 3'd2) ? 2'd1 : 2'd2) : state)) : (n946 ? 2'd2 : 2'd0)) : state))));
      llc_dst <= (flush_valid ? 5'd0 : (n1010 ? llc_dst : (cps_xfer ? (n796 ? (is_prefix ? llc_dst : (n782 ? arg1 : llc_dst)) : llc_dst) : llc_dst)));
      llc_kind <= (flush_valid ? 3'd2 : (n1010 ? llc_kind : (cps_xfer ? (n796 ? (is_prefix ? llc_kind : (n782 ? n783 : llc_kind)) : llc_kind) : llc_kind)));
      llc_hi <= (flush_valid ? 16'd0 : (n1010 ? llc_hi : (cps_xfer ? (n796 ? llc_hi : (n946 ? cps_data : llc_hi)) : llc_hi)));
      uop_busy <= (n1027 ? 1'b1 : (uop_ready ? 1'b0 : uop_busy));
      uop_hold <= (n1027 ? {n995[126:32], (bytes_held + 32'd2)} : uop_hold);
    end
  end

endmodule
