// The board top for the CDC demonstration.
//
// Two devices under test, one bitstream, four arms. What varies between arms is
// generated into arm_config.vh by build.sh; everything else is here.
//
//   arm A   both sides on clock A                  control
//   arm B   two clocks, salts straight across      what the compiler emits today
//   arm C   two clocks, salts through 2-flop syncs
//   arm E   two clocks, a proven depth-8 async FIFO in place of the pipes
//   arm G   two clocks, the crossing WRITTEN AND WIRED BY THE COMPILER
//
// Arm G is arm E's measurement made again against `--async-export` output: the
// same board, the same clocks, the same checkers, with the hand-wired FIFO
// replaced by whatever `ddl build --async-export` puts in the file. Slot 1
// crosses out of a `buffer out`, slot 2 into a `buffer in`, so both generated
// shells are on the board and not only in the bench.
//
// SLOT 1 is `pipe_cdc`, the 2-entry salt pipe with its write side on clock A
// and its read side on clock B. SLOT 2 is the compiler's own salt link: `gen`
// on clock A, `chk` on clock B, and the three bundles between them are exactly
// what src/ir_graph.rs:762-764 emits for two instances in a graph. In arm E
// both slots become `cdc_fifo`.
//
// Every endpoint -- the source feeding a slot, the sink draining it -- is
// single-domain, where the protocol is known good. Only the object in the
// middle crosses.
//
// THE REPORT LIVES IN CLOCK B. Every counter worth reading is on the receiving
// side, so the UART goes there too. Moving them into another domain to be
// printed would put a clock crossing inside the instrument measuring one.

`include "arm_config.vh"

// Divider widths, so a testbench can see a whole report line in microseconds
// instead of the second the board takes. ONLY the widths change: the report
// state machine, the character order and the UART are the same logic either way.
`ifndef RPT_TICK_BITS
  `define RPT_TICK_BITS 27
`endif
`ifndef CHAR_DIV_BITS
  `define CHAR_DIV_BITS 14
`endif

module cdc_demo_top (
    input        clk_27m,
    input        rst_n,
    input        uart_rx,   // named by the .cst; unused here
    output       uart_tx,
    output [5:0] led
);

  // ---------------------------------------------------------------- clocks
  wire clk_a, lock_a;
  wire clk_b, lock_b;

  // No parameter overrides: build.sh bakes the dividers into cdc_pll_gen.v as
  // literals, because GowinSynthesis silently substitutes a default for a
  // parameter forwarded into rPLL and reports it only as a WARN. See
  // cdc_pll.v.in.
  cdc_pll_a u_pll_a (
      .clk_in (clk_27m),
      .clk_out(clk_a),
      .lock   (lock_a)
  );

`ifdef ARM_A
  // The control arm is one clock, so it is one PLL. NOT a fabric mux: a clock
  // selected through logic is a different experiment from a clock out of a PLL.
  assign clk_b  = clk_a;
  assign lock_b = lock_a;
`else
  cdc_pll_b u_pll_b (
      .clk_in (clk_27m),
      .clk_out(clk_b),
      .lock   (lock_b)
  );
`endif

  // ----------------------------------------------------------------- reset
  //
  // The button is asynchronous to both PLL outputs, and the DUTs take a
  // SYNCHRONOUS reset -- which is what the compiler emits. Driven raw, a
  // release near a clock edge is caught by some flops and not others, and that
  // failure has nothing to do with the crossing while looking exactly like it.
  //
  // So: one common source, gated on both locks, then TWO FLOPS AND A HOLD
  // COUNTER PER DOMAIN, each clocked by its own clock. `por_async` is stable
  // long before either counter expires, so both sides are held together and
  // released within a bounded skew -- and both salts start at zero, which is
  // all the release skew needs. If A goes first it pushes twice and stalls at
  // write_salt = 11; if B goes first it sees empty and idles. Nothing is lost.
  //
  // 256 cycles is far past the floor, which is three of the slower clock's
  // periods: two for the synchronizer to propagate and one to act on it.
  wire por_async = rst_n & lock_a & lock_b;

  reg       por_a_meta, por_a_sync;
  reg [7:0] hold_a;
  always @(posedge clk_a) begin
    por_a_meta <= por_async;
    por_a_sync <= por_a_meta;
    if (!por_a_sync)          hold_a <= 8'd0;
    else if (hold_a != 8'hFF) hold_a <= hold_a + 8'd1;
  end
  wire rst_a_n = (hold_a == 8'hFF);

  reg       por_b_meta, por_b_sync;
  reg [7:0] hold_b;
  always @(posedge clk_b) begin
    por_b_meta <= por_async;
    por_b_sync <= por_b_meta;
    if (!por_b_sync)          hold_b <= 8'd0;
    else if (hold_b != 8'hFF) hold_b <= hold_b + 8'd1;
  end
  wire rst_b_n = (hold_b == 8'hFF);

  // Counting starts after a warm-up, so a one-off startup effect cannot be read
  // as a crossing fault. rx and cyc start together, so their ratio is a rate.
  reg [10:0] warm;
  always @(posedge clk_b) begin
    if (!rst_b_n)             warm <= 11'd0;
    else if (warm != 11'h7FF) warm <= warm + 11'd1;
  end
  // Registered, not a wire. As a comparator it drove the clock enable of every
  // counter below and showed up in ten violating paths; as a flop it drives one.
  reg run;
  always @(posedge clk_b) begin
    if (!rst_b_n)                  run <= 1'b0;
    else if (warm == 11'h7FF)      run <= 1'b1;
  end

  // ---------------------------------------------------------------- slot 1
  wire        valid1;
  wire [15:0] item1;
  wire        drop1 = valid1;

`ifdef ARM_G
  // THE COMPILER WROTE AND WIRED THIS CROSSING. `gen` was built with
  // `--async-export gen.o=rx`, so the emitted wrapper holds a
  // `ddl_cdc_out_16x8` -- `ddl_cdc_fifo`, `ddl_rst_cross` and one AND gate of
  // glue -- between the adapter and the port. The process runs on clock A; its
  // face is read on clock B.
  //
  // There is no source counter and no `full1` here: the DDL process IS the
  // source, and the compiler's own adapter applies the backpressure. Arm E's
  // slot 1 is the same measurement with the FIFO wired in by hand, so the two
  // are read against each other directly.
  //
  // Note what is NOT wired: a second reset. `rst_a_n` is the only reset this
  // module takes, and `ddl_rst_cross` inside it carries that into clock B, so
  // both halves of the FIFO leave zero together. `rst_b_n` never reaches it.
  gen u_slot1 (
      .clk            (clk_a),
      .rst_n          (rst_a_n),
      .o_has_data     (valid1),
      .o_drop_item    (drop1),
      .o_data_read_out(item1),
      .rx_clk         (clk_b)
  );
`else
  reg  [15:0] src1;
  wire        full1;

  always @(posedge clk_a) begin
    if (!rst_a_n)    src1 <= 16'd0;
    else if (!full1) src1 <= src1 + 16'd1;
  end

  `ifdef ARM_E
  wire empty1;
  assign valid1 = !empty1;
  cdc_fifo u_slot1 (
      .wclk  (clk_a),
      .wrst_n(rst_a_n),
      .wpush (1'b1),
      .wdata (src1),
      .wfull (full1),
      .rclk  (clk_b),
      .rrst_n(rst_b_n),
      .rpop  (drop1),
      .rdata (item1),
      .rempty(empty1)
  );
  `else
  pipe_cdc #(
      .SYNC  (`SLOT1_SYNC),
      .DERIVE(`SLOT1_DERIVE)
  ) u_slot1 (
      .sender_clk    (clk_a),
      .sender_rst_n  (rst_a_n),
      .put           (1'b1),
      .data_in       (src1),
      .full          (full1),
      .reciever_clk  (clk_b),
      .reciever_rst_n(rst_b_n),
      .drop          (drop1),
      .valid         (valid1),
      .item          (item1)
  );
  `endif
`endif

  // ---------------------------------------------------------------- slot 2
  wire        valid2;
  wire [15:0] item2;
  wire        drop2 = valid2;

`ifdef ARM_G
  // The same compiler-written crossing in the OTHER direction. `chk` was built
  // with `--async-export chk.src=tx`, so its wrapper holds a `ddl_cdc_in_16x8`:
  // the process core runs on clock B, and only the `src` face was moved onto
  // clock A. `dst` stays on clock B, un-crossed, where the sink already lives.
  //
  // Slot 1 crosses out of a `buffer out`; this crosses into a `buffer in`. Both
  // shells are therefore exercised on the board rather than only in the bench,
  // and both deliver into clock B where the checkers are.
  //
  // Its ceiling is 0.5 items per clock-B cycle, not 0.75 -- `chk` is a
  // two-state process and forwards one item every two cycles. That is the SAME
  // ceiling arm A measures for this slot on one clock, which is what makes the
  // comparison worth making: the crossing should cost nothing.
  reg  [15:0] src2g;
  wire        src2g_room;

  always @(posedge clk_a) begin
    if (!rst_a_n)        src2g <= 16'd0;
    else if (src2g_room) src2g <= src2g + 16'd1;
  end

  chk u_slot2 (
      .clk              (clk_b),
      .rst_n            (rst_b_n),
      .src_can_receive  (src2g_room),
      .src_receive_en   (src2g_room),
      .src_data_write_in(src2g),
      .tx_clk           (clk_a),
      .dst_has_data     (valid2),
      .dst_drop_item    (drop2),
      .dst_data_read_out(item2)
  );
`else
`ifdef ARM_E
  // Arm E measures the fix twice rather than measuring the DDL link, which
  // cannot be bridged by a FIFO without the adapters Phase 1 adds.
  reg  [15:0] src2;
  wire        full2;
  wire        empty2;

  always @(posedge clk_a) begin
    if (!rst_a_n)    src2 <= 16'd0;
    else if (!full2) src2 <= src2 + 16'd1;
  end

  assign valid2 = !empty2;
  cdc_fifo u_slot2 (
      .wclk  (clk_a),
      .wrst_n(rst_a_n),
      .wpush (1'b1),
      .wdata (src2),
      .wfull (full2),
      .rclk  (clk_b),
      .rrst_n(rst_b_n),
      .rpop  (drop2),
      .rdata (item2),
      .rempty(empty2)
  );
`else
  // The compiler's own link. `gen` drives the write salt and both entries on
  // clock A; `chk` drives the read salt on clock B. THESE THREE BUNDLES ARE THE
  // CROSSING.
  wire [1:0]  gen_wsalt;
  wire [31:0] gen_data;
  wire [1:0]  chk_rsalt;
  wire [1:0]  chk_wsalt_in;
  wire [1:0]  gen_rsalt_in;

  gen u_gen (
      .clk    (clk_a),
      .rst_n  (rst_a_n),
      .o_wsalt(gen_wsalt),
      .o_rsalt(gen_rsalt_in),
      .o_data (gen_data)
  );

`ifdef SLOT2_SYNC
  // Arm C: each salt through two flops IN THE DOMAIN THAT READS IT. A
  // synchronizer on the wrong clock is not a synchronizer.
  reg [1:0] ws2_meta /* synthesis syn_preserve = 1 */;
  reg [1:0] ws2_sync /* synthesis syn_preserve = 1 */;
  reg [1:0] rs2_meta /* synthesis syn_preserve = 1 */;
  reg [1:0] rs2_sync /* synthesis syn_preserve = 1 */;

  always @(posedge clk_b) begin
    if (!rst_b_n) begin
      ws2_meta <= 2'b00;
      ws2_sync <= 2'b00;
    end else begin
      ws2_meta <= gen_wsalt;
      ws2_sync <= ws2_meta;
    end
  end

  always @(posedge clk_a) begin
    if (!rst_a_n) begin
      rs2_meta <= 2'b00;
      rs2_sync <= 2'b00;
    end else begin
      rs2_meta <= chk_rsalt;
      rs2_sync <= rs2_meta;
    end
  end

  assign chk_wsalt_in = ws2_sync;
  assign gen_rsalt_in = rs2_sync;
`else
  assign chk_wsalt_in = gen_wsalt;
  assign gen_rsalt_in = chk_rsalt;
`endif

  // `chk` forwards onto a second pipe, drained in clock B by the sink below.
  wire [1:0]  chk_dst_wsalt;
  wire [31:0] chk_dst_data;
  reg  [1:0]  sink2_rsalt;

  chk u_chk (
      .clk      (clk_b),
      .rst_n    (rst_b_n),
      .src_wsalt(chk_wsalt_in),
      .src_rsalt(chk_rsalt),
      .src_data (gen_data),
      .dst_wsalt(chk_dst_wsalt),
      .dst_rsalt(sink2_rsalt),
      .dst_data (chk_dst_data)
  );

  // The consumer half of the salt protocol, entirely inside clock B, where it
  // is known good. docs/salt-protocol.md is the specification.
  wire sink2_ridx  = sink2_rsalt[0] ^ sink2_rsalt[1];
  wire sink2_empty = (chk_dst_wsalt == sink2_rsalt);

  assign valid2 = !sink2_empty;
  assign item2  = sink2_ridx ? chk_dst_data[31:16] : chk_dst_data[15:0];

  always @(posedge clk_b) begin
    if (!rst_b_n)   sink2_rsalt <= 2'b00;
    else if (drop2) sink2_rsalt <= sink2_rsalt ^ (sink2_ridx ? 2'd2 : 2'd1);
  end
`endif
`endif

  // -------------------------------------------------------------- checkers
  //
  // Both sinks resync their expectation on a mismatch, which is what makes the
  // two failure signatures distinguishable: a startup or reset desync is a
  // constant offset and costs ONE error, while a crossing fault is a recurring
  // race and keeps accumulating.
  //
  // THE SHAPES HERE ARE CHOSEN FOR TIMING, and the first build is why. At 81
  // MHz the control arm closed at 70.1 with 42 violated endpoints, every one of
  // them in this instrument rather than in a DUT -- and an instrument that does
  // not meet timing produces corrupted counters that read exactly like the
  // failure it is looking for. Each shape is noted where it is used.
  // The item and the take are REGISTERED before anything arithmetic sees them.
  // Straight from the DUT the path is salt register -> item mux -> 16-bit add
  // (`expect <= item + 1`) and 16-bit compare (`item != expect`) -> register:
  // 8.2 ns, and the whole of what still missed 135 MHz after the report was
  // fixed. Split in two it is a mux to a flop, then arithmetic from a flop. The
  // counters are tallies; a cycle of latency is invisible in them.
  reg [15:0] item1_q, item2_q;
  reg        took1_q, took2_q;

  always @(posedge clk_b) begin
    if (!rst_b_n) begin
      item1_q <= 16'd0; took1_q <= 1'b0;
      item2_q <= 16'd0; took2_q <= 1'b0;
    end else begin
      item1_q <= item1; took1_q <= drop1;
      item2_q <= item2; took2_q <= drop2;
    end
  end

  reg [31:0] rx1, err1;
  reg [23:0] idle1, idle1_hw;
  reg [15:0] expect1;
  reg        armed1, miss1;

  always @(posedge clk_b) begin
    if (!rst_b_n) begin
      rx1 <= 32'd0; err1 <= 32'd0; idle1 <= 24'd0; idle1_hw <= 24'd0;
      expect1 <= 16'd0; armed1 <= 1'b0; miss1 <= 1'b0;
    end else begin
      // Registered before it reaches the counter's enable. A 16-bit compare
      // driving a 32-bit carry chain's CE was the second-worst path in an
      // earlier build; a tally does not care that it arrives a cycle late.
      miss1 <= took1_q && run && armed1 && (item1_q != expect1);
      if (miss1) err1 <= err1 + 32'd1;

      if (took1_q) begin
        idle1 <= 24'd0;
        if (run) rx1 <= rx1 + 32'd1;
        expect1 <= item1_q + 16'd1;
        armed1  <= 1'b1;
      end else begin
        idle1 <= idle1 + 24'd1;
        // A BITWISE OR, not a running max. Tracking the true maximum needs a
        // 24-bit compare driving 24 clock enables, which was the worst path in
        // the design once everything else was fixed. OR-ing every idle count
        // seen keeps the TOP SET BIT exact -- the magnitude of the longest
        // stall, which is all the lockup test reads -- for one LUT level and no
        // enable logic at all. The lower bits are not a number; do not read
        // them as one.
        idle1_hw <= idle1_hw | idle1;
      end
    end
  end

  reg [31:0] rx2, err2;
  reg [23:0] idle2, idle2_hw;
  reg [15:0] expect2;
  reg        armed2, miss2;

  always @(posedge clk_b) begin
    if (!rst_b_n) begin
      rx2 <= 32'd0; err2 <= 32'd0; idle2 <= 24'd0; idle2_hw <= 24'd0;
      expect2 <= 16'd0; armed2 <= 1'b0; miss2 <= 1'b0;
    end else begin
      miss2 <= took2_q && run && armed2 && (item2_q != expect2);
      if (miss2) err2 <= err2 + 32'd1;

      if (took2_q) begin
        idle2 <= 24'd0;
        if (run) rx2 <= rx2 + 32'd1;
        expect2 <= item2_q + 16'd1;
        armed2  <= 1'b1;
      end else begin
        idle2 <= idle2 + 24'd1;
        idle2_hw <= idle2_hw | idle2;
      end
    end
  end

  reg [31:0] cyc;
  always @(posedge clk_b) begin
    if (!rst_b_n) cyc <= 32'd0;
    else if (run) cyc <= cyc + 32'd1;
  end

  // ---------------------------------------------------------------- report
  //
  // One line a second, in hex:
  //   <arm> rx1 err1 idle1_hw rx2 err2 idle2_hw cyc
  //
  // Throughput is rx/cyc and is a measurement rather than an impression. Each
  // DUT has its own baseline: slot 1 moves one item per cycle when it can, slot
  // 2 is a two-state process and moves one per two. Arm A establishes both, and
  // every other arm is read against arm A for the SAME slot.
  //
  // ONE 224-BIT SHIFT REGISTER, not seven snapshots and a mux. The obvious
  // form -- snap0..snap6, a 7-way select on the field index, then a variable
  // shift to pick the nibble -- put two of the three worst paths in the design
  // between one flop and the next, and clk_b closed at 97 MHz against 135. Here
  // the seven counters load straight into one register (one level of logic) and
  // every character is the top four bits, so there is no mux and no shifter on
  // any path. An instrument that misses timing reports corrupted counters, and
  // corrupted counters read exactly like the failure this is looking for.
  reg [223:0] snapsr;
  reg  [`RPT_TICK_BITS-1:0] rpt_tick;
  reg         rpt_fire;
  reg         sending;
  reg   [2:0] fld;
  reg   [3:0] pos;
  // `pos < 8` as a register. As a decode it drove the clock enable of all 224
  // shift flops and was the worst path in arm E.
  reg         pos_hex;
  reg         in_lf;
  reg   [7:0] tx_data;
  reg         tx_valid;
  wire        tx_ready;

  wire [3:0] nib  = snapsr[223:220];
  wire [7:0] hex  = (nib < 4'd10) ? (8'd48 + {4'd0, nib}) : (8'd87 + {4'd0, nib});
  wire [7:0] ch   = in_lf         ? 8'h0A :
                    (pos == 4'd9) ? `ARM_CHAR :
                    (pos == 4'd8) ? ((fld == 3'd6) ? 8'h0D : 8'h20) : hex;
  // ONE CHARACTER EVERY 2^14 CYCLES, from a registered tick -- not "whenever the
  // UART says it is ready". `tx_ready` is a compare inside the transmitter, and
  // using it here put that compare in front of the clock enable of all 224
  // shift-register flops: the worst path in arms C and E. A byte takes 10 x
  // DIVISOR cycles (11720 at the fastest report clock here), so 16384 is always
  // long enough and the transmitter is always idle when this fires.
  reg [`CHAR_DIV_BITS-1:0] cdiv;
  reg        char_tick;
  always @(posedge clk_b) begin
    if (!rst_b_n) begin
      cdiv      <= 0;
      char_tick <= 1'b0;
    end else begin
      cdiv      <= cdiv + `CHAR_DIV_BITS'd1;
      // GATED ON THE TRANSMITTER BEING IDLE. Without this, a tick landing while
      // the UART is busy drops the character but still shifts the register --
      // fields slide, the content runs out early and the tail goes to zeros,
      // which is exactly the corruption this reporter produced on the board
      // after a reset press. `tx_ready` is `!busy`, one inverter on a register,
      // so char_tick's D path grows by a gate and the 224 shift-register clock
      // enables still come from a register -- which is the whole reason
      // readiness was taken out of `step` in the first place.
      char_tick <= (&cdiv) && tx_ready;
    end
  end

  wire step = sending && char_tick;

  // Registered, and this is the other half of the same lesson: as a bare wire
  // the 27-bit compare drove the clock enable of all 224 snapshot flops and
  // accounted for 24 of the 25 worst paths. As a flop it drives one.
  always @(posedge clk_b) begin
    if (!rst_b_n) begin
      rpt_tick <= 0;
      rpt_fire <= 1'b0;
    end else begin
      rpt_tick <= rpt_tick + `RPT_TICK_BITS'd1;
      rpt_fire <= (rpt_tick == 0);
    end
  end

  always @(posedge clk_b) begin
    if (!rst_b_n) begin
      sending  <= 1'b0;
      fld      <= 3'd0;
      pos      <= 4'd9;
      in_lf    <= 1'b0;
      tx_valid <= 1'b0;
      tx_data  <= 8'd0;
      snapsr   <= 224'd0;
      pos_hex  <= 1'b0;
    end else begin
      tx_valid <= 1'b0;

      if (!sending && rpt_fire) begin
        // 0xA5 IS A MARKER, not data. The idle counters are 24 bits and their
        // top byte was previously a hardwired zero -- which meant a garbled
        // line still looked plausible. Now every correct line reads `a5......`
        // in fields 3 and 6, and any misalignment is visible immediately
        // instead of taking an afternoon to prove from field deltas.
        snapsr  <= {rx1, err1, {8'hA5, idle1_hw},
                    rx2, err2, {8'hA5, idle2_hw}, cyc};
        sending <= 1'b1;
        fld     <= 3'd0;
        pos     <= 4'd9;          // the arm letter comes first
        pos_hex <= 1'b0;
        in_lf   <= 1'b0;
      end else if (step) begin
        tx_valid <= 1'b1;
        tx_data  <= ch;
        if (in_lf) begin
          in_lf   <= 1'b0;
          sending <= 1'b0;
        end else if (pos == 4'd9) begin
          pos     <= 4'd0;
          pos_hex <= 1'b1;
        end else if (pos == 4'd8) begin
          if (fld == 3'd6) begin
            in_lf <= 1'b1;
          end else begin
            fld     <= fld + 3'd1;
            pos     <= 4'd0;
            pos_hex <= 1'b1;
          end
        end else if (pos_hex) begin
          // Only a hex character consumes nibbles, so 7 fields x 8 shifts
          // empties the register exactly.
          pos     <= pos + 4'd1;
          pos_hex <= (pos != 4'd7);
          snapsr  <= {snapsr[219:0], 4'b0000};
        end
      end
    end
  end

  cdc_uart_tx #(
      .DIVISOR(`UART_DIVISOR)
  ) u_tx (
      .clk  (clk_b),
      .rst_n(rst_b_n),
      .valid(tx_valid),
      .data (tx_data),
      .ready(tx_ready),
      .tx   (uart_tx)
  );

  // ------------------------------------------------------------------ leds
  // Active low: driving 0 lights the LED.
  wire any_err = (err1 != 32'd0) || (err2 != 32'd0);
  wire lockup  = idle1_hw[20] || idle2_hw[20];   // ~7.8 ms at 135 MHz
  assign led = ~{err1[3:0], lockup, any_err};

endmodule
