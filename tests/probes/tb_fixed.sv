`timescale 1ns/1ps
module tb_fixed;
  reg scalar=0; reg [1:0] scalar_i=0;
  wire [7:0] scalar_sx,scalar_not_sx,collision_sum;
  wire scalar_t,scalar_s,scalar_d;
  reg scalar_other=0; wire scalar_cmp; wire [7:0] scalar_signed_sx;
  scalar_signed bsign(scalar,scalar_other,scalar_cmp,scalar_signed_sx);
  scalar_sext bsx(scalar,scalar_sx);
  scalar_expr bex(scalar,scalar_not_sx);
  scalar_trunc btr(scalar,scalar_t);
  scalar_slice bsl(scalar,scalar_s);
  scalar_dynamic bdy(scalar,scalar_i,scalar_d);
  name_collision bnames(8'd17,8'd29,8'd43,collision_sum);
  reg clk=0; always #5 clk=~clk;
  reg rst_n=0;
  reg [1:0] source_ws=0;
  wire [1:0] graph_r,graph_w; wire [15:0] graph_d;
  collision_graph bg(clk,rst_n,source_ws,graph_r,16'h0004,graph_w,2'b00,graph_d);
  wire [1:0] drain_r,drain_w,peek_r,once_r,once_w;
  wire [15:0] drain_d,once_d;
  wire drain_observed,drain_en,peek_observed,peek_en;
  polling_drain cd(clk,rst_n,source_ws,drain_r,16'h002a,drain_w,2'b11,drain_d,drain_observed,drain_en);
  polling_peek cp(clk,rst_n,source_ws,peek_r,16'h002a,peek_observed,peek_en);
  polling_once co(clk,rst_n,source_ws,once_r,16'h002a,once_w,2'b00,once_d);
  reg [1:0] fw_source=0,fw_read=0; reg [15:0] fw_pair=0;
  wire [1:0] fw_consumed,fw_written,fw_unused; wire [15:0] fw_data,fw_unused_data;
  polling_forward cf(clk,rst_n,fw_source,fw_consumed,fw_pair,fw_written,fw_read,fw_data,fw_unused,2'b11,fw_unused_data);
  integer fw_produced=0,fw_received=0;
  wire [1:0] p4_r,p4_dw,p4_ow; wire [15:0] p4_dd,p4_od;
  p4 u4(clk,rst_n,source_ws,p4_r,18'h00055,p4_dw,2'b11,p4_dd,p4_ow,2'b11,p4_od);
  wire [1:0] p1_ar,p1_br,p1_ow; wire [15:0] p1_od;
  p1 u1(clk,rst_n,source_ws,p1_ar,16'h002a,2'b00,p1_br,16'h0000,p1_ow,2'b00,p1_od);
  wire [1:0] p5a_sr,p5a_or,p5a_dw; wire [15:0] p5a_dd;
  p5a u5a(clk,rst_n,source_ws,p5a_sr,16'h002a,2'b00,p5a_or,16'h0000,p5a_dw,2'b00,p5a_dd);
  wire [1:0] p5b_sr,p5b_dw; wire [15:0] p5b_dd;
  p5b u5b(clk,rst_n,source_ws,p5b_sr,16'h002a,p5b_dw,2'b11,p5b_dd);
  wire [1:0] p5c_sr,p5c_or,p5c_dw; wire [15:0] p5c_dd;
  p5c u5c(clk,rst_n,source_ws,p5c_sr,16'h002a,source_ws,p5c_or,16'h0007,p5c_dw,2'b00,p5c_dd);
  reg [7:0] x=0; wire [64:0] packed_e; wire [7:0] roundtrip;
  pack_small packer(x,packed_e); unpack unpacker(packed_e,roundtrip);
  reg [31:0] v=0; reg [4:0] k=0; wire [31:0] sx; wire [3:0] trunc;
  p7a u7a(v,sx); p7b u7b(v,k,trunc);
  wire [1:0] p8_qr,p8_ow,p8_pw; wire [15:0] p8_od,p8_pd;
  p8 u8(clk,rst_n,source_ws,p8_qr,18'h001ff,p8_ow,2'b00,p8_od,p8_pw,2'b00,p8_pd);
  reg [1:0] r9=0; wire [1:0] w9; wire [7:0] d9;
  p9 u9(clk,rst_n,w9,r9,d9);
  reg [1:0] r10=0; wire [1:0] w10; wire [1:0] d10;
  p10 u10(clk,rst_n,w10,r10,d10);
  wire [1:0] p11_ir,p11_w; wire [15:0] p11_d;
  p11 u11(clk,rst_n,source_ws,p11_ir,16'h002a,p11_w,2'b00,p11_d);
  reg [1:0] local_ws=0,rl=0,rr=0; wire [1:0] local_sr,wl,rec_sr,wr;
  wire [15:0] dl,dr;
  lifetimes ul(clk,rst_n,local_ws,local_sr,16'h0401,wl,rl,dl);
  received_local ur(clk,rst_n,local_ws,rec_sr,16'h0203,wr,rr,dr);
  reg [1:0] ra=0; wire [1:0] arm_sr,wa; wire [31:0] da;
  arm_scopes ua(clk,rst_n,local_ws,arm_sr,34'h22468aa00,wa,ra,da);
  integer count9=0,count10=0,countl=0,countr=0,counta=0,entries10=0,cycles=0;
  integer j,sh; reg [31:0] shifted;
  reg [7:0] expected_l[0:11]; reg [7:0] expected_r[0:4];
  reg [15:0] expected_a[0:3];
  function automatic [1:0] advance(input [1:0] s);
    advance=s ^ ((^s) ? 2'b10 : 2'b01);
  endfunction
  always @(negedge clk) if(rst_n) begin
    cycles=cycles+1;
    if(cycles > 2 && (p4_r !== source_ws || p4_dw !== 0 || p4_ow !== 0))
      $fatal(1,"p4 stale drop depended on full outputs");
    if(peek_r !== 0 || fw_unused !== 0 || drain_w !== 0) $fatal(1,"unrequested transfer");
    if(cycles % 11 >= 5 && fw_written != fw_read) begin
      if(fw_received >= 64 || ((^fw_read) ? fw_data[15:8] : fw_data[7:0]) !== fw_received[7:0])
        $fatal(1,"polling forwarding duplicated or lost item %0d",fw_received);
      fw_received=fw_received+1; fw_read=advance(fw_read);
    end
    if(fw_produced < 64 && fw_source != (~fw_consumed) && cycles % 7 != 0) begin
      if(^fw_source) fw_pair[15:8]=fw_produced[7:0]; else fw_pair[7:0]=fw_produced[7:0];
      fw_source=advance(fw_source); fw_produced=fw_produced+1;
    end
    if(p1_br !== 0 || p5a_or !== 0) $fatal(1,"empty buffer consumed");
    if(p5b_dw !== 0 || p5b_dd !== 0) $fatal(1,"full buffer overwritten");
    if(p5c_dw !== 0) $fatal(1,"send executed after break");
    // Scope entry occurs once per eight accepted sends, even under stalls.
    if(u10.n == 8) entries10=entries10+1;
    if(cycles % 11 >= 5) begin
      if(w9 != r9) begin
        if(count9 >= 8 || ((^r9) ? d9[7:4] : d9[3:0]) !== count9[3:0])
          $fatal(1,"p9 sequence mismatch at %0d",count9);
        count9=count9+1; r9=advance(r9);
      end
      if(w10 != r10) begin
        if(((^r10) ? d10[1] : d10[0]) !== 1'b1) $fatal(1,"p10 bit changed");
        count10=count10+1; r10=advance(r10);
      end
      if(wl != rl) begin
        if(countl >= 12 || ((^rl) ? dl[15:8] : dl[7:0]) !== expected_l[countl])
          $fatal(1,"scope lifetime mismatch at %0d",countl);
        countl=countl+1; rl=advance(rl);
      end
      if(wr != rr) begin
        if(countr >= 5 || ((^rr) ? dr[15:8] : dr[7:0]) !== expected_r[countr])
          $fatal(1,"receive lifetime mismatch at %0d",countr);
        countr=countr+1; rr=advance(rr);
      end
      if(wa != ra) begin
        if(counta >= 4 || ((^ra) ? da[31:16] : da[15:0]) !== expected_a[counta])
          $fatal(1,"match scope mismatch at %0d",counta);
        counta=counta+1; ra=advance(ra);
      end
    end
  end
  initial begin
    for(j=0;j<2;j=j+1) begin
      scalar=j; scalar_i=0; #1;
      if(scalar_sx !== {8{scalar}} || scalar_not_sx !== {8{!scalar}} ||
         scalar_t !== scalar || scalar_s !== scalar || scalar_d !== scalar || collision_sum !== 8'd89)
        $fatal(1,"scalar operation or port collision failed");
      scalar_i=1; #1;
      if(scalar_d !== 1'bx) $fatal(1,"out of range scalar index lost X semantics");
      for(sh=0;sh<2;sh=sh+1) begin
        scalar_other=sh; #1;
        if(scalar_cmp !== (scalar < scalar_other) || scalar_signed_sx !== {8{scalar}})
          $fatal(1,"scalar selection lost its unsigned type");
      end
    end
    expected_l[0]=1; expected_l[1]=2; expected_l[2]=3;
    expected_l[3]=100; expected_l[4]=101; expected_l[5]=4; expected_l[6]=9;
    expected_l[7]=4; expected_l[8]=5; expected_l[9]=6; expected_l[10]=7; expected_l[11]=9;
    expected_r[0]=3; expected_r[1]=2; expected_r[2]=1; expected_r[3]=2; expected_r[4]=1;
    expected_a[0]=16'haa; expected_a[1]=16'hab; expected_a[2]=16'h1234; expected_a[3]=16'h1236;
    for(j=0;j<256;j=j+1) begin
      x=j; v=32'h9abcde00 | j;
      for(sh=0;sh<32;sh=sh+1) begin
        k=sh; #1;
        if(roundtrip !== x) $fatal(1,"enum roundtrip %0d",j);
        if(sx !== {{24{v[7]}},v[7:0]}) $fatal(1,"sign extension %0d",j);
        shifted=v >> k;
        if(trunc !== shifted[3:0]) $fatal(1,"truncation %0d %0d",j,sh);
      end
    end
    @(negedge clk); #1; rst_n=1; source_ws=1; local_ws=3;
    repeat(300) @(posedge clk);
    #2;
    if(count9 != 8 || countl != 12 || countr != 5 || counta != 4 || count10 < 16 || entries10 < 2)
      $fatal(1,"missing progress: p9=%0d local=%0d receive=%0d p10=%0d entries=%0d",count9,countl,countr,count10,entries10);
    if(p1_od[7:0] !== 42 || p11_d[7:0] !== 43 || p8_pd[7:0] !== 255)
      $fatal(1,"relay or dispatch data mismatch");
    if(graph_w !== 1 || graph_d[7:0] !== 7) $fatal(1,"colliding module names changed graph behavior");
    if(drain_r !== 1 || once_r !== 1 || once_w !== 1 || once_d[7:0] !== 42 || fw_received != 64)
      $fatal(1,"communication progress or one-shot execution failed");
    $display("TB_PASS: compiler probes, scalar/name edges, nested lifetimes, and 64 local transfers under backpressure");
    $finish;
  end
  initial begin #20000; $fatal(1,"TIMEOUT"); end
endmodule
