-- The K2G execute stage, with the register file inside it.
--
-- This is the answer to "is k2g_regfile encodable in DDL". The storage is,
-- and always was -- four arrays, one write port, asynchronous reads. What is
-- not encodable is the MODULE: 24 flat ports exposing combinational reads of
-- local state to a different module. desc.md:26 forbids exactly that, and it
-- is a design position rather than a gap.
--
-- Putting the file behind a channel instead would compile, and would be worse
-- than not compiling. docs/gowin-sv-support.md:103-118 is explicit: the
-- 3-stage F/D/X design exists BECAUSE the value array reads asynchronously --
-- that is what lets one cycle do register read -> forward -> address add ->
-- memory address. A channel registers the response, which is the fourth stage
-- the whole arrangement is built to avoid. k2g_core.sv:596 is the path in
-- question: `xb_value + uop.imm`, combinational, in the same cycle as the
-- array read.
--
-- So the arrays live in the process that reads them, reached by subscript, and
-- the boundary carries micro-ops in and writeback packets out. The read stays
-- asynchronous, the address adder stays in the same cycle, and there is no
-- channel between them.
--
-- WHAT IS HERE. The register file and the path that depends on its read
-- timing; predication and condition codes; the reserved-F32 operand check;
-- alignment and range; and the fault priority chain that decides between them.
--
-- WHAT IS STILL IN k2g_core.sv. The CSP port, the widening multiply, jumps,
-- and the memory stage.
-- The first two are BLOCKED rather than skipped: both
-- need the process to spend a second cycle without accepting a new uop, and a
-- DDL process with no blocking operation is one state that fires every cycle
-- and cannot decline its input. Mixing a blocking `@send` into this body to
-- get that second state is what the language would want -- and `@try_rcv`
-- beside it is refused today (`@try_rcv` is not supported in a process with
-- states). Jumps need `isa`, which is not in `uop_t`; it wants a payload
-- struct carrying the uop and the address it started at.
--
-- WHAT THE AREA COMPARISON MEASURES, which is not what it looks like.
-- `verify.sh` reports the DDL at 3106 primitives against the reference's 2761,
-- +12.5%. All of that is the `wb` channel and none of it is the logic.
--
-- The reference's `packet` is a bare combinational output with no handshake.
-- This process emits a real pipe, so it pays a head and a skid: 2N+2 flops for
-- an N-bit payload, plus the muxing that feeds them. The netlist names them --
-- 168 of the 192 DFFREs are `wb_hold` (84) and `wb_skid` (84), and the
-- reference has the other 24 and nothing else. Narrowing the packet from 84
-- bits to 46 removes 223 primitives, so the channel scales with the payload
-- and dominates the difference.
--
-- Which makes the comparison unfair rather than the design wasteful. Narrow
-- the output to one bit and the DDL synthesizes to 2555 against the
-- reference's 2761 -- SMALLER, by 7.5%. (Read that number as a direction and
-- not a total: with nothing reading `fault` and `fault_info` they optimize
-- away, so it flatters by however much that mux costs. `fault_valid` stays
-- live through `commits`.)
--
-- The port is here so the equivalence testbench can watch what X computed; the
-- arrays are written from `w`, inside. In `k2g_machine.ddl` this becomes the
-- real X->M boundary and the cost stops being scaffolding.
--
-- THE VALUE ARRAY HAS EXACTLY ONE WRITE PORT, and in DDL it cannot have two.
-- k2g_regfile.sv:18-24 records what a second one cost when it was tried: no
-- RAM was inferred at all, and 32 primitives plus ~100 LUTs became 1120
-- flip-flops and ~3700 LUTs of read muxing.

-- What X computed, which becomes the register file's write port at the next
-- edge. k2g_core.sv:1111 -- "X computes a writeback packet; W is what actually
-- drives the register file's write port. That is the split."
import "k2g_types.ddl"
-- The slice CALLS the verified ALU and shifter rather than repeating them.
import "k2g_alu.ddl"
import "k2g_shift.ddl"

struct wb_t
  we_value: i1
  we_tag: i1
  we_overflow: i1
  we_flag: i1
  addr: i5
  value: i32
  tag: rdt_e
  overflow: i1
  flag: i1

  -- What X detected, travelling with the packet rather than on ports of its
  -- own. `fault_valid` suppresses every write above it; `fault_info` is the
  -- number the FAULT_INFO port reports (spec 8).
  fault_valid: i1
  fault: fault_e
  fault_info: i32

-- A register's bits always match what its tag claims (spec 5.1, 1.1.1), so a
-- value written back is re-extended to its tag's width. `RDT_U32` and
-- `RDT_S32` are the identity, which is why one normalize after the writeback
-- mux costs nothing on the branches that do not need it (k2g_core.sv:880).
fun rdt_normalize (v: i32, t: rdt_e, o: out i32)
  let sgn: i1 = rdt_is_signed(t)
  -- On one line each: a wrapped `if ... then ... else` parses as a block
  -- rather than as an expression, which lowering then refuses.
  let as_byte: i32 = if sgn then @concat(@rep(v[7], 24), v[7..0]) else @concat(24'd0, v[7..0])
  let as_half: i32 = if sgn then @concat(@rep(v[15], 16), v[15..0]) else @concat(16'd0, v[15..0])
  let width: i2 = t[1..0]
  o = if width == 2'd0 then as_byte else if width == 2'd1 then as_half else v

-- Store width comes from the source register's tag, not the opcode (spec 5.3),
-- so the width is a function of a tag everywhere it is needed.
fun rdt_width_bytes (t: rdt_e, w: out i3)
  let low: i2 = t[1..0]
  w = if low == 2'd0 then 3'd1 else if low == 2'd1 then 3'd2 else 3'd4

-- `mem_bytes` is where ADDR_OUT_OF_RANGE begins (spec 8), so it is
-- architecture rather than configuration: the emulator carries the same
-- number and the cosimulation is what checks that they agree. A plain
-- parameter is folded at compile time and is not a port.
process k2g_xstage (uops: buffer in uop_t, wb: buffer out wb_t,
                    mem_bytes: i32 = 32'h00800000)
  var values: #[impl(lutram)] [i32; 32] = @zeroed()
  -- The reset tag matches the emulator's, so a register that has never been
  -- written still compares equal.
  var tags: #[impl(lutram)] [rdt_e; 32] = RDT_U32
  var overflow: #[impl(lutram)] [i1; 32] = @zeroed()
  var flagbit: #[impl(lutram)] [i1; 32] = @zeroed()

  -- The W stage: what the previous cycle computed. It drives the array write
  -- port AND the forwarding mux, which is why it has to be read before it is
  -- reassigned at the bottom of the body.
  var w: wb_t = @zeroed()

  loop
    let (uop, got) = @try_rcv(uops)

    -- ---- register read -------------------------------------------------
    -- Asynchronous, straight out of the arrays. Nothing is registered between
    -- here and the address adder below; that is the property this whole file
    -- exists to keep.
    let ra: i5 = uop.dst
    let rb: i5 = uop.src
    -- The third port reads the register an XCP/XCN prefix names. Predication
    -- is out of this slice, so the forwarded `xc_*` below feed nothing and are
    -- stripped -- but the port is read here because K2G reads it, and leaving
    -- it out would quietly compare a two-port file against a three-port one.
    let rc: i5 = uop.cond_reg

    let ra_value: i32 = values[ra]
    let rb_value: i32 = values[rb]
    let rc_value: i32 = values[rc]
    let ra_tag: rdt_e = tags[ra]
    let rb_tag: rdt_e = tags[rb]
    let rc_tag: rdt_e = tags[rc]
    let ra_ovf: i1 = overflow[ra]
    let rb_ovf: i1 = overflow[rb]
    let rc_ovf: i1 = overflow[rc]
    let ra_flg: i1 = flagbit[ra]
    let rb_flg: i1 = flagbit[rb]
    let rc_flg: i1 = flagbit[rc]

    -- ---- forwarding W -> X ---------------------------------------------
    -- The file reads before it writes, so this instruction cannot see the
    -- write its predecessor is committing this very cycle. There is exactly
    -- one source to forward from, which keeps it a mux rather than a priority
    -- network.
    --
    -- PER FIELD, NOT PER REGISTER. A comparison writes only `flag`, an
    -- arithmetic op writes only `overflow`, and a copy writes everything.
    -- Forwarding a whole register on any write would overwrite the fields the
    -- instruction in W is not touching. The tag matters most and is the least
    -- obvious: a store takes its width from the source register's tag, so an
    -- unforwarded tag does not corrupt a value, it writes the wrong number of
    -- bytes.
    let writing: i1 = w.we_value | w.we_tag | w.we_overflow | w.we_flag
    let ma: i1 = writing & (w.addr == ra)
    let mb: i1 = writing & (w.addr == rb)
    let mc: i1 = writing & (w.addr == rc)

    let xa_value: i32 = if ma & w.we_value then w.value else ra_value
    let xb_value: i32 = if mb & w.we_value then w.value else rb_value
    let xc_value: i32 = if mc & w.we_value then w.value else rc_value
    let xa_tag: rdt_e = if ma & w.we_tag then w.tag else ra_tag
    let xb_tag: rdt_e = if mb & w.we_tag then w.tag else rb_tag
    let xc_tag: rdt_e = if mc & w.we_tag then w.tag else rc_tag
    let xa_ovf: i1 = if ma & w.we_overflow then w.overflow else ra_ovf
    let xb_ovf: i1 = if mb & w.we_overflow then w.overflow else rb_ovf
    let xc_ovf: i1 = if mc & w.we_overflow then w.overflow else rc_ovf
    let xa_flg: i1 = if ma & w.we_flag then w.flag else ra_flg
    let xb_flg: i1 = if mb & w.we_flag then w.flag else rb_flg
    let xc_flg: i1 = if mc & w.we_flag then w.flag else rc_flg

    -- ---- predication ----------------------------------------------------
    -- Zero and negative are computed from the register value on demand; there
    -- is no global condition register (spec 7). This is what the third read
    -- port exists for, and reading it is no longer free of consequence.
    var cond_raw: i1 = 1'b1
    match uop.cond
      .CCK_OVERFLOW =>
        cond_raw = xc_ovf
      .CCK_FLAG =>
        cond_raw = xc_flg
      .CCK_ZERO =>
        cond_raw = xc_value == 32'd0
      .CCK_NEGATIVE =>
        cond_raw = xc_value[31]
      .CCK_POSITIVE =>
        cond_raw = (xc_value != 32'd0) & (!xc_value[31])
      _ =>
        cond_raw = 1'b1
    let unpredicated: i1 = uop.cond == CCK_NONE
    let cond_met: i1 = if unpredicated then 1'b1 else cond_raw ^ uop.cond_invert

    -- ---- reserved F32 operands -------------------------------------------
    -- `F32` is reserved (spec 11), so reading a register carrying the tag
    -- faults. Expressed as "which read ports does this uop architecturally
    -- use", because that is what the check physically is -- the ports read
    -- every cycle regardless, so an unqualified test would fault on registers
    -- the instruction never looks at.
    --
    -- The emulator's `check_no_f32_operands` enumerates the same set in the
    -- same A-before-B order, so `FAULT_INFO` agrees when both carry the tag.
    var reads_a: i1 = @zeroed()
    var reads_b: i1 = @zeroed()
    match uop.kind
      .UOP_COPY | .UOP_LOAD | .UOP_CSP_LOAD | .UOP_BEXT =>
        reads_b = 1'b1
      .UOP_STORE | .UOP_CSP_STORE | .UOP_BINS =>
        reads_a = 1'b1
        reads_b = 1'b1
      .UOP_UNARY =>
        reads_a = 1'b1
      .UOP_ARITH | .UOP_LOGIC | .UOP_SHIFT | .UOP_CMP =>
        reads_a = 1'b1
        reads_b = !uop.use_imm
      -- The link register is written, not read.
      .UOP_PREP_JUMP =>
        reads_a = uop.jump_kind != JT_REL_IMM
      -- NOP, PUT_IMM, SET_TAG, PERFORM_JUMP, HALT and FAULT read no register.
      -- `uop_fault` clears `dst` and `src`, so a decode fault reports its own
      -- cause rather than a tag it never read.
      _ =>
        reads_a = 1'b0

    let f32_a: i1 = reads_a & (xa_tag == RDT_F32)
    let f32_b: i1 = reads_b & (xb_tag == RDT_F32)
    -- The predicate register is read whatever the predicate decides, so its
    -- tag is checked outside the `cond_met` gate.
    let f32_cond: i1 = (!unpredicated) & (xc_tag == RDT_F32)

    -- ---- functional units ----------------------------------------------
    -- The two verified helpers, called rather than reimplemented. An
    -- immediate carries no tag of its own, so it takes the left operand's
    -- interpretation.
    let alu_b: i32 = if uop.use_imm then uop.imm else xb_value
    let alu_b_tag: rdt_e = if uop.use_imm then xa_tag else xb_tag
    let (arith_result, arith_ovf, logic_result, cmp_result, unary_result) = k2g_alu(
        xa_value, alu_b, xa_tag, alu_b_tag,
        uop.arith_op, uop.logic_op, uop.cmp_op, uop.unary_op)

    let amount: i5 = if uop.use_imm then uop.imm[4..0] else xb_value[4..0]
    let (shift_result, bext_result, bins_result) = k2g_shift(
        xa_value, xb_value, amount, uop.shift_op, uop.bm_start, uop.bm_span)

    -- ---- the address adder ----------------------------------------------
    -- read -> forward -> add, in one cycle. A synchronous register read would
    -- put a pipeline stage in the middle of this.
    let load_addr: i32 = xb_value + uop.imm
    let store_addr: i32 = xa_value + uop.imm
    let is_load: i1 = uop.kind == UOP_LOAD
    let access_addr: i32 = if is_load then load_addr else store_addr
    -- A store takes its width from the SOURCE register's tag, not the opcode.
    let access_tag: rdt_e = if is_load then uop.datakind else xb_tag

    -- The FLAG prefix makes a logic op work on flag bits instead of values
    -- (k2g_core.sv:981). Computed here so the writeback arm is a choice
    -- between two ready answers.
    var flag_logic: i1 = @zeroed()
    match uop.logic_op
      .LOGIC_AND =>
        flag_logic = xa_flg & xb_flg
      .LOGIC_OR =>
        flag_logic = xa_flg | xb_flg
      _ =>
        flag_logic = xa_flg ^ xb_flg

    -- ---- alignment and range ---------------------------------------------
    -- Checked before anything commits, which is what makes faults precise
    -- (spec 8).
    let access_width: i3 = rdt_width_bytes(access_tag)
    let is_store: i1 = uop.kind == UOP_STORE
    let is_access: i1 = is_load | is_store

    var misaligned: i1 = @zeroed()
    if access_width == 3'd1 then
      misaligned = 1'b0
    else
      if access_width == 3'd2 then
        misaligned = access_addr[0]
      else
        misaligned = access_addr[1] | access_addr[0]

    -- The last byte the access touches has to be inside memory too, so the
    -- width is added before the comparison rather than after it.
    let last_byte: i32 = access_addr + @zext(access_width, 32)
    let out_of_range: i1 = last_byte > mem_bytes

    -- ---- fault detection --------------------------------------------------
    -- A PRIORITY CHAIN, and the order is architecture rather than taste. The
    -- emulator rejects an `F32` operand before the instruction executes, so an
    -- `F32` address register faults as FP rather than as whatever address it
    -- would have computed -- which puts the operand check ahead of the access
    -- checks and not after them.
    --
    -- `f32_cond` sits outside the `cond_met` gate: the predicate register is
    -- read whatever the predicate decides.
    var fault_valid: i1 = @zeroed()
    var fault: fault_e = FAULT_NONE
    var fault_info: i32 = @zeroed()

    if got & f32_cond then
      fault_valid = 1'b1
      fault = FAULT_FP_UNIMPLEMENTED
      fault_info = @zext(uop.cond_reg, 32)
    else
      if got & cond_met then
        if uop.kind == UOP_FAULT then
          fault_valid = 1'b1
          fault = uop.fault
          -- The offending code point, carried through decode in `imm`.
          fault_info = uop.imm
        else
          if f32_a | f32_b then
            fault_valid = 1'b1
            fault = FAULT_FP_UNIMPLEMENTED
            fault_info = if f32_a then @zext(uop.dst, 32) else @zext(uop.src, 32)
          else
            if is_access & misaligned then
              fault_valid = 1'b1
              fault = if is_load then FAULT_MISALIGNED_LOAD else FAULT_MISALIGNED_STORE
              fault_info = access_addr
            else
              if is_access & out_of_range then
                fault_valid = 1'b1
                fault = FAULT_ADDR_OUT_OF_RANGE
                fault_info = access_addr
              else
                fault_valid = 1'b0

    -- Nothing commits behind a fault, and nothing commits on a predicate that
    -- did not hold. One term, used by every arm of the writeback below, which
    -- is what keeps a new opcode from forgetting it.
    let commits: i1 = got & cond_met & (!fault_valid)

    -- ---- the writeback packet -------------------------------------------
    -- `n_raw` is the value before normalization and `n_norm` the tag to
    -- normalize it to. They are NOT the same as `n.tag`: a shift normalizes to
    -- the operand's tag while writing back the destination's, so the two
    -- differ on three branches (k2g_core.sv:880).
    var n: wb_t = @zeroed()
    var n_raw: i32 = @zeroed()
    var n_norm: rdt_e = RDT_U32
    n.addr = uop.dst
    n.tag = uop.datakind
    n.overflow = 1'b0
    n.flag = 1'b0
    n.fault_valid = fault_valid
    n.fault = fault
    n.fault_info = fault_info

    match uop.kind
      .UOP_PUT_IMM =>
        -- Put-constant clears both flags (k2g_core.sv:932).
        n.we_value = commits
        n.we_tag = commits
        n.we_overflow = commits
        n.we_flag = commits
        n_raw = uop.imm
        n_norm = uop.datakind
        n.tag = uop.datakind
      .UOP_SET_TAG =>
        -- RDT re-normalizes, which is what makes LD8 + RDT S8 a
        -- sign-extending byte load (spec 5.10). It writes the VALUE as well as
        -- the tag, which is the whole point of it.
        n.we_value = commits
        n.we_tag = commits
        n_raw = xa_value
        n_norm = uop.datakind
        n.tag = uop.datakind
      .UOP_COPY =>
        -- A full move: value, tag and both flags (spec 5.2). The value is
        -- already normalized to the tag it arrives with.
        n.we_value = commits
        n.we_tag = commits
        n.we_overflow = commits
        n.we_flag = commits
        n_raw = xb_value
        n.tag = xb_tag
        n.overflow = xb_ovf
        n.flag = xb_flg
      .UOP_ARITH =>
        n.we_value = commits
        n.we_overflow = commits
        n_raw = arith_result
        n.overflow = arith_ovf
      .UOP_LOGIC =>
        if uop.on_flags then
          n.we_flag = commits
          n.flag = flag_logic
        else
          n.we_value = commits
          n_raw = logic_result
      .UOP_UNARY =>
        if uop.on_flags then
          n.we_flag = commits
          n.flag = !xa_flg
        else
          n.we_value = commits
          n_raw = unary_result
          n_norm = xa_tag
      .UOP_CMP =>
        -- Comparisons write only flag_bit of the left operand (spec 5.9).
        n.we_flag = commits
        n.flag = cmp_result
      .UOP_SHIFT =>
        n.we_value = commits
        n_raw = shift_result
        n_norm = xa_tag
      .UOP_BEXT =>
        -- Extract writes the whole register; insert writes only the value
        -- (spec 5.8).
        n.we_value = commits
        n.we_tag = commits
        n.we_overflow = commits
        n.we_flag = commits
        n_raw = bext_result
        n.tag = RDT_U32
      .UOP_BINS =>
        n.we_value = commits
        n_raw = bins_result
      .UOP_LOAD | .UOP_STORE =>
        n.we_value = 1'b0
      _ =>
        n.we_value = 1'b0

    -- ONE normalize, after the mux, rather than one per source. Six branches
    -- would otherwise put a sign-extend in front of six of the mux's inputs.
    n.value = rdt_normalize(n_raw, n_norm)

    -- A load's value comes back from the memory stage, which is not in this
    -- slice; the core writes nothing here. The address and the tag that will
    -- decide the store width are published instead, so the adder above and the
    -- width decision are both observable.
    if is_access then
      n.value = access_addr
      n.tag = access_tag

    -- ---- the array write port -------------------------------------------
    -- Driven by W, not by X. That is what makes the read above see the value
    -- as of the start of the cycle, and it is why the forwarding mux exists.
    if w.we_value then
      values[w.addr] = w.value
    if w.we_tag then
      @assert(w.tag != RDT_UNCLAIMED_7, "the reserved tag encoding was written")
      tags[w.addr] = w.tag
    if w.we_overflow then
      overflow[w.addr] = w.overflow
    if w.we_flag then
      flagbit[w.addr] = w.flag

    w = n
    @try_send(wb, n)
