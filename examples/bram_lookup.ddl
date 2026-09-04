-- A block RAM, the cycle its read costs, and the port that fills it.
--
-- `lutram` and `bram` are not two names for storage. They are two primitives
-- with different timing, and the annotation picks which one the synthesizer
-- infers:
--
--   lutram   one synchronous write port, ASYNCHRONOUS reads. The value is
--            there in the same cycle as the address. Cheap for small arrays,
--            expensive per bit, and it is what k2g_xstage's register file
--            needs -- read, forward and address-add all in one cycle.
--
--   bram     one synchronous write port, SYNCHRONOUS reads. The value arrives
--            a cycle after the address. Dense, and the only thing that holds
--            a table of any size.
--
-- WHERE THE CYCLE GOES has to be somewhere the source can point at, and the
-- only place in this language that holds one is a state of a blocking
-- `process`. So a `bram` read is a statement of its own -- `let v = t[a]` --
-- and it costs a state. A read written inside an expression is refused,
-- because there is no way to say the rest of the expression waits.
--
-- IT HAS A FILL PORT, and that is not decoration. A `bram` cannot have a
-- reset -- the reset loop is a write to every element, and 256x32 comes out as
-- 8192 flip-flops, so the compiler refuses the initialiser. A table with no
-- reset AND no write port has no drivers at all: every synthesizer deletes it,
-- and the version of this file without a fill port synthesized to six cells
-- and inferred nothing. Storage you cannot write is not storage.
--
-- WHAT THE ANNOTATION ACTUALLY DECIDES, measured on GowinSynthesis for the
-- GW1NR-9C rather than assumed:
--
--   this file, 256x32, read registered      -> 1 SDPB (block RAM)
--   this file with `lutram` instead         -> 1 SDPB, the same
--   k2g_xstage, 32x32, read used in the
--     same cycle it is addressed            -> 20 RAM16SDP1 + 32 RAM16SDP4
--                                              (distributed)
--
-- So the annotation does not pick the cell. It decides what the PROGRAM is
-- allowed to do, and the tool picks the cell from that. `bram` forbids using
-- the value in the cycle you addressed it, which is what leaves the
-- synthesizer free to choose a block RAM; `lutram` permits it, and where a
-- design actually depends on it -- k2g_xstage reads the register file,
-- forwards, and adds an address in one cycle -- distributed RAM is the only
-- thing that can do the job, so that is what comes out.
--
-- The backend emits both ports inside the memory's own clocked block, which is
-- the canonical template:
--
--     always @(posedge clk) begin
--       if (fire_s1) t[addr_r] <= din_data;
--       if (in_s2)   t_q       <= t[addr_r];
--     end
--
-- The earlier shape -- `wire q = t[a];` with the flop in the state machine --
-- infers the same SDPB and the same cell count, because GowinSynthesis retimes
-- the flop into the RAM's output register itself. The template above does not
-- depend on it being willing to.
--
-- examples/verify.sh counts the RAM primitives in the netlist, because that is
-- the only place the answer exists.

process bram_lookup (cmd: buffer in u16, din: buffer in u32, resp: buffer out u32)
  var t: #[impl(bram)] [u32; 256]

  loop
    let c = @rcv(cmd)

    -- Decoded once, in the cycle the command's handshake completes, so the
    -- branch below costs no cycle of its own.
    let addr: u8 = c[7..0]
    let is_write: u1 = c[15]

    if is_write then
      let d = @rcv(din)
      t[addr] = d
    else
      -- The address is registered on its way here: the fetch happens a cycle
      -- after the command was accepted, and `cmd_data` by then belongs to
      -- whatever the source is offering next.
      let v = t[addr]
      @send(resp, v)
