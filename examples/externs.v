// Black-box stubs for the `extern` modules the examples instantiate.
//
// An `extern` is a module DDL did not compile: the Verilog is hand-written and
// lives outside the program, so no file the compiler emits will contain it.
// The lint gate in tests/lint.rs still needs one, because a pipe into a module
// that is not there has nothing driving the flag coming back -- and UNDRIVEN
// is a real defect class for generated Verilog, worth keeping switched on
// rather than waiving away for the one file that trips it.
//
// A stub, not an implementation: it accepts everything and computes nothing.
// The interface has to match the `extern` declaration, and examples/fanout.ddl
// is where that is written.
//
// WHAT THIS FILE IS EVIDENCE OF. It used to speak the salt protocol, and
// accepting one `u32` and dropping it took a gray-code read pointer, a
// two-entry data port to index into, and a comment explaining why the reset
// was synchronous. The compiler now puts an adapter on its side of the
// boundary, so what is left here is `!full` tied high.

// `extern sink_ext (a: buffer in u32)` -- examples/fanout.ddl:27
module sink_ext (
  input         clk,
  input         rst_n,
  output        a_can_receive,
  input         a_receive_en,
  input  [31:0] a_data_write_in
);
  // Always ready, so the producer never stalls and every item is accepted and
  // dropped on the floor.
  assign a_can_receive = 1'b1;

  wire _unused = &{1'b0, clk, rst_n, a_receive_en, a_data_write_in, 1'b0};
endmodule
