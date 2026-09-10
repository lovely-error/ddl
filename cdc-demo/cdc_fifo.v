// Arm E's positive control: a correct asynchronous FIFO.
//
// Transcribed from KAMASUTRA2G/rtl/k2g_cdc_fifo.sv with WIDTH=16 and DEPTH=8
// folded in, and rewritten from SystemVerilog into the Verilog-2001 subset the
// rest of this build uses. The `logic`/`always_ff`/`$clog2` forms are gone;
// nothing else about it is changed. That file's header is the argument for
// every line here, and the two rules it states are the ones this demo exists
// to test the absence of:
//
//   * pointers are GRAY CODED, so a pointer sampled mid-transition is the old
//     value or the new one and never a mixture;
//   * every pointer crossing a domain passes through TWO flip-flops in the
//     receiving domain before it is used.
//
// It is here to answer, on this board and before the compiler is taught to
// emit anything, whether a depth-8 crossing streams at full rate with zero
// errors. If it does not, Phase 1 is building the wrong thing.
//
// `syn_preserve` on the synchronizer stages, for the reason the original
// records: without it GowinSynthesis may duplicate a stage to ease fan-out,
// and a duplicated synchronizer flop is two flops that can resolve differently
// from one metastable input -- the failure the chain exists to prevent.
//
// `wfull`/`rempty` are REGISTERED here, and that is the original's choice, not
// a correctness requirement: they are computed from `*_next`, so a
// combinational version would close the loop full -> push -> bin_next ->
// gray_next -> full. DDL's salt compares the CURRENT pointers instead and has
// no such loop; Phase 1 keeps the salt form, which is one cycle shorter in
// each direction.

module cdc_fifo (
    // Write side.
    input             wclk,
    input             wrst_n,
    input             wpush,
    input      [15:0] wdata,
    output reg        wfull,

    // Read side.
    input             rclk,
    input             rrst_n,
    input             rpop,
    output     [15:0] rdata,
    output reg        rempty
);

  reg [15:0] mem [0:7];

  reg  [3:0] wbin, wgray;
  reg  [3:0] rbin, rgray;

  reg  [3:0] wgray_meta /* synthesis syn_preserve = 1 */;
  reg  [3:0] wgray_sync /* synthesis syn_preserve = 1 */;
  reg  [3:0] rgray_meta /* synthesis syn_preserve = 1 */;
  reg  [3:0] rgray_sync /* synthesis syn_preserve = 1 */;

  wire wdo = wpush && !wfull;
  wire rdo = rpop  && !rempty;

  wire [3:0] wbin_next  = wbin + {3'b000, wdo};
  wire [3:0] wgray_next = wbin_next ^ {1'b0, wbin_next[3:1]};
  wire [3:0] rbin_next  = rbin + {3'b000, rdo};
  wire [3:0] rgray_next = rbin_next ^ {1'b0, rbin_next[3:1]};

  // ---- write side --------------------------------------------------------
  always @(posedge wclk) begin
    if (!wrst_n) begin
      wbin  <= 4'd0;
      wgray <= 4'd0;
    end else begin
      wbin  <= wbin_next;
      wgray <= wgray_next;
    end
  end

  always @(posedge wclk) begin
    if (wdo) mem[wbin[2:0]] <= wdata;
  end

  always @(posedge wclk) begin
    if (!wrst_n) begin
      rgray_meta <= 4'd0;
      rgray_sync <= 4'd0;
    end else begin
      rgray_meta <= rgray;
      rgray_sync <= rgray_meta;
    end
  end

  // Full: pointers equal except for the top two bits inverted, which is what
  // "one lap ahead" looks like in gray code.
  wire wfull_next = (wgray_next == {~rgray_sync[3:2], rgray_sync[1:0]});

  always @(posedge wclk) begin
    if (!wrst_n) wfull <= 1'b0;
    else         wfull <= wfull_next;
  end

  // ---- read side ---------------------------------------------------------
  always @(posedge rclk) begin
    if (!rrst_n) begin
      rbin  <= 4'd0;
      rgray <= 4'd0;
    end else begin
      rbin  <= rbin_next;
      rgray <= rgray_next;
    end
  end

  always @(posedge rclk) begin
    if (!rrst_n) begin
      wgray_meta <= 4'd0;
      wgray_sync <= 4'd0;
    end else begin
      wgray_meta <= wgray;
      wgray_sync <= wgray_meta;
    end
  end

  wire rempty_next = (rgray_next == wgray_sync);

  always @(posedge rclk) begin
    if (!rrst_n) rempty <= 1'b1;
    else         rempty <= rempty_next;
  end

  // Read combinationally at an address that crossed as a synchronized pointer,
  // so the entry has been stable for at least two receiving-domain edges.
  assign rdata = mem[rbin[2:0]];

endmodule
