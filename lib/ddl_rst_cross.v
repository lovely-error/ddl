// Carries one reset across a clock boundary, so both halves of a
// `ddl_cdc_fifo` leave zero together.
//
// WHY THIS EXISTS. A CDC FIFO's two halves share pointer state. Reset one and
// not the other and it reports occupancy that was never written, or space that
// is not there, and stays wrong forever -- the one failure that survives adding
// synchronizers. So both halves must be released from zero together, always.
//
// The obvious arrangement -- run `rst_n` through two flops on the foreign clock
// and use that -- does not achieve it. `rst_n` is synchronous to `clk`. If the
// foreign clock is much slower, a short pulse can fall entirely between two of
// its edges and be missed, at which point the core half resets and the foreign
// half does not. That is a single-side reset by another name, and it fails
// silently and permanently. Documenting a minimum pulse width instead of fixing
// it just moves the failure to whoever does not read the document.
//
// So the assertion is STRETCHED UNTIL THE FAR SIDE ANSWERS:
//
//   core     `resetting` is set by `!rst_n` and stays set -- however short the
//            pulse -- until the acknowledgement comes back AND `rst_n` is high
//            again;
//   foreign  two flops on `f_clk` see `resetting` and hold that half while it is
//            high; a THIRD flop then reports that the hold has actually been
//            applied, and that is what travels back;
//   core     two flops on `clk` see it return, and only then may `resetting`
//            clear.
//
// There is no minimum pulse width for the instantiator to know, at any clock
// ratio -- verified in simulation from 1:1 to 1:128 and at 8:1, with `rst_n`
// asserted for a single core cycle (cdc-demo/sim/tb_rst_cross.sv).
//
// Release order is core-then-foreign, which is harmless because both pointer
// sets are at zero: whichever half comes up first stalls on full or idles on
// empty until the other joins it.
//
// THE ONE COST, and it is the right side of the trade. If `f_clk` is not
// running -- PLL unlocked, clock gated, board still coming up -- the
// acknowledgement never returns, `resetting` stays set and that boundary is
// held. Nothing moves, loudly. But when the clock does start the answer comes
// back and both halves release from zero, so it SELF-HEALS; a plain two-flop
// synchronizer that had missed the pulse would come up desynchronised and stay
// wrong. The design that can hang is the one that recovers.
//
// If a crossed pipe never moves, check that its clock is running before
// suspecting anything else. See docs/clock-domains.md.

module ddl_rst_cross (
    // Core domain.
    input  clk,
    input  rst_n,
    // Foreign domain.
    input  f_clk,

    // Active-low resets for the two halves of the crossing.
    output w_rst_n,      // synchronous to clk
    output r_rst_n       // synchronous to f_clk
);

  // Sticky in the core domain: set by any assertion of `rst_n`, however brief.
  reg resetting;

  // The level travelling out, and the same level coming back. Attributes as in
  // ddl_cdc_fifo -- every vendor spells "do not optimise this flop" its own way
  // and ignores the others, and a duplicated synchronizer stage is two flops
  // that can resolve differently from one metastable input.
  (* ASYNC_REG = "TRUE", keep = "true", PRESERVE = "TRUE",
     altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION FORCED_IF_ASYNCHRONOUS" *)
  reg f_meta /* synthesis syn_preserve = 1 */;
  (* ASYNC_REG = "TRUE", keep = "true", PRESERVE = "TRUE",
     altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION FORCED_IF_ASYNCHRONOUS" *)
  reg f_sync /* synthesis syn_preserve = 1 */;
  // The acknowledgement is NOT `f_sync` itself, and the difference matters.
  // `f_sync` rises at the instant the foreign half begins to be held — before it
  // has executed its reset on any foreign edge. Acknowledging there releases the
  // core half while the foreign pointer is still whatever it was, and the core
  // half immediately samples it across. In silicon that is benign, because a
  // held pointer reads zero and flops power up at zero; in simulation it
  // propagates X, which is how it was found. Either way the property to want is
  // the stronger one: **the core half is not released until the foreign half has
  // actually applied its reset.** One more flop buys exactly that.
  reg f_applied;

  (* ASYNC_REG = "TRUE", keep = "true", PRESERVE = "TRUE",
     altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION FORCED_IF_ASYNCHRONOUS" *)
  reg ack_meta /* synthesis syn_preserve = 1 */;
  (* ASYNC_REG = "TRUE", keep = "true", PRESERVE = "TRUE",
     altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION FORCED_IF_ASYNCHRONOUS" *)
  reg ack_sync /* synthesis syn_preserve = 1 */;

  // Deliberately NOT reset by `rst_n`: this register is what remembers that
  // `rst_n` happened, so a reset of its own would erase the thing it is for.
  always @(posedge clk) begin
    if (!rst_n)          resetting <= 1'b1;
    else if (ack_sync)   resetting <= 1'b0;
  end

  always @(posedge f_clk) begin
    f_meta    <= resetting;
    f_sync    <= f_meta;
    f_applied <= f_sync;      // one foreign edge after the hold took effect
  end

  always @(posedge clk) begin
    ack_meta <= f_applied;
    ack_sync <= ack_meta;
  end

  // The core half is held while `rst_n` is low or the handshake is outstanding;
  // the foreign half while it can see the handshake.
  assign w_rst_n = !(!rst_n || resetting);
  assign r_rst_n = !f_sync;

endmodule
