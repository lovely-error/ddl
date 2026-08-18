// Hand-written reference for k3g_stage, in the exact style of k3g_expand.sv.
//
// The DDL version writes only the field permutation; this writes the handshake
// too. If the two agree cycle for cycle under arbitrary backpressure, the
// generated handshake is the one a person would have written.
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

  logic        busy;
  logic [48:0] out_item;

  // `valid` is a function of this stage's own state and never of `ready`.
  assign uops_valid = busy;
  assign uops_data  = out_item;
  assign iops_ready = !busy || uops_ready;

  wire logic up_xfer   = iops_valid && iops_ready;
  wire logic down_xfer = uops_valid && uops_ready;

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      busy     <= 1'b0;
      out_item <= '0;
    end else begin
      if (up_xfer) begin
        out_item <= next_item;
        busy     <= 1'b1;
      end else if (down_xfer) begin
        busy     <= 1'b0;
      end
    end
  end

endmodule
