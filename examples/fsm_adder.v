// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/fsm_adder.ddl -o examples/fsm_adder.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module fsm_adder (
    input         clk,
    input         rst_n,
    input         src_valid,
    output        src_ready,
    input  [31:0] src_data,
    output        dst_valid,
    input         dst_ready,
    output [31:0] dst_data
);

  reg [1:0] state;
  reg [31:0] a_r;
  reg [31:0] b_r;

  wire in_s0 = state == 2'd0;
  wire in_s1 = state == 2'd1;
  wire in_s2 = state == 2'd2;
  wire fire_s0 = in_s0 & src_valid;
  wire fire_s1 = in_s1 & src_valid;
  wire fire_s2 = in_s2 & dst_ready;

  assign src_ready = (in_s0 | in_s1);
  assign dst_valid = in_s2;
  assign dst_data = (a_r + b_r);

  always @(posedge clk) begin
    if (!rst_n) begin
      state <= 2'd0;
      a_r <= 32'd0;
      b_r <= 32'd0;
    end else begin
      state <= (((fire_s0 | fire_s1) | fire_s2) ? (fire_s0 ? 2'd1 : (fire_s1 ? 2'd2 : (fire_s2 ? 2'd0 : 2'd0))) : state);
      a_r <= (fire_s0 ? src_data : a_r);
      b_r <= (fire_s1 ? src_data : b_r);
    end
  end

endmodule
