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

    input  logic        iops_valid,
    output logic        iops_ready,
    input  logic [31:0] iops_data,

    output logic        uops_valid,
    input  logic        uops_ready,
    output logic [48:0] uops_data
);

  // in_item_t: epoch[2:0] kind[2:0] dst[4:0] src[4:0] imm[15:0], first field
  // in the high bits.
  wire logic [2:0]  in_epoch = iops_data[31:29];
  wire logic [2:0]  in_kind  = iops_data[28:26];
  wire logic [4:0]  in_dst   = iops_data[25:21];
  wire logic [4:0]  in_src   = iops_data[20:16];
  wire logic [15:0] in_imm   = iops_data[15:0];

  localparam logic [2:0] K_LOAD  = 3'd3;
  localparam logic [2:0] K_STORE = 3'd4;

  wire logic is_mem = (in_kind == K_LOAD) || (in_kind == K_STORE);

  // out_item_t: epoch[2:0] kind[2:0] dst[4:0] src[4:0] imm[31:0] is_mem
  wire logic [48:0] next_item =
      {in_epoch, in_kind, in_dst, in_src, {16'd0, in_imm}, is_mem};

  logic        head_full, skid_full;
  logic [48:0] head, skid;

  // Both handshake outputs are register outputs. Neither looks at the other
  // side's.
  assign uops_valid = head_full;
  assign uops_data  = head;
  assign iops_ready = !skid_full;

  wire logic up_xfer = iops_valid && iops_ready;
  wire logic pop     = head_full && uops_ready;

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      head_full <= 1'b0;
      skid_full <= 1'b0;
      head      <= '0;
      skid      <= '0;
    end else begin
      // The head drains first, pulling the skid down behind it.
      if (pop) begin
        if (skid_full) begin
          head      <= skid;
          skid_full <= 1'b0;
        end else begin
          head_full <= 1'b0;
        end
      end

      // An arriving item goes to the head if the head is free or freeing this
      // cycle, and to the skid otherwise. It can never find the skid occupied,
      // because that is exactly what `iops_ready` refused.
      if (up_xfer) begin
        if (!head_full || pop) begin
          head      <= next_item;
          head_full <= 1'b1;
        end else begin
          skid      <= next_item;
          skid_full <= 1'b1;
        end
      end
    end
  end

endmodule
