// Hand-written reference for mul3: the three-stage pipeline a person would
// write, under the salt protocol.
//
// THE VALIDITY CHAIN IS ONE BIT SHORTER THAN THE PIPELINE, and that is the
// part worth getting right rather than transcribing. Under valid/ready the
// last stage needed its own `v2` beside the output slot's occupancy, and the
// two said the same thing twice. `wsalt` says it once: the pipeline is three
// deep and the chain is `v0`, `v1`, and then the salt distance to the
// consumer's.
//
// The other half of what disappeared is the skid shuffle. A producer here only
// ever pushes -- there is no pop, no skid-draining-into-head, and no second
// copy of the payload, because taking is the consumer's business and it does
// not need this side's help to do it.
`timescale 1ns/1ps

module mul3_ref (
    input  logic        clk,
    input  logic        rst_n,

    input  logic [1:0]  src_wsalt,
    output logic [1:0]  src_rsalt,
    input  logic [31:0] src_data,

    output logic [1:0]  dst_wsalt,
    input  logic [1:0]  dst_rsalt,
    output logic [63:0] dst_data
);

  // ---- the consuming side --------------------------------------------------
  logic [1:0] src_rsalt_q;
  wire logic  in_ridx  = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire logic  in_empty = (src_wsalt == src_rsalt_q);
  wire logic [15:0] in_item = in_ridx ? src_data[31:16] : src_data[15:0];

  assign src_rsalt = src_rsalt_q;

  // ---- the producing side --------------------------------------------------
  logic [1:0]  dst_wsalt_q;
  logic [31:0] e0, e1;
  wire logic   out_widx = dst_wsalt_q[0] ^ dst_wsalt_q[1];
  wire logic   out_full = (dst_wsalt_q == ~dst_rsalt);

  assign dst_wsalt = dst_wsalt_q;
  assign dst_data  = {e1, e0};

  // The pipeline moves when there is somewhere for what leaves it to go.
  wire logic shift = !out_full;

  logic        v0, v1;
  logic [15:0] doubled_q;
  logic [31:0] wide_q;

  wire logic [15:0] doubled = in_item + in_item;
  wire logic [31:0] wide    = {16'd0, doubled_q};
  wire logic [31:0] scaled  = wide_q + wide_q;

  // The input is taken when something is offered and the pipeline is moving.
  wire logic take = !in_empty && shift;
  // ...and the last stage's result is pushed on the same condition, one bit
  // further down the chain.
  wire logic push = shift && v1;

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0; v1 <= 1'b0;
      doubled_q   <= '0;
      wide_q      <= '0;
      e0          <= '0;
      e1          <= '0;
      src_rsalt_q <= 2'b00;
      dst_wsalt_q <= 2'b00;
    end else begin
      if (shift) begin
        v0        <= !in_empty;
        v1        <= v0;
        doubled_q <= doubled;
        wide_q    <= wide;
      end
      if (take) src_rsalt_q[in_ridx] <= ~src_rsalt_q[in_ridx];
      if (push) begin
        if (out_widx) e1 <= scaled; else e0 <= scaled;
        dst_wsalt_q[out_widx] <= ~dst_wsalt_q[out_widx];
      end
    end
  end

endmodule
