// Hand-written reference for k3g_stage, in the exact style of k3g_expand.sv.
//
// The DDL version writes only the field permutation; this writes the handshake
// too. If the two agree cycle for cycle under arbitrary backpressure, the
// generated handshake is the one a person would have written.
//
// TWO ENTRIES, head and skid, which is what makes `ready` a register.
//
// A one-entry slot can only answer "I am empty, or my consumer is taking it
// this cycle" -- and that second clause is the consumer's `ready`, so the
// producer's `ready` becomes a wire straight through the module and a chain of
// N stages is one combinational path N modules long. k3g_chan.sv:193 refuses
// that outright with `assign up.ready = !full`, and its comment insists both
// sides be "register-derived, never a function of the opposite side's
// handshake". The skid entry is what buys that here: the producer learns about
// a stall one cycle late and has somewhere to put the item it had already
// committed to sending.
//
// Written procedurally, the way this is usually written by hand, rather than
// as the boolean next-state equations the compiler emits -- so agreement
// between the two is evidence rather than a transcription.
//
// Ports are the flat valid/ready/data triples rather than a k3g_chan
// interface, which is the flattening k3g_chan.sv:60 already pre-commits to.
`timescale 1ns/1ps

module k3g_stage_ref (
    input  logic        clk,
    input  logic        rst_n,

    // Two entries, a salt each way. `docs/attic/pipe.sv`'s `PipeCDC`, in one
    // clock domain -- which is where its missing synchronizers stop mattering.
    input  logic [1:0]  iops_wsalt,
    output logic [1:0]  iops_rsalt,
    input  logic [63:0] iops_data,

    output logic [1:0]  uops_wsalt,
    input  logic [1:0]  uops_rsalt,
    output logic [97:0] uops_data
);

  // ---- the consuming side --------------------------------------------------
  logic [1:0] iops_rsalt_q;
  wire logic  in_ridx  = iops_rsalt_q[0] ^ iops_rsalt_q[1];
  wire logic  in_empty = (iops_wsalt == iops_rsalt_q);
  wire logic [31:0] in_item = in_ridx ? iops_data[63:32] : iops_data[31:0];

  assign iops_rsalt = iops_rsalt_q;

  // in_item_t: epoch[2:0] kind[2:0] dst[4:0] src[4:0] imm[15:0], first field
  // in the high bits.
  wire logic [2:0]  in_epoch = in_item[31:29];
  wire logic [2:0]  in_kind  = in_item[28:26];
  wire logic [4:0]  in_dst   = in_item[25:21];
  wire logic [4:0]  in_src   = in_item[20:16];
  wire logic [15:0] in_imm   = in_item[15:0];

  localparam logic [2:0] K_LOAD  = 3'd3;
  localparam logic [2:0] K_STORE = 3'd4;

  wire logic is_mem = (in_kind == K_LOAD) || (in_kind == K_STORE);

  // out_item_t: epoch[2:0] kind[2:0] dst[4:0] src[4:0] imm[31:0] is_mem
  wire logic [48:0] next_item =
      {in_epoch, in_kind, in_dst, in_src, {16'd0, in_imm}, is_mem};

  // ---- the producing side --------------------------------------------------
  logic [1:0]  uops_wsalt_q;
  logic [48:0] e0, e1;
  wire logic   out_widx = uops_wsalt_q[0] ^ uops_wsalt_q[1];
  // Both bits inverted is one lap ahead in gray code, which for two entries is
  // full.
  wire logic   out_full = (uops_wsalt_q == ~uops_rsalt);

  assign uops_wsalt = uops_wsalt_q;
  assign uops_data  = {e1, e0};

  // One transfer, both ends of it. The stage takes an item only when it has
  // somewhere to put the result -- which is the same predicate the valid/ready
  // version wrote as `iops_ready = !skid_full`, arrived at differently.
  wire logic xfer = !in_empty && !out_full;

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      iops_rsalt_q <= 2'b00;
      uops_wsalt_q <= 2'b00;
      e0           <= '0;
      e1           <= '0;
    end else if (xfer) begin
      if (out_widx) e1 <= next_item; else e0 <= next_item;
      uops_wsalt_q[out_widx] <= ~uops_wsalt_q[out_widx];
      iops_rsalt_q[in_ridx]  <= ~iops_rsalt_q[in_ridx];
    end
  end

endmodule
