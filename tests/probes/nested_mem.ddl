-- Writing part of an element of a memory, in RTL.
--
-- `t[a][k] = v` offers the write port a whole element assembled from a read of
-- the same address. That read is only free on an asynchronous memory, so every
-- module here is `lutram`; the synchronous kinds refuse the construct in the
-- compiler and there is nothing to simulate.
--
-- The lowered-circuit tests in tests/memory.rs check the same shapes against
-- the IR. These exist because the IR is not what runs: the splice becomes a
-- concatenation of part-selects and the forwarding becomes a wire, and a
-- simulator that agrees with the lowering can still disagree with Verilog.
--
-- Every module answers the same thing for every item, so the bench does not
-- have to track how many got through -- only what came out.

-- One lane, at an address and a lane number the item carries.
--   [3..0] address, [5..4] lane, [13..6] the byte
process nm_lane (cmd: buffer in u16, o: buffer out [u8; 4])
  var t: #[impl(lutram)] [[u8; 4]; 16] = @zeroed()

  loop
    let c = @rcv(cmd)
    let a: u4 = c[3..0]
    let lane: u2 = c[5..4]
    let d: u8 = c[13..6]
    t[a][lane] = d
    @send(o, t[a])

-- TWO lanes of one row in ONE cycle. The second lane's read has to see the
-- first lane's write, which has not reached the array yet -- forwarded, or
-- lane 0 is silently dropped by the second write port.
process nm_two (cmd: buffer in u8, o: buffer out [u8; 4])
  var t: #[impl(lutram)] [[u8; 4]; 16] = @zeroed()

  loop
    let c = @rcv(cmd)
    let a: u4 = c[3..0]
    t[a][2'd0] = 8'd170
    t[a][2'd1] = 8'd187
    @send(o, t[a])

-- A field of a struct element, rather than an element of an array element.
struct nm_pair
  lo: u8
  hi: u8

process nm_field (cmd: buffer in u8, o: buffer out nm_pair)
  var t: #[impl(lutram)] [nm_pair; 16] = @zeroed()

  loop
    let c = @rcv(cmd)
    let a: u4 = c[3..0]
    t[a].lo = 8'd9
    t[a].hi = 8'd4
    @send(o, t[a])

-- A nested write in each arm of an `if`. Both arms start from the same write
-- port count, so they share one port and the join muxes it -- on a merged
-- element here rather than on a plain one.
process nm_branch (cmd: buffer in u8, o: buffer out [u8; 4])
  var t: #[impl(lutram)] [[u8; 4]; 16] = @zeroed()

  loop
    let c = @rcv(cmd)
    let a: u4 = c[3..0]
    if c[7] then
      t[a][2'd0] = 8'd17
    else
      t[a][2'd3] = 8'd34
    @send(o, t[a])

-- The same construct in a pipeline. The read and the write are one statement,
-- so they cannot land in different stages; what this checks is that the memory
-- is seen as WRITTEN at all, since stage ownership is decided from that and a
-- nested target used to answer "writes nothing".
sequence nm_seq (req: buffer in u8, o: buffer out [u8; 4])
  var t: #[impl(lutram)] [[u8; 4]; 16] = @zeroed()

  let q = @rcv(req)
  let a: u4 = q[3..0]
  t[a][2'd1] = 8'd5
  let x = t[a]

  |||

  @send(o, x)
