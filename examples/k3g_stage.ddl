-- A channel stage: one registered entry between two back-pressured pipes.
--
-- This is the shape k3g_expand.sv hand-writes, reduced to a payload small
-- enough to read. The point is not the field permutation -- it is that the
-- handshake is GENERATED. The body says what to do with an item; it never
-- mentions valid or ready.
--
-- Channel rule 3 (k3g_chan.sv:22) says `valid` must not depend combinationally
-- on `ready`. Here it cannot: an output pipe's `valid` is the `busy` register
-- and nothing else is allowed to drive it. k2g_chan.sv records the bug the
-- rule exists to prevent -- routing a stall back into `cp_valid` closed a loop
-- through stall -> decode -> CSP request -> stall.

enum kind_e: u3
  K_NOP
  K_ADD
  K_SUB
  K_LOAD
  K_STORE
  K_JUMP

struct in_item_t
  epoch: u3
  kind: kind_e
  dst: u5
  src: u5
  imm: u16

struct out_item_t
  epoch: u3
  kind: kind_e
  dst: u5
  src: u5
  imm: u32
  is_mem: u1

process k3g_stage (iops: buffer in in_item_t, uops: buffer out out_item_t)

  -- The process is a program: it runs once and stops. `loop` is what makes
  -- it repeat, once per cycle, for as long as the design runs.
  loop
    let (item, got) = @peek(iops)

    -- A fixed-format expansion, not a decode: unpack known fields and hand them
    -- on. The input stays in its buffer until the output accepts the expansion.
    var out: out_item_t = @zeroed()
    out.epoch = item.epoch
    out.kind = item.kind
    out.dst = item.dst
    out.src = item.src
    out.imm = {16'd0, item.imm}

    match item.kind
      .K_LOAD | .K_STORE =>
        out.is_mem = 1'b1
      _ =>
        out.is_mem = 1'b0

    -- An offer is made on the branch it is written on, so the cycles with no
    -- item have to say so rather than relying on the slot to notice.
    if got then
      let taken = @try_send(uops, out)
      if taken then
        let consumed = @drop(iops)
        @assert(consumed)
