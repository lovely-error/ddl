// DUT-1: the 2-entry gray-salt pipe, with its two sides on two clocks.
//
// Transcribed from KAMASUTRA2G/docs/attic/pipe.sv `PipeCDC` with Item = u16.
// That module is the ancestor of DDL's salt protocol, and the two are the same
// protocol: walking the writer from reset toggles write_salt[write_index] and
// then write_index, giving 00 -> 01 -> 11 -> 10 -> 00, which is DDL's gray
// sequence (docs/salt-protocol.md:92), with write_index equal to
// salt[0]^salt[1] at every state -- exactly what DDL derives instead of
// storing. The flags agree at all three occupancies too:
// write_salt[wi] != read_salt[wi] holds exactly when wsalt == ~rsalt, and
// read_salt[ri] == write_salt[ri] exactly when wsalt == rsalt.
//
// FOUR DELIBERATE CHANGES FROM THE ORIGINAL, each so the experiment measures
// what it claims to:
//
//   1. `storage[1:0]` is two explicit registers and a mux rather than an
//      array. An array read combinationally can be inferred as distributed
//      RAM, whose write-enable and read timing are not the flops this is
//      about. It is also what DDL emits (pack_entries/entry_of), so this moves
//      DUT-1 closer to DUT-2 rather than further away.
//   2. One `reset` becomes `sender_rst_n` and `reciever_rst_n`. The original
//      takes a single reset into both always blocks (pipe.sv:73, :84), which
//      is its own defect and would confound the crossing with a reset fault.
//   3. `syn_preserve` on every salt and index register, so the tool cannot
//      merge or duplicate the structure under test.
//   4. Two parameters, SYNC and DERIVE, selecting the arms below.
//
// SYNC = 0 is the original and is arm B. SYNC = 1 adds a two-flop synchronizer
// on each salt, EACH INTO THE DOMAIN THAT READS IT, and is arm C.
//
// DERIVE = 1 computes both indices from their own salt rather than storing
// them, and is arm F. It is a test of one specific claim about WHY arm B
// fails. `full` and `empty` are combinational off an asynchronous input and
// each fans out to several registers, which can disagree about the flag within
// a single cycle. With a STORED index that disagreement can break the module's
// central invariant -- write_index must always equal write_salt[0]^write_salt[1]
// -- and once those diverge, `full` tests the wrong bit, writes land in the
// wrong slot and the structure decoheres permanently. With a DERIVED index the
// invariant holds by construction, so the same disagreement can only lose or
// corrupt ONE item rather than breaking the FIFO.
//
// It is not a fix. Only synchronization makes the crossing correct; this only
// bounds the damage when it is not. And DDL already derives both indices and
// was still corrupted 99.994% of the time at pair 0, so the expectation going
// in is that this helps and does not rescue.

module pipe_cdc #(
    parameter SYNC   = 0,
    parameter DERIVE = 0
) (
    // Write side.
    input             sender_clk,
    input             sender_rst_n,
    input             put,
    input      [15:0] data_in,
    output            full,

    // Read side.
    input             reciever_clk,
    input             reciever_rst_n,
    input             drop,
    output            valid,
    output     [15:0] item
);

  reg [1:0] write_salt /* synthesis syn_preserve = 1 */;
  reg [1:0] read_salt  /* synthesis syn_preserve = 1 */;

  wire write_index;
  wire read_index;

  // `storage`, as two flops. Written in the sender's domain, read
  // combinationally in the receiver's -- the same electrical situation as
  // DDL's `o_data`, which ships both entries out of the producer as wires.
  reg [15:0] e0 /* synthesis syn_preserve = 1 */;
  reg [15:0] e1 /* synthesis syn_preserve = 1 */;

  // What each side sees of the other's salt. THE CROSSING.
  wire [1:0] read_salt_seen;    // in the sender's domain
  wire [1:0] write_salt_seen;   // in the receiver's domain

  wire do_put;
  wire do_drop;

  generate
    if (SYNC == 0) begin : g_raw
      // Arm B: straight across, which is what the compiler emits today.
      assign read_salt_seen  = read_salt;
      assign write_salt_seen = write_salt;
    end else begin : g_sync
      // Arm C: two flops in the domain that READS the salt. Note which clock
      // each chain runs on -- a synchronizer on the wrong clock is not a
      // synchronizer. What this buys is not delay: it makes the flag a
      // combinational function of registers in the READER's own domain, so
      // every register it feeds necessarily agrees about it.
      reg [1:0] rs_meta /* synthesis syn_preserve = 1 */;
      reg [1:0] rs_sync /* synthesis syn_preserve = 1 */;
      reg [1:0] ws_meta /* synthesis syn_preserve = 1 */;
      reg [1:0] ws_sync /* synthesis syn_preserve = 1 */;

      always @(posedge sender_clk) begin
        if (!sender_rst_n) begin
          rs_meta <= 2'b00;
          rs_sync <= 2'b00;
        end else begin
          rs_meta <= read_salt;
          rs_sync <= rs_meta;
        end
      end

      always @(posedge reciever_clk) begin
        if (!reciever_rst_n) begin
          ws_meta <= 2'b00;
          ws_sync <= 2'b00;
        end else begin
          ws_meta <= write_salt;
          ws_sync <= ws_meta;
        end
      end

      assign read_salt_seen  = rs_sync;
      assign write_salt_seen = ws_sync;
    end
  endgenerate

  generate
    if (DERIVE == 0) begin : g_stored
      // The original: a register of its own, which can drift out of step with
      // the salt and take the whole structure with it.
      reg wi_r /* synthesis syn_preserve = 1 */;
      reg ri_r /* synthesis syn_preserve = 1 */;

      always @(posedge sender_clk) begin
        if (!sender_rst_n)  wi_r <= 1'b0;
        else if (do_put)    wi_r <= ~wi_r;
      end

      always @(posedge reciever_clk) begin
        if (!reciever_rst_n) ri_r <= 1'b0;
        else if (do_drop)    ri_r <= ~ri_r;
      end

      assign write_index = wi_r;
      assign read_index  = ri_r;
    end else begin : g_derived
      // Arm F: the index IS the salt's parity, so it cannot disagree with it.
      assign write_index = write_salt[0] ^ write_salt[1];
      assign read_index  = read_salt[0]  ^ read_salt[1];
    end
  endgenerate

  assign full  = write_salt[write_index] != read_salt_seen[write_index];
  assign valid = !(read_salt[read_index] == write_salt_seen[read_index]);
  assign item  = read_index ? e1 : e0;

  assign do_put  = put  && !full;
  assign do_drop = drop && valid;

  always @(posedge sender_clk) begin
    if (!sender_rst_n) begin
      write_salt <= 2'b00;
      e0         <= 16'd0;
      e1         <= 16'd0;
    end else if (do_put) begin
      if (write_index) e1 <= data_in;
      else             e0 <= data_in;
      write_salt[write_index] <= ~write_salt[write_index];
    end
  end

  always @(posedge reciever_clk) begin
    if (!reciever_rst_n) begin
      read_salt <= 2'b00;
    end else if (do_drop) begin
      read_salt[read_index] <= ~read_salt[read_index];
    end
  end

endmodule
