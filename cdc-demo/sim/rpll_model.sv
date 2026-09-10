// A behavioural stand-in for the GW1NR-9C's rPLL primitive.
//
// `k2g_pll.sv` instantiates the primitive directly rather than generating it
// through the IDE, so the arithmetic is reviewable -- but the primitive itself
// only exists in Gowin's `prim_sim.v`, which the sim scripts do not compile
// (rtl/sim/psram.sh is the one place that does, and it needs the whole 18000
// -instance library to do it). Without this, nothing that instantiates
// `k2g_pll` can be simulated at all, which would mean `k2g_soc` could only ever
// be tested by flashing it.
//
// It models the frequency and nothing else. The output period is derived by
// MEASURING the input rather than from the FCLKIN string parameter, so a
// testbench that runs the crystal at some convenient rate gets a memory clock
// in the right ratio without having to keep two numbers in step:
//
//   CLKOUT = CLKIN * (FBDIV_SEL+1) / (IDIV_SEL+1)
//
// Not modelled, and deliberately: phase, jitter, duty adjustment, the dynamic
// divider inputs, the secondary outputs, and any relationship between CLKIN
// and CLKOUT edges. A design that depended on the phase between them would
// pass here and fail on a board, which is exactly why every crossing in this
// repository goes through `k2g_cdc_fifo` and treats the two as unrelated.
//
// LOCK rises after LOCK_NS and stays high. The real part takes tens of
// microseconds; the number here is short enough not to dominate a simulation
// and long enough that a design which uses the clock before LOCK still sees a
// window in which it must not.

`timescale 1ns/1ps

module rPLL #(
    parameter FCLKIN            = "100.0",
    parameter DEVICE            = "GW1NR-9C",
    parameter integer IDIV_SEL  = 0,
    parameter integer FBDIV_SEL = 0,
    parameter integer ODIV_SEL  = 8,
    parameter DYN_IDIV_SEL      = "false",
    parameter DYN_FBDIV_SEL     = "false",
    parameter DYN_ODIV_SEL      = "false",
    parameter integer DYN_SDIV_SEL = 2,
    parameter PSDA_SEL          = "0000",
    parameter DYN_DA_EN         = "false",
    parameter DUTYDA_SEL        = "1000",
    parameter CLKOUT_FT_DIR     = 1'b1,
    parameter CLKOUTP_FT_DIR    = 1'b1,
    parameter integer CLKOUT_DLY_STEP  = 0,
    parameter integer CLKOUTP_DLY_STEP = 0,
    parameter CLKFB_SEL         = "internal",
    parameter CLKOUT_BYPASS     = "false",
    parameter CLKOUTP_BYPASS    = "false",
    parameter CLKOUTD_BYPASS    = "false",
    parameter CLKOUTD_SRC       = "CLKOUT",
    parameter CLKOUTD3_SRC      = "CLKOUT",

    // Simulation-only.
    parameter real LOCK_NS = 2000.0
) (
    output reg  CLKOUT,
    output reg  LOCK,
    output wire CLKOUTP,
    output wire CLKOUTD,
    output wire CLKOUTD3,

    input wire       RESET,
    input wire       RESET_P,
    input wire       CLKIN,
    input wire       CLKFB,
    input wire [5:0] FBDSEL,
    input wire [5:0] IDSEL,
    input wire [5:0] ODSEL,
    input wire [3:0] PSDA,
    input wire [3:0] DUTYDA,
    input wire [3:0] FDLY
);

  realtime e0, e1, out_half;

  initial begin
    CLKOUT = 1'b0;
    LOCK   = 1'b0;

    // Two edges to measure the input period, then run.
    @(posedge CLKIN);
    e0 = $realtime;
    @(posedge CLKIN);
    e1 = $realtime;

    out_half = ((e1 - e0) * (IDIV_SEL + 1)) / ((FBDIV_SEL + 1) * 2);

    fork
      forever #(out_half) CLKOUT = ~CLKOUT;
      begin
        #(LOCK_NS);
        LOCK = 1'b1;
      end
    join_none
  end

  assign CLKOUTP  = CLKOUT;
  assign CLKOUTD  = CLKOUT;
  assign CLKOUTD3 = CLKOUT;

  wire _unused = &{1'b0, RESET, RESET_P, CLKFB, FBDSEL, IDSEL, ODSEL,
                   PSDA, DUTYDA, FDLY};

endmodule
