`timescale 1ns/1ps
module tb_adversarial;
  reg clk=0; always #5 clk=~clk;
  reg rst_n=0; reg [1:0] ws=0,other_ws=0;
  wire [1:0] c1r,c2r,ir,o1w,o2w,fr,orr,fw,br,bw;
  wire [15:0] o1d,o2d,fd,bd;
  joined_if j(clk,rst_n,ws,c1r,2'b00,ws,c2r,2'b01,ws,ir,16'd42,o1w,2'b00,o1d,o2w,2'b00,o2d);
  for_read2 f(clk,rst_n,2'b11,fr,16'h0703,other_ws,orr,16'd0,fw,2'b00,fd);
  store_and_load b(clk,rst_n,ws,br,16'd9,bw,2'b00,bd);
  wire [1:0] sw; wire [15:0] sd;
  store_before_read sb(clk,rst_n,sw,2'b00,sd);
  wire [1:0] cw,mr,mw,pr; wire [15:0] cd,md; wire [7:0] pd; wire pe;
  conditional_forward cb(clk,rst_n,cw,2'b00,cd);
  exclusive_match em(clk,rst_n,2'b11,mr,2'b10,mw,2'b00,md);
  exclusive_port ep(clk,rst_n,2'b11,pr,2'b10,pd,pe);
  integer port_count=0;
  always @(posedge clk) if(rst_n && pe) begin
    if(pd !== (port_count == 0 ? 8'd2 : 8'd1) || port_count > 1)
      $fatal(1,"exclusive port lost or duplicated payload");
    port_count++;
  end
  integer errors=0;
  initial begin
    repeat(3) @(negedge clk); rst_n=1; ws=1;
    repeat(5) @(negedge clk); other_ws=1;
    repeat(40) @(negedge clk);
    if(o2w !== 1 || o2d[7:0] !== 1) begin
      $display("MISMATCH join: salt=%h value=%h",o2w,o2d[7:0]); errors++;
    end
    if(fw !== 1 || fd[7:0] !== 12) begin
      $display("MISMATCH for crossing: salt=%h value=%h",fw,fd[7:0]); errors++;
    end
    if(bw !== 1 || bd[7:0] !== 9) begin
      $display("MISMATCH bram: salt=%h value=%h",bw,bd[7:0]); errors++;
    end
    if(sw !== 3 || sd !== 16'h0909) begin
      $display("MISMATCH same-state bram: salt=%h value=%h",sw,sd); errors++;
    end
    if(cw !== 3 || cd !== 16'h140a) begin
      $display("MISMATCH conditional forwarding: salt=%h value=%h",cw,cd); errors++;
    end
    if(mw !== 3 || md !== 16'h0102 || mr !== 3 || port_count != 2) begin
      $display("MISMATCH exclusive sends: salt=%h data=%h ports=%0d",mw,md,port_count); errors++;
    end
    if(errors) $fatal(1,"adversarial failures: %0d",errors);
    $display("TB_PASS: adversarial process joins, crossing reads and BRAM forwarding");
    $finish;
  end
endmodule
