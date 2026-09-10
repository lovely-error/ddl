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

  // ---- several pipes on one sequence --------------------------------------
  //
  // The head is a join and the tail a scatter, and the property neither the
  // Rust interpreter nor a golden file states well is the SIMULTANEITY: both
  // outputs are written on the same edge, from the same item, or neither is.
  reg [1:0] jaws=0, jbws=0;
  reg [31:0] ja_data=0, jb_data=0;
  wire [1:0] jar,jbr,jsw,jdw;
  wire [31:0] jsd,jdd;
  seq_join j(clk,rst_n,jaws,jar,ja_data,jbws,jbr,jb_data,
             jsw,2'b00,jsd,jdw,2'b00,jdd);

  // An optional input: `b` never offers anything, and the pipeline must run
  // anyway and report `ok` low by leaving `x` alone.
  reg [1:0] oaws=0;
  reg [31:0] oa_data=0;
  wire [1:0] oar,obr,oow;
  wire [31:0] ood;
  seq_optional o(clk,rst_n,oaws,oar,oa_data,2'b00,obr,32'd0,oow,2'b00,ood);

  // The same shape with the optional input actually delivering, and delivering
  // LESS than the blocking one: `a` carries two items and `b` one, so the pair
  // that fires first has its `ok` high and the second has it low. That the two
  // items get different answers is the whole of what `ok` means -- an `ok`
  // stuck high or low would pass a test where `b` was always full or always
  // empty, and this is the one it cannot pass.
  reg [1:0] paws=0, pbws=0;
  reg [31:0] pa_data=0, pb_data=0;
  wire [1:0] par,pbr,pow;
  wire [31:0] pod;
  seq_optional p(clk,rst_n,paws,par,pa_data,pbws,pbr,pb_data,pow,2'b00,pod);

  // Both sinks of the join advance together, every cycle, always.
  always @(posedge clk) begin
    if (rst_n && jsw !== jdw) $fatal(1,"join sinks moved apart: %b vs %b",jsw,jdw);
    if (rst_n && obr !== 2'b00) $fatal(1,"an empty optional input was consumed");
  end

  initial begin
    repeat(3) @(negedge clk);
    a.mem[6]=16'd123; a.mem[2]=16'd456; m.mem[0]=16'd7;
    rst_n=1;
    // Empty stages contain zero: execution assertions must remain quiet.
    repeat(6) @(negedge clk);
    input_data=32'h00020006; ws=3;

    // `a` carries 6 then 2; `b` carries 1 twice. The sums are 7 and 3, the
    // differences 5 and 1, and each pair belongs to one item.
    ja_data=32'h00020006; jb_data=32'h00010001;
    oa_data=32'h00020006;
    // `a` carries 6 then 2; `b` carries 1 and then has nothing more.
    pa_data=32'h00020006; pb_data=32'h00000001;
    repeat(4) @(negedge clk);
    // Held back one input to prove the join waits for it: nothing may leave.
    jaws=3;
    repeat(6) @(negedge clk);
    if(jsw !== 0 || jdw !== 0) $fatal(1,"the join fired without its second input");
    if(jar !== 0) $fatal(1,"the join consumed an input it could not use");
    jbws=3;
    // The optional input never arrives, so this one runs on `a` alone.
    oaws=3;
    // And here it arrives once: the first item is 6+1, the second is 2 alone.
    paws=3; pbws=1;

    repeat(20) @(negedge clk);
    if(sw !== 3 || sd !== 32'h000a000a) $fatal(1,"lexical shadowing");
    if(aw !== 3 || ad !== 32'h01c8007b) $fatal(1,"same-name BRAM address and result");
    if(mw !== 3 || md !== 32'h002a0007) $fatal(1,"read-first source order");
    if(vw !== 3 || vd !== input_data) $fatal(1,"assertion pipeline transfer");
    if(lw !== 3 || ld !== 32'h002a002a) $fatal(1,"contextual send literal");
    if(jsw !== 3 || jsd !== 32'h00030007) $fatal(1,"join sums: %h",jsd);
    if(jdw !== 3 || jdd !== 32'h00010005) $fatal(1,"join differences: %h",jdd);
    if(jar !== 3 || jbr !== 3) $fatal(1,"both inputs step together");
    if(oow !== 3 || ood !== 32'h00020006) $fatal(1,"optional input absent: %h",ood);
    if(pow !== 3 || pod !== 32'h00020007) $fatal(1,"optional input paired per item: %h",pod);
    if(pbr !== 1) $fatal(1,"the optional input gave up %b entries, not one",pbr);
    if(par !== 3) $fatal(1,"the blocking input ran to completion regardless");
    $display("TB_PASS: sequence scoping, BRAM identities, assertions, literals, read-first, join/scatter, optional input RTL");
    $finish;
  end
endmodule
