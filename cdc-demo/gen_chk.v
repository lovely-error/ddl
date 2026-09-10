// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build cdc-demo/gen_chk.ddl -o cdc-demo/gen_chk.v --bare-export gen,chk
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module gen (
    input         clk,
    input         rst_n,
    output [1:0]  o_wsalt,
    input  [1:0]  o_rsalt,
    output [31:0] o_data
);

  reg [15:0] n;
  reg state;
  reg [15:0] o_e0;
  reg [15:0] o_e1;
  reg [1:0] o_wsalt_q;

  wire o_full = o_wsalt_q == (~o_rsalt);
  wire o_widx = o_wsalt_q[0] ^ o_wsalt_q[1];
  wire in_s0 = state == 1'b0;
  wire fire_s0 = in_s0 & (!o_full);

  assign o_wsalt = o_wsalt_q;
  assign o_data = {o_e1, o_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      n <= 16'd0;
      state <= 1'b0;
      o_e0 <= 16'd0;
      o_e1 <= 16'd0;
      o_wsalt_q <= 2'd0;
    end else begin
      n <= (fire_s0 ? (n + 16'd1) : n);
      state <= (fire_s0 ? (fire_s0 ? 1'b0 : 1'b0) : state);
      o_e0 <= ((fire_s0 & (!o_widx)) ? n : o_e0);
      o_e1 <= ((fire_s0 & o_widx) ? n : o_e1);
      o_wsalt_q <= (fire_s0 ? (o_wsalt_q ^ (o_widx ? 2'd2 : 2'd1)) : o_wsalt_q);
    end
  end

endmodule

module chk (
    input         clk,
    input         rst_n,
    input  [1:0]  src_wsalt,
    output [1:0]  src_rsalt,
    input  [31:0] src_data,
    output [1:0]  dst_wsalt,
    input  [1:0]  dst_rsalt,
    output [31:0] dst_data
);

  reg state;
  reg [1:0] src_rsalt_q;
  reg [15:0] dst_e0;
  reg [15:0] dst_e1;
  reg [1:0] dst_wsalt_q;
  reg [15:0] v_r;

  wire src_empty = src_wsalt == src_rsalt_q;
  wire src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire [15:0] src_item = src_ridx ? src_data[31:16] : src_data[15:0];
  wire dst_full = dst_wsalt_q == (~dst_rsalt);
  wire dst_widx = dst_wsalt_q[0] ^ dst_wsalt_q[1];
  wire in_s0 = state == 1'b0;
  wire in_s1 = state == 1'b1;
  wire fire_s0 = in_s0 & (!src_empty);
  wire fire_s1 = in_s1 & (!dst_full);

  assign src_rsalt = src_rsalt_q;
  assign dst_wsalt = dst_wsalt_q;
  assign dst_data = {dst_e1, dst_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      state <= 1'b0;
      src_rsalt_q <= 2'd0;
      dst_e0 <= 16'd0;
      dst_e1 <= 16'd0;
      dst_wsalt_q <= 2'd0;
      v_r <= 16'd0;
    end else begin
      state <= ((fire_s0 | fire_s1) ? (fire_s0 ? 1'b1 : (fire_s1 ? 1'b0 : 1'b0)) : state);
      src_rsalt_q <= (fire_s0 ? (src_rsalt_q ^ (src_ridx ? 2'd2 : 2'd1)) : src_rsalt_q);
      dst_e0 <= ((fire_s1 & (!dst_widx)) ? v_r : dst_e0);
      dst_e1 <= ((fire_s1 & dst_widx) ? v_r : dst_e1);
      dst_wsalt_q <= (fire_s1 ? (dst_wsalt_q ^ (dst_widx ? 2'd2 : 2'd1)) : dst_wsalt_q);
      v_r <= (fire_s0 ? src_item : v_r);
    end
  end

endmodule
