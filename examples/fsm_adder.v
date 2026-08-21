// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/fsm_adder.ddl -o examples/fsm_adder.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module fsm_adder (
    input         clk,
    input         rst_n,
    input  [1:0]  src_wsalt,
    output [1:0]  src_rsalt,
    input  [63:0] src_data,
    output [1:0]  dst_wsalt,
    input  [1:0]  dst_rsalt,
    output [63:0] dst_data
);

  reg [1:0] state;
  reg [1:0] src_rsalt_q;
  reg [31:0] dst_e0;
  reg [31:0] dst_e1;
  reg [1:0] dst_wsalt_q;
  reg [31:0] a_r;
  reg [31:0] b_r;

  wire src_empty = src_wsalt == src_rsalt_q;
  wire n7 = !src_empty;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [31:0] src_item = src_ridx ? src_data[63:32] : src_data[31:0];
  wire dst_full = dst_wsalt_q == (~dst_rsalt);
  wire dst_widx = dst_wsalt_q[0] ^ dst_wsalt_q[1];
  wire in_s0 = state == 2'd0;
  wire in_s1 = state == 2'd1;
  wire in_s2 = state == 2'd2;
  wire fire_s0 = in_s0 & n7;
  wire fire_s1 = in_s1 & n7;
  wire fire_s2 = in_s2 & (!dst_full);
  wire [31:0] n39 = a_r + b_r;
  wire src_take = fire_s0 | fire_s1;

  assign src_rsalt = src_rsalt_q;
  assign dst_wsalt = dst_wsalt_q;
  assign dst_data = {dst_e1, dst_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      state <= 2'd0;
      src_rsalt_q <= 2'd0;
      dst_e0 <= 32'd0;
      dst_e1 <= 32'd0;
      dst_wsalt_q <= 2'd0;
      a_r <= 32'd0;
      b_r <= 32'd0;
    end else begin
      state <= (((fire_s0 | fire_s1) | fire_s2) ? (fire_s0 ? 2'd1 : (fire_s1 ? 2'd2 : (fire_s2 ? 2'd0 : 2'd0))) : state);
      src_rsalt_q <= (src_take ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      dst_e0 <= ((fire_s2 & (!dst_widx)) ? n39 : dst_e0);
      dst_e1 <= ((fire_s2 & dst_widx) ? n39 : dst_e1);
      dst_wsalt_q <= (fire_s2 ? (dst_wsalt_q ^ (dst_widx ? 2'd2 : 2'd1)) : dst_wsalt_q);
      a_r <= (fire_s0 ? src_item : a_r);
      b_r <= (fire_s1 ? src_item : b_r);
    end
  end

endmodule
