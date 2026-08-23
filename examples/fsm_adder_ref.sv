// Hand-written reference for fsm_adder: the state machine a person would
// write for "take two, add, hand on".
//
// The DDL version writes only the sequence. If the two agree cycle for cycle
// under arbitrary backpressure, the generated schedule is the one a person
// would have produced.
//
// THIS ONE GREW STORAGE IT DID NOT HAVE, and that is a real change rather than
// a rename. Under valid/ready the output was `assign dst_data = a_r + b_r`
// with `dst_valid = (state == S_OUT)` -- a state machine had no output
// register at all, and the sum was recomputed combinationally for as long as
// the consumer took to notice it.
//
// It cannot work that way now. The consumer reads whichever entry its own
// `rsalt` names, on a cycle this side may already have left S_OUT; so S_OUT
// has to PUSH the sum into an entry and leave it there. That is two entries
// and a salt of storage, and one more cycle before the consumer sees a result.
// Written out here rather than transcribed from the compiler, because "the
// schedule a person would have produced" is the whole claim being tested.
`timescale 1ns/1ps

module fsm_adder_ref (
    input  logic        clk,
    input  logic        rst_n,

    input  logic [1:0]  src_wsalt,
    output logic [1:0]  src_rsalt,
    input  logic [63:0] src_data,

    output logic [1:0]  dst_wsalt,
    input  logic [1:0]  dst_rsalt,
    output logic [63:0] dst_data
);

  typedef enum logic [1:0] { S_A, S_B, S_OUT } state_e;
  state_e state;

  logic [31:0] a_r, b_r;

  // ---- the consuming side --------------------------------------------------
  logic [1:0] src_rsalt_q;
  wire logic  in_ridx  = src_rsalt_q[0] ^ src_rsalt_q[1];
  wire logic  in_empty = (src_wsalt == src_rsalt_q);
  wire logic [31:0] in_item = in_ridx ? src_data[63:32] : src_data[31:0];

  assign src_rsalt = src_rsalt_q;

  // ---- the producing side --------------------------------------------------
  logic [1:0]  dst_wsalt_q;
  logic [31:0] e0, e1;
  wire logic   out_widx = dst_wsalt_q[0] ^ dst_wsalt_q[1];
  wire logic   out_full = (dst_wsalt_q == ~dst_rsalt);

  assign dst_wsalt = dst_wsalt_q;
  assign dst_data  = {e1, e0};

  // A state does its work on the cycle it can: a receiving state needs
  // something offered, the sending state needs somewhere to put the sum.
  wire logic take_a = (state == S_A)   && !in_empty;
  wire logic take_b = (state == S_B)   && !in_empty;
  wire logic put    = (state == S_OUT) && !out_full;

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      state       <= S_A;
      a_r         <= '0;
      b_r         <= '0;
      e0          <= '0;
      e1          <= '0;
      src_rsalt_q <= 2'b00;
      dst_wsalt_q <= 2'b00;
    end else begin
      unique case (state)
        S_A: if (take_a) begin
          a_r   <= in_item;
          state <= S_B;
        end
        S_B: if (take_b) begin
          b_r   <= in_item;
          state <= S_OUT;
        end
        default: if (put) begin
          state <= S_A;
        end
      endcase

      // Taking IS toggling; there is no separate `ready` to raise.
      if (take_a || take_b) src_rsalt_q[in_ridx] <= ~src_rsalt_q[in_ridx];
      if (put) begin
        if (out_widx) e1 <= a_r + b_r; else e0 <= a_r + b_r;
        dst_wsalt_q[out_widx] <= ~dst_wsalt_q[out_widx];
      end
    end
  end

endmodule
