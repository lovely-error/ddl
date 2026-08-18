// Hand-written reference for fsm_adder: the state machine a person would
// write for "take two, add, hand on".
//
// The DDL version writes only the sequence. If the two agree cycle for cycle
// under arbitrary backpressure, the generated schedule is the one a person
// would have produced.
`timescale 1ns/1ps

module fsm_adder_ref (
    input  logic        clk,
    input  logic        rst_n,

    input  logic        src_valid,
    output logic        src_ready,
    input  logic [31:0] src_data,

    output logic        dst_valid,
    input  logic        dst_ready,
    output logic [31:0] dst_data
);

  typedef enum logic [1:0] { S_A, S_B, S_OUT } state_e;
  state_e state;

  logic [31:0] a_r, b_r;

  // `ready` may depend on anything; `valid` is a function of state only.
  assign src_ready = (state == S_A) || (state == S_B);
  assign dst_valid = (state == S_OUT);
  assign dst_data  = a_r + b_r;

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      state <= S_A;
      a_r   <= '0;
      b_r   <= '0;
    end else begin
      unique case (state)
        S_A: if (src_valid) begin
          a_r   <= src_data;
          state <= S_B;
        end
        S_B: if (src_valid) begin
          b_r   <= src_data;
          state <= S_OUT;
        end
        default: if (dst_ready) begin
          state <= S_A;
        end
      endcase
    end
  end

endmodule
