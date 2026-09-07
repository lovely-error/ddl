`timescale 1ns/1ps
module tb_sequence;
  reg clk=0; always #5 clk=~clk;
  reg rst_n=0;
  reg [1:0] ws=0;
  reg [31:0] input_data=0;
  wire [1:0] sr,sw,ar,aw,mr,mw,vr,vw,lr,lw;
  wire [31:0] sd,ad,md,vd,ld;
  seq_scope s(clk,rst_n,ws,sr,input_data,sw,2'b00,sd);
  seq_address a(clk,rst_n,ws,ar,input_data,aw,2'b00,ad);
  seq_read_first m(clk,rst_n,ws,mr,8'd0,mw,2'b00,md);
  seq_assert v(clk,rst_n,ws,vr,input_data,vw,2'b00,vd);
  seq_literal l(clk,rst_n,ws,lr,input_data,lw,2'b00,ld);
  initial begin
    repeat(3) @(negedge clk);
    a.mem[6]=16'd123; a.mem[2]=16'd456; m.mem[0]=16'd7;
    rst_n=1;
    // Empty stages contain zero: execution assertions must remain quiet.
    repeat(6) @(negedge clk);
    input_data=32'h00020006; ws=3;
    repeat(20) @(negedge clk);
    if(sw !== 3 || sd !== 32'h000a000a) $fatal(1,"lexical shadowing");
    if(aw !== 3 || ad !== 32'h01c8007b) $fatal(1,"same-name BRAM address and result");
    if(mw !== 3 || md !== 32'h002a0007) $fatal(1,"read-first source order");
    if(vw !== 3 || vd !== input_data) $fatal(1,"assertion pipeline transfer");
    if(lw !== 3 || ld !== 32'h002a002a) $fatal(1,"contextual send literal");
    $display("TB_PASS: sequence scoping, BRAM identities, assertions, literals, read-first RTL");
    $finish;
  end
endmodule
