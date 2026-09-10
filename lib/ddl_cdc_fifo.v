// An asynchronous FIFO, for crossing a clock domain at a DDL boundary.
//
// DDL COMPILES TO ONE CLOCK DOMAIN. Every face the compiler presents -- the
// Show-Ahead FIFO on an `--export` target, the mirror of it on an `extern`, and
// the raw salt ports of `--bare-export` -- is synchronous to that module's
// `clk`. Wiring any of them to logic on another clock is broken, and it is
// broken silently: it compiles, it lints, and it simulates perfectly, because
// Verilog samples old-or-new and models no metastability.
//
// It is not broken subtly. Measured on a Tang Nano 9K at 81 -> 108 MHz, with
// the compiler's own emitted link across the boundary:
//
//   two clocks, no FIFO      100% of items corrupt, 54,000,000 errors/second
//   two clocks, this module  0 errors in 1.14 billion items, 100% of the rate
//
// The full experiment, including the arm that fails and why, is in `cdc-demo/`.
// The reasoning that says a crossing needs this is in `docs/clock-domains.md`.
//
// PROVENANCE, and it is the point. This is not new code written for the
// occasion. It is KAMASUTRA2G/rtl/k2g_cdc_fifo.sv, transcribed into the
// Verilog-2001 subset, run on hardware as arm E of `cdc-demo/`, and shipped
// with its two parameters restored. If you change it, the hardware evidence no
// longer applies to what you are running -- rebuild `cdc-demo/` arm E and
// measure again, or do not change it.
//
// WHAT MAKES IT CORRECT, in the order the failures come:
//
//   * pointers are GRAY CODED, so exactly one bit changes per increment and a
//     pointer sampled mid-transition is the old value or the new one, never a
//     mixture of the two;
//   * every pointer crossing a domain passes through TWO FLIP-FLOPS in the
//     receiving domain before anything uses it. This is the load-bearing part,
//     and not for the reason usually given: its first job is to make the flag a
//     function of registers in the RECEIVING domain, so that the several
//     registers it feeds cannot disagree about it within one cycle. Resolving
//     metastability is its second job. `cdc-demo/` measured the first;
//   * the storage never crosses. It is written in one domain and read
//     combinationally in the other at an address that has ALREADY crossed as a
//     synchronized pointer, so the entry has been stable for at least two
//     receiving-domain edges before it is read. That is also what makes the
//     read side Show-Ahead -- `rdata` is valid whenever `rempty` is low, with
//     no read latency;
//   * `.sdc`/`.xdc` MUST declare the two clocks asynchronous. An unconstrained
//     crossing passes timing analysis and fails on hardware; a crossing the
//     tool decides to time may be "closed" and then break when placement moves.
//     See docs/clock-domains.md for the per-tool spelling.
//
// AW RATHER THAN DEPTH, deliberately. Depth is 2**AW. `$clog2` in a
// part-select bound makes GowinSynthesis exit 1 with an empty log, as do width
// casts of the form `(AW+1)'(...)`, so neither appears here and the address
// width is given directly. AW must be at least 2 (four entries): the full test
// compares the top two pointer bits and needs an address bit beneath them, and
// at AW=1 the expression's `[AW-2:0]` is a reversed part-select that fails
// elaboration a long way from the instantiation that caused it.
//
// CHOOSING AW. The credit loop is about six cycles -- three in each direction,
// for a synchronizer pair plus the act that follows it -- so sustained
// throughput is roughly `depth / 6` of the slower clock, capped at 1. Measured
// in simulation across four clock ratios: depth 2 gives 0.333, depth 8 gives
// 1.000. **AW = 3 (depth 8) is the smallest that streams at the full rate of
// the slower clock at any ratio.** Smaller is correct but slower; larger buys
// buffering for bursts, at a flop per bit per entry.

module ddl_cdc_fifo #(
    parameter WIDTH = 32,
    parameter AW    = 3          // depth is 2**AW; AW >= 2
) (
    // Write side.
    input              wclk,
    input              wrst_n,
    input              wpush,     // ignored when wfull
    input  [WIDTH-1:0] wdata,
    output reg         wfull,

    // Read side. Show-Ahead: rdata is valid whenever rempty is low.
    input              rclk,
    input              rrst_n,
    input              rpop,      // ignored when rempty
    output [WIDTH-1:0] rdata,
    output reg         rempty
);

  localparam DEPTH = (1 << AW);

  reg [WIDTH-1:0] mem [0:DEPTH-1];

  // Binary and gray forms of each pointer, one bit wider than the address.
  // The extra bit is what distinguishes full from empty: equal pointers are
  // empty, pointers differing only in the top bit are full.
  reg [AW:0] wbin, wgray;
  reg [AW:0] rbin, rgray;

  // Each pointer after two flops in the other domain.
  //
  // THE ATTRIBUTES ARE A REQUEST, NOT A GUARANTEE, and every vendor spells the
  // request differently, so all of them are here and each tool ignores the
  // rest. Without one that the tool honours, synthesis may retime logic into
  // the chain or duplicate a stage to ease fan-out -- and a duplicated
  // synchronizer flop is two flops that can resolve to different values from
  // one metastable input, which is the failure the chain exists to prevent.
  //
  //   Vivado     ASYNC_REG         no merge/retime, AND places the pair close
  //   Quartus    altera_attribute, PRESERVE
  //   Yosys      keep
  //   Gowin,     syn_preserve      (Synplify heritage; SUG550 5.13)
  //   Lattice,
  //   Microchip
  //
  // ONLY VIVADO'S ALSO CONSTRAINS PLACEMENT. Everywhere else the two flops may
  // land far apart and the resolution time is whatever routing gives, which
  // shortens the settling window and lowers the MTBF. And the only way to know
  // an attribute was honoured is to grep the post-synthesis netlist for these
  // registers; `cdc-demo/build.sh` does exactly that for Gowin and is the
  // recipe to copy. Verified there and nowhere else so far.
  (* ASYNC_REG = "TRUE", keep = "true", PRESERVE = "TRUE",
     altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION FORCED_IF_ASYNCHRONOUS" *)
  reg [AW:0] wgray_meta /* synthesis syn_preserve = 1 */;
  (* ASYNC_REG = "TRUE", keep = "true", PRESERVE = "TRUE",
     altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION FORCED_IF_ASYNCHRONOUS" *)
  reg [AW:0] wgray_sync /* synthesis syn_preserve = 1 */;
  (* ASYNC_REG = "TRUE", keep = "true", PRESERVE = "TRUE",
     altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION FORCED_IF_ASYNCHRONOUS" *)
  reg [AW:0] rgray_meta /* synthesis syn_preserve = 1 */;
  (* ASYNC_REG = "TRUE", keep = "true", PRESERVE = "TRUE",
     altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION FORCED_IF_ASYNCHRONOUS" *)
  reg [AW:0] rgray_sync /* synthesis syn_preserve = 1 */;

  // Zero-extended concatenation rather than a width cast: `(AW+1)'(...)` makes
  // GowinSynthesis exit 1 with an empty log.
  wire        wdo        = wpush && !wfull;
  wire [AW:0] wbin_next  = wbin + {{AW{1'b0}}, wdo};
  wire [AW:0] wgray_next = wbin_next ^ {1'b0, wbin_next[AW:1]};

  wire        rdo        = rpop && !rempty;
  wire [AW:0] rbin_next  = rbin + {{AW{1'b0}}, rdo};
  wire [AW:0] rgray_next = rbin_next ^ {1'b0, rbin_next[AW:1]};

  // ---- write side --------------------------------------------------------
  always @(posedge wclk) begin
    if (!wrst_n) begin
      wbin  <= {(AW+1){1'b0}};
      wgray <= {(AW+1){1'b0}};
    end else begin
      wbin  <= wbin_next;
      wgray <= wgray_next;
    end
  end

  always @(posedge wclk) begin
    if (wdo) mem[wbin[AW-1:0]] <= wdata;
  end

  // The read pointer, brought across.
  always @(posedge wclk) begin
    if (!wrst_n) begin
      rgray_meta <= {(AW+1){1'b0}};
      rgray_sync <= {(AW+1){1'b0}};
    end else begin
      rgray_meta <= rgray;
      rgray_sync <= rgray_meta;
    end
  end

  // Full: pointers equal except for the top two bits inverted, which is what
  // "one lap ahead" looks like in gray code.
  //
  // REGISTERED, not combinational. `wfull` gates `wpush`, which feeds
  // `wbin_next` and so `wgray_next` -- computing full from that combinationally
  // closes the loop and the FIFO never accepts anything. The flop breaks it and
  // costs nothing: a full FIFO stays full until the reader moves, which takes
  // more than a cycle to observe across the crossing anyway. It does put the
  // increment, the gray conversion and the compare in one cycle, which is this
  // module's Fmax limit -- about 130 MHz on a GW1NR-9C at the slow corner.
  wire wfull_next = (wgray_next == {~rgray_sync[AW:AW-1], rgray_sync[AW-2:0]});

  always @(posedge wclk) begin
    if (!wrst_n) wfull <= 1'b0;
    else         wfull <= wfull_next;
  end

  // ---- read side ---------------------------------------------------------
  always @(posedge rclk) begin
    if (!rrst_n) begin
      rbin  <= {(AW+1){1'b0}};
      rgray <= {(AW+1){1'b0}};
    end else begin
      rbin  <= rbin_next;
      rgray <= rgray_next;
    end
  end

  always @(posedge rclk) begin
    if (!rrst_n) begin
      wgray_meta <= {(AW+1){1'b0}};
      wgray_sync <= {(AW+1){1'b0}};
    end else begin
      wgray_meta <= wgray;
      wgray_sync <= wgray_meta;
    end
  end

  // Registered for symmetry with `wfull`, and for the same reason: `rempty`
  // gates `rpop`, which feeds `rbin_next` and `rgray_next`.
  wire rempty_next = (rgray_next == wgray_sync);

  always @(posedge rclk) begin
    if (!rrst_n) rempty <= 1'b1;
    else         rempty <= rempty_next;
  end

  // Read combinationally at an address that crossed as a synchronized pointer,
  // so the entry has been stable for at least two receiving-domain edges.
  assign rdata = mem[rbin[AW-1:0]];

`ifdef SIMULATION
  initial begin
    if (AW < 2) begin
      $display("ddl_cdc_fifo: AW must be at least 2 (four entries)");
      $finish;
    end
  end

  // Overflow and underflow are silent corruption otherwise: a dropped item
  // becomes a response that never arrives, and the far side waits forever.
  always @(posedge wclk) begin
    if (wrst_n && wpush && wfull) $display("ddl_cdc_fifo: push while full");
  end
  always @(posedge rclk) begin
    if (rrst_n && rpop && rempty) $display("ddl_cdc_fifo: pop while empty");
  end
`endif

endmodule
