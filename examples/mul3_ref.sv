// Hand-written reference for mul3: the three-stage pipeline a person would
// write, with a validity chain, a shift enable, and a skid entry behind the
// output so `src_ready` is a register.
//
// The skid is the part that is easy to leave out and expensive to leave out.
// Without it `shift` is `!v2 || dst_ready` and `src_ready` is `shift`, so the
// producer's `ready` is a wire straight through to the consumer's -- and a
// chain of these is one combinational path as long as the chain. k3g_chan.sv
// puts it as a rule: `ready` is "never a function of the opposite side's
// handshake". The second entry is where the item the producer had already
// committed to goes while the head is stalled, which is what lets `ready` stop
// watching `dst_ready`.
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

  // The second output entry.
  logic        skid_full;
  logic [31:0] skid_q;

  // The pipeline moves when there is somewhere for what leaves it to go.
  wire logic shift = !skid_full;

  wire logic [15:0] doubled = src_data + src_data;
  wire logic [31:0] wide    = {16'd0, doubled_q};
  wire logic [31:0] scaled  = wide_q + wide_q;

  // Nothing leaves the last stage on a cycle the pipeline does not shift, so
  // the offer is gated on `shift` and not merely on `v1`.
  wire logic pop       = v2 && dst_ready;
  wire logic offer     = shift && v1;
  wire logic keep_head = v2 && !pop;
  wire logic to_head   = offer && (!v2 || pop);
  wire logic from_skid = pop && skid_full;
  wire logic to_skid   = offer && keep_head;

  assign src_ready = shift;
  assign dst_valid = v2;
  assign dst_data  = out_q;

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      v0 <= 1'b0; v1 <= 1'b0; v2 <= 1'b0;
      doubled_q <= '0;
      wide_q    <= '0;
      out_q     <= '0;
      skid_full <= 1'b0;
      skid_q    <= '0;
    end else begin
      if (shift) begin
        v0 <= src_valid;
        v1 <= v0;
        doubled_q <= doubled;
        wide_q    <= wide;
      end

      // `from_skid` and `to_head` cannot both hold: an offer needs an empty
      // skid, and `from_skid` needs a full one.
      v2 <= keep_head || from_skid || to_head;
      if (from_skid)   out_q <= skid_q;
      else if (to_head) out_q <= scaled;

      skid_full <= (skid_full && !pop) || to_skid;
      if (to_skid) skid_q <= scaled;
    end
  end

endmodule
