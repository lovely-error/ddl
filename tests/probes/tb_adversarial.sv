`timescale 1ns/1ps
//
// Connections are NAMED throughout. They were positional, and the interfaces
// moved underneath them: `exclusive_port` grew from an enable-and-payload pair
// to the salt protocol every other pipe here uses, and the seven positional
// arguments still described the old shape. Questa reported too few ports, two
// width mismatches and an unconnected `o_data`, and the bench then failed on a
// value it had never actually read. Positionally, that is one edit away from
// happening again; by name it is a compile error that says which port.
//
module tb_adversarial;
  reg clk=0; always #5 clk=~clk;
  reg rst_n=0; reg [1:0] ws=0,other_ws=0;
  wire [1:0] c1r,c2r,ir,o1w,o2w,fr,orr,fw,br,bw;
  wire [15:0] o1d,o2d,fd,bd;

  joined_if j(
    .clk(clk), .rst_n(rst_n),
    .c1_wsalt(ws),      .c1_rsalt(c1r), .c1_data(2'b00),
    .c2_wsalt(ws),      .c2_rsalt(c2r), .c2_data(2'b01),
    .i_wsalt(ws),       .i_rsalt(ir),   .i_data(16'd42),
    .o1_wsalt(o1w),     .o1_rsalt(2'b00), .o1_data(o1d),
    .o2_wsalt(o2w),     .o2_rsalt(2'b00), .o2_data(o2d)
  );

  for_read2 f(
    .clk(clk), .rst_n(rst_n),
    .src_wsalt(2'b11),      .src_rsalt(fr),  .src_data(16'h0703),
    .other_wsalt(other_ws), .other_rsalt(orr), .other_data(16'd0),
    .dst_wsalt(fw),         .dst_rsalt(2'b00), .dst_data(fd)
  );

  store_and_load b(
    .clk(clk), .rst_n(rst_n),
    .i_wsalt(ws), .i_rsalt(br),   .i_data(16'd9),
    .o_wsalt(bw), .o_rsalt(2'b00), .o_data(bd)
  );

  wire [1:0] sw; wire [15:0] sd;
  store_before_read sb(
    .clk(clk), .rst_n(rst_n),
    .o_wsalt(sw), .o_rsalt(2'b00), .o_data(sd)
  );

  wire [1:0] cw,mr,mw,pr,pw; wire [15:0] cd,md,pd;
  conditional_forward cb(
    .clk(clk), .rst_n(rst_n),
    .o_wsalt(cw), .o_rsalt(2'b00), .o_data(cd)
  );

  exclusive_match em(
    .clk(clk), .rst_n(rst_n),
    .c_wsalt(2'b11), .c_rsalt(mr),   .c_data(2'b10),
    .o_wsalt(mw),    .o_rsalt(2'b00), .o_data(md)
  );

  // Same interface as `exclusive_match`, and the same contract: two flags in,
  // `2` for the false one and `1` for the true one, so both arrive packed as
  // `0102`. The old bench watched a payload-and-enable pair that this process
  // has not presented since it became a buffer.
  exclusive_port ep(
    .clk(clk), .rst_n(rst_n),
    .c_wsalt(2'b11), .c_rsalt(pr),   .c_data(2'b10),
    .o_wsalt(pw),    .o_rsalt(2'b00), .o_data(pd)
  );

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
    if(mw !== 3 || md !== 16'h0102 || mr !== 3) begin
      $display("MISMATCH exclusive match: salt=%h data=%h rsalt=%h",mw,md,mr); errors++;
    end
    if(pw !== 3 || pd !== 16'h0102 || pr !== 3) begin
      $display("MISMATCH exclusive port: salt=%h data=%h rsalt=%h",pw,pd,pr); errors++;
    end
    if(errors) $fatal(1,"adversarial failures: %0d",errors);
    $display("TB_PASS: adversarial process joins, crossing reads and BRAM forwarding");
    $finish;
  end
endmodule
