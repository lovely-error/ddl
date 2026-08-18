// Hand-written reference for mul3: the three-stage pipeline a person would
// write, with a validity chain and a global shift enable.
`timescale 1ns/1ps

module mul3_ref (
    input  logic        clk,
    input  logic        rst_n,

    input  logic        src_valid,
    output logic        src_ready,
    input  logic [15:0] src_data,

    output logic        dst_valid,
    input  logic        dst_ready,
    output logic [31:0] dst_data
);

  logic        v0, v1, v2;
  logic [15:0] doubled_q;
  logic [31:0] wide_q, out_q;

  // The pipeline moves when whatever is leaving it can be taken.
  wire logic shift = !v2 || dst_ready;

  wire logic [15:0] doubled = src_data + src_data;
  wire logic [31:0] wide    = {16'd0, doubled_q};
  wire logic [31:0] scaled  = wide_q + wide_q;

  assign src_ready = shift;
  assign dst_valid = v2;
  assign dst_data  = out_q;

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0; v1 <= 1'b0; v2 <= 1'b0;
      doubled_q <= '0;
      wide_q    <= '0;
      out_q     <= '0;
    end else if (shift) begin
      v0 <= src_valid;
      v1 <= v0;
      v2 <= v1;
      doubled_q <= doubled;
      wide_q    <= wide;
      out_q     <= scaled;
    end
  end

endmodule
