// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/bram_lookup.ddl -o examples/bram_lookup.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module bram_lookup (
    input         clk,
    input         rst_n,
    input         req_valid,
    output        req_ready,
    input  [7:0]  req_data,
    output        resp_valid,
    input         resp_ready,
    output [31:0] resp_data
);

  reg [1:0] state;
  reg [7:0] a_r;

  reg [31:0] t [0:255];
  reg [31:0] t_q;

  wire in_s0 = state == 2'd0;
  wire in_s1 = state == 2'd1;
  wire in_s2 = state == 2'd2;
  wire fire_s0 = in_s0 & req_valid;
  wire fire_s2 = in_s2 & resp_ready;

  assign req_ready = in_s0;
  assign resp_valid = in_s2;
  assign resp_data = t_q;

  always @(posedge clk) begin
    if (!rst_n) begin
      state <= 2'd0;
      a_r <= 8'd0;
    end else begin
      state <= (((fire_s0 | in_s1) | fire_s2) ? (fire_s0 ? 2'd1 : (in_s1 ? 2'd2 : (fire_s2 ? 2'd0 : 2'd0))) : state);
      a_r <= (fire_s0 ? req_data : a_r);
    end
  end

  // t [0:255] -- block RAM: one sync write port, sync reads
  always @(posedge clk) begin
    if (in_s1) t_q <= t[a_r];
  end

endmodule
