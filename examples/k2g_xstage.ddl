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
-- WHAT THIS IS NOT. Predication and condition codes, fault detection, the CSP
-- port, multi-cycle sequencing, jumps, the memory stage and its byte-enable
-- network, and the widening multiply are all left in k2g_core.sv. The point
-- here is the register file and the path that depends on its read timing, not
-- a whole core.
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

process k2g_xstage (uops: buffer in uop_t, wb: buffer out wb_t)
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

    -- ---- the writeback packet -------------------------------------------
    var n: wb_t = @zeroed()
    n.addr = uop.dst
    n.value = 32'd0
    n.tag = xa_tag
    n.overflow = 1'b0
    n.flag = 1'b0

    match uop.kind
      .UOP_PUT_IMM =>
        n.we_value = got
        n.we_tag = got
        n.value = uop.imm
        n.tag = uop.datakind
      .UOP_SET_TAG =>
        n.we_tag = got
        n.tag = uop.datakind
      .UOP_COPY =>
        n.we_value = got
        n.we_tag = got
        n.we_overflow = got
        n.we_flag = got
        n.value = xb_value
        n.tag = xb_tag
        n.overflow = xb_ovf
        n.flag = xb_flg
      .UOP_ARITH =>
        n.we_value = got
        n.we_overflow = got
        n.value = arith_result
        n.overflow = arith_ovf
      .UOP_LOGIC =>
        n.we_value = got
        n.value = logic_result
      .UOP_UNARY =>
        n.we_value = got
        n.value = unary_result
      .UOP_CMP =>
        n.we_flag = got
        n.flag = cmp_result
      .UOP_SHIFT =>
        n.we_value = got
        n.value = shift_result
      .UOP_BEXT =>
        n.we_value = got
        n.value = bext_result
      .UOP_BINS =>
        n.we_value = got
        n.value = bins_result
      -- A load's address is what X produces; the value comes back from the
      -- memory stage, which is not in this slice. The address is published so
      -- the adder above is observable.
      .UOP_LOAD | .UOP_STORE =>
        n.we_value = 1'b0
        n.value = access_addr
        n.tag = access_tag
      _ =>
        n.we_value = 1'b0

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
