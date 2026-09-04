// Black-box stubs for the `extern` modules the examples instantiate.
//
// An `extern` is a module DDL did not compile: the Verilog is hand-written and
// lives outside the program, so no file the compiler emits will contain it.
// The lint gate in tests/lint.rs still needs one, because a pipe into a module
// that is not there has nothing driving the `rsalt` coming back -- and UNDRIVEN
// is a real defect class for generated Verilog, worth keeping switched on
// rather than waiving away for the one file that trips it.
//
// A stub, not an implementation: it accepts everything and computes nothing.
// The interface has to match the `extern` declaration, and examples/fanout.ddl
// is where that is written.

// `extern sink_ext (a: buffer in u32)` -- examples/fanout.ddl:27
module sink_ext (
  input         clk,
  input         rst_n,
  input  [1:0]  a_wsalt,
  output [1:0]  a_rsalt,
  input  [63:0] a_data
);
  // Always ready: the read salt chases the write salt, so the producer never
  // stalls and every item is accepted and dropped on the floor.
  //
  // Synchronous reset, because that is what the compiler emits -- a design
  // that flops one net both ways is a real defect, and a stub that disagreed
  // with the generated code would report it as one.
  reg [1:0] rsalt_q;
  always @(posedge clk) begin
    if (!rst_n) rsalt_q <= 2'd0;
    else        rsalt_q <= a_wsalt;
  end
  assign a_rsalt = rsalt_q;

  wire _unused = &{1'b0, a_data, 1'b0};
endmodule
