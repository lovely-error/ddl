-- K2G instruction decoder: the prefix accumulator, in DDL.
--
-- A port of rtl/k2g_decode.sv. Consumes one 16-bit code point per cycle.
-- Prefix code points fold into an accumulator; the first non-prefix code point
-- is the main opcode and completes the instruction, at which point a `uop_t`
-- is emitted. Long-constant forms (LLC_*) additionally consume one or two
-- trailing literal code points.
--
-- Prefixes that *reinterpret* their main opcode are resolved here, so the
-- execute stage never sees a prefix: BMX+SHL is an extract, UTO+MUL is a
-- widening multiply, CSP+LD is a port read, and MPD/MPI+TST and ESP+ST are
-- no-ops rather than the compares and stores they would otherwise be (spec
-- 3.2). ISTORE+ST is the one that is NOT reinterpreted: it is a real store,
-- and what the prefix adds is a flag saying where the bytes have to become
-- visible (spec 3.2.1).
--
-- Reads of a `var` see the value at the start of the cycle plus whatever this
-- body has already assigned, so the SystemVerilog's `pfx_next = pfx;` opening
-- line has no counterpart here -- that is what the shadow variable was for.

-- Accumulated prefix state. Field order matches pfx_t in k2g_decode.sv.
import "k2g_types.ddl"

struct pfx_t
  xi_valid: u1
  xi_zext: u1              -- XIZEXT rather than XI (spec 4.1)
  xi_value: u32            -- already sign- or zero-extended

  cond: cond_kind_e
  cond_reg: u5
  cond_invert: u1

  uto_valid: u1
  uto_reg: u5

  bmx_valid: u1
  bm_start: u5
  bm_span: u5

  flag: u1
  csp: u1
  istore: u1
  esp: u1
  prefetch_d: u1
  prefetch_i: u1
  ordering: u1             -- any of LTL/SL/SS/SR: ignored, but recorded

  count: u4                -- prefix code points consumed
  bytes: u32               -- total code point bytes consumed

-- LLC forms need trailing literal code points; the FSM waits for them.
enum state_e: u2
  S_PREFIX
  S_LLC_HI
  S_LLC_LO

-- ---- helpers -------------------------------------------------------------

-- (The condition-code decode lives inside the process, in one match that
-- produces both values -- see below. Two functions would each repeat the same
-- five comparisons, which the SystemVerilog avoids by computing them in one
-- always_comb.)

-- One code point, and whether it starts a new instruction stream.
--
-- The SystemVerilog carries the redirect on a separate `flush` wire, and that
-- leaves a question nothing answers: when a flush and a code point arrive on
-- the same cycle, which happened first? Today the code point is accepted --
-- `accept` does not consult `flush` -- and then thrown away by the reset
-- branch, so the producer is told an item was delivered and it was not.
--
-- Carried IN the stream it redirects, the ordering is a fact of the channel
-- rather than a convention, and nothing is accepted-then-discarded. It also
-- stops the redirect being something that can be dropped: a lost flush would
-- leave the accumulator holding prefixes from an abandoned path and decode the
-- new stream as a continuation of it.
struct cp_item_t
  code: u16
  restart: u1

process k2g_decode (
    -- The code point stream. `cps_valid` / `cps_ready` / `cps_data` are
    -- generated: the SystemVerilog spells the same handshake by hand as
    -- `cp_valid` and `accept`.
    cps: buffer in cp_item_t,

    -- Micro-ops out. This is a `buffer`, and that is what removes `hold`.
    --
    -- The SystemVerilog needs a separate `hold` input because `uop_valid` has
    -- no `ready` beside it: with no way for the sink to refuse an item, the
    -- only way to stop the accumulator was a second signal that freezes it.
    -- Its own comment records the trap that then opens -- gating `cp_valid`
    -- with the stall instead would close a loop through
    -- stall -> decode -> CSP request -> stall. A back-pressured channel has
    -- no such choice to get wrong: `cps_ready` falls out of the slot being
    -- full, `uop_valid` is a register, and rule 3 holds by construction.
    uop: buffer out uop_t)

  var pfx: pfx_t = @zeroed()
  var state: state_e = S_PREFIX
  var llc_dst: u5 = @zeroed()
  var llc_kind: rdt_e = RDT_U32
  var llc_hi: u16 = @zeroed()

  -- The process is a program: it runs once and stops. `loop` is what makes
  -- it repeat, once per cycle, for as long as the design runs.
  loop

    -- The item, and whether one transferred this cycle. `cp_valid` is the
    -- TRANSFER, not the offer: it is already `cps_valid && cps_ready`, so it is
    -- false on a cycle the sink is refusing -- which is exactly what
    -- `cp_valid && !hold` used to spell out.
    let (item, cp_valid) = @try_rcv(cps)
    let cp: u16 = item.code

    -- The redirect rides with the code point that begins the new stream, so it
    -- cannot arrive out of order with it and cannot be lost.
    let flushing: u1 = cp_valid & item.restart

    -- The register values as of this clock edge. The body mutates the registers
    -- freely; these are what gets restored when the update is not taken, which
    -- is how `hold` freezes the accumulator without suppressing the decode.
    let pfx_held: pfx_t = pfx
    let state_held: state_e = state
    let llc_dst_held: u5 = llc_dst
    let llc_kind_held: rdt_e = llc_kind
    let llc_hi_held: u16 = llc_hi
    let bytes_held: u32 = pfx.bytes

    -- ---- field extraction --------------------------------------------------
    let lb: lb_e = @cast(cp[15..10])
    let arg1: u5 = cp[9..5]
    let arg2: u5 = cp[4..0]
    let imm10: u10 = cp[9..0]

    -- ---- prefix classification ---------------------------------------------
    -- EP1 selects on arg2; EP2 escapes again and selects on arg1. Note the field
    -- swap between the two levels (spec 2).
    let is_ep1: u1 = lb == LB_EP1
    let ep1_op: ep1_e = @cast(arg2)
    let ep2_op: ep2_e = @cast(arg1)
    let is_ep2: u1 = is_ep1 & (ep1_op == EP1_EP2)
    let is_ep1_only: u1 = is_ep1 & !is_ep2

    let pfx_xi: u1 = (lb == LB_XI) | (lb == LB_XIZEXT)
    let pfx_bmx: u1 = lb == LB_BMX_0_0
    let pfx_xc: u1 = (lb == LB_XCP) | (lb == LB_XCN)
    let pfx_uto: u1 = is_ep1_only & (ep1_op == EP1_UTO)
    let pfx_order: u1 = is_ep1_only &
        ((ep1_op == EP1_SS) | (ep1_op == EP1_SR) |
         (ep1_op == EP1_SL) | (ep1_op == EP1_LTL))
    let pfx_esp: u1 = is_ep1_only & (ep1_op == EP1_ESP)
    let pfx_flag: u1 = is_ep2 & (ep2_op == EP2_FLAG)
    let pfx_csp: u1 = is_ep2 & (ep2_op == EP2_CSP)
    let pfx_mpi: u1 = is_ep2 & (ep2_op == EP2_MPI)
    let pfx_mpd: u1 = is_ep2 & (ep2_op == EP2_MPD)
    let pfx_istore: u1 = is_ep2 & (ep2_op == EP2_ISTORE)

    let is_prefix: u1 = pfx_xi | pfx_bmx | pfx_xc | pfx_uto | pfx_order |
        pfx_esp | pfx_flag | pfx_csp | pfx_mpi | pfx_mpd | pfx_istore

    -- A condition code outside the assigned set makes XCP/XCN illegal. The
    -- emulator used to fall back to treating the code point as a main opcode,
    -- which then decoded as garbage; here it faults (spec 7).
    var xc_cond: cond_kind_e = CCK_NONE
    var xc_cond_ok: u1 = 1'b1
    let cc: cc_e = @cast(arg2)
    match cc
      .CC_OVERFLOW_SET =>
        xc_cond = CCK_OVERFLOW
      .CC_FLAG_SET =>
        xc_cond = CCK_FLAG
      .CC_VALUE_ZERO =>
        xc_cond = CCK_ZERO
      .CC_VALUE_NEGATIVE =>
        xc_cond = CCK_NEGATIVE
      .CC_VALUE_POSITIVE =>
        xc_cond = CCK_POSITIVE
      _ =>
        xc_cond = CCK_NONE
        xc_cond_ok = 1'b0

    -- ---- immediate assembly ------------------------------------------------
    -- XI sign-extends from bit 9, XIZEXT zero-extends (spec 4.1).
    let imm10_sext: u32 = {@rep(imm10[9], 22), imm10}
    let imm10_zext: u32 = {22'd0, imm10}
    let xi_ext: u32 = if lb == LB_XI then imm10_sext else imm10_zext

    -- Three different combining rules depending on the main opcode (spec 4).
    let imm_alu: u32 = if pfx.xi_valid
        then {pfx.xi_value[26..0], arg2}
        else {@rep(arg2[4], 27), arg2}
    let imm_mem: u32 = if pfx.xi_valid then pfx.xi_value else 32'd0

    -- DISPI assembles a 20-bit displacement then shifts left by one. The
    -- extension follows the prefix that supplied it -- previously this
    -- sign-extended unconditionally, so XIZEXT could yield a negative
    -- displacement (spec 4.1).
    let disp20: u20 = {pfx.xi_value[9..0], imm10}
    let disp_from_xi: u32 = if pfx.xi_zext
        then {12'd0, disp20}
        else {@rep(disp20[19], 12), disp20}
    let disp_ext: u32 = if pfx.xi_valid then disp_from_xi else imm10_sext
    let disp_bytes: u32 = {disp_ext[30..0], 1'b0}

    -- ---- main decode -------------------------------------------------------
    var main_uop: uop_t = @zeroed()
    var main_is_llc: u1 = 1'b0
    var main_llc_kind: rdt_e = RDT_U32

    main_uop = @zeroed()
    main_is_llc = 1'b0
    main_llc_kind = RDT_U32

    -- Carry accumulated prefix state onto every decoded instruction.
    main_uop.cond = pfx.cond
    main_uop.cond_reg = pfx.cond_reg
    main_uop.cond_invert = pfx.cond_invert
    main_uop.uto_valid = pfx.uto_valid
    main_uop.uto_reg = pfx.uto_reg
    main_uop.on_flags = pfx.flag
    main_uop.bm_start = pfx.bm_start
    main_uop.bm_span = pfx.bm_span
    main_uop.dst = arg1
    main_uop.src = arg2

    match lb
      -- ---- put constant ----
      .LB_PUC8 | .LB_PUC16 | .LB_PUC32 =>
        main_uop.kind = UOP_PUT_IMM
        main_uop.imm = imm_alu
        main_uop.datakind = if lb == LB_PUC8 then RDT_U8
            else if lb == LB_PUC16 then RDT_U16 else RDT_U32

      .LB_RDT =>
        -- 3'b111 is the one unassigned tag encoding.
        if (arg2[2..0] == 3'b111) | (arg2[4..3] != 2'b00) then
          main_uop = uop_fault(FAULT_ILLEGAL_OPCODE, cp)
        else
          main_uop.kind = UOP_SET_TAG
          main_uop.datakind = @cast(arg2[2..0])

      .LB_CPY =>
        main_uop.kind = UOP_COPY

      -- ---- loads ----
      .LB_LD8 | .LB_LD16 | .LB_LD32 | .LB_LDF32 =>
        main_uop.datakind = if lb == LB_LD8 then RDT_U8
            else if lb == LB_LD16 then RDT_U16
            else if lb == LB_LD32 then RDT_U32 else RDT_F32
        if lb == LB_LDF32 then
          main_uop = uop_fault(FAULT_FP_UNIMPLEMENTED, cp)   -- reserved (spec 11)
        else
          if pfx.csp then
            -- CSP+LD: port number in arg2, destination arg1 (spec 9).
            main_uop.kind = UOP_CSP_LOAD
          else
            main_uop.kind = UOP_LOAD
            main_uop.imm = imm_mem

      -- ---- stores, and the prefixes that replace them ----
      .LB_ST =>
        if pfx.esp then
          -- Still honoured as a no-op. Ignoring the prefix would perform a
          -- real store, which is a silent wrong answer (spec 3.2, 10).
          main_uop.kind = UOP_NOP
        else
          if pfx.csp then
            -- CSP+ST: port number in arg1, data in arg2 -- the opposite
            -- operand positions from the load form.
            main_uop.kind = UOP_CSP_STORE
          else
            -- A store either way. `is_insn` says the bytes are instructions,
            -- so they must reach shared memory and the instruction side must
            -- be told (spec 3.2.1) -- it does not change what is written or
            -- where.
            main_uop.kind = UOP_STORE
            main_uop.imm = imm_mem
            main_uop.is_insn = pfx.istore

      -- ---- arithmetic ----
      .LB_ADD | .LB_SUB | .LB_MUL | .LB_DIV =>
        if lb == LB_DIV then
          main_uop = uop_fault(FAULT_DIV_UNIMPLEMENTED, cp)   -- cut from v1
        else
          main_uop.kind = UOP_ARITH
          main_uop.arith_op = if lb == LB_ADD then ARITH_ADD
              else if lb == LB_SUB then ARITH_SUB else ARITH_MUL
          main_uop.use_imm = pfx.xi_valid
          main_uop.imm = imm_alu

      -- ---- logic ----
      .LB_AND | .LB_OR | .LB_XOR =>
        main_uop.kind = UOP_LOGIC
        main_uop.logic_op = if lb == LB_AND then LOGIC_AND
            else if lb == LB_OR then LOGIC_OR else LOGIC_XOR
        main_uop.use_imm = pfx.xi_valid
        main_uop.imm = imm_alu
        -- There is no immediate flag bit, so FLAG with an immediate is
        -- meaningless (spec 5.6).
        if pfx.flag & pfx.xi_valid then
          main_uop = uop_fault(FAULT_ILLEGAL_PREFIX_COMBO, cp)

      -- ---- shifts, and the bit-field ops that replace them ----
      .LB_SHL | .LB_SHR | .LB_SHRA =>
        if pfx.bmx_valid then
          if lb == LB_SHL then
            main_uop.kind = UOP_BEXT
            if pfx.bm_span == 5'd0 then
              main_uop = uop_fault(FAULT_BEXT_ZERO_SPAN, cp)
          else
            if lb == LB_SHR then
              main_uop.kind = UOP_BINS      -- span 0 is a defined no-op
            else
              main_uop = uop_fault(FAULT_ILLEGAL_PREFIX_COMBO, cp)
        else
          main_uop.kind = UOP_SHIFT
          main_uop.shift_op = if lb == LB_SHL then SHIFT_LL
              else if lb == LB_SHR then SHIFT_LR else SHIFT_AR

      -- Immediate shift amounts ignore XI entirely (spec 4).
      .LB_SHLI | .LB_SHRI | .LB_SHRAI =>
        main_uop.kind = UOP_SHIFT
        main_uop.shift_op = if lb == LB_SHLI then SHIFT_LL
            else if lb == LB_SHRI then SHIFT_LR else SHIFT_AR
        main_uop.use_imm = 1'b1
        main_uop.imm = {27'd0, arg2}

      -- ---- comparisons, and the prefetch that replaces them ----
      .LB_TST | .LB_TSTN | .LB_LT | .LB_GT | .LB_LTE | .LB_GTE =>
        if pfx.prefetch_d | pfx.prefetch_i then
          -- Also a no-op that must be honoured: ignoring it would perform a
          -- real compare and write a flag bit (spec 3.2).
          main_uop.kind = UOP_NOP
        else
          main_uop.kind = UOP_CMP
          main_uop.use_imm = pfx.xi_valid
          main_uop.imm = imm_alu
          main_uop.cmp_op = if lb == LB_TST then CMP_EQ
              else if lb == LB_TSTN then CMP_NE
              else if lb == LB_LT then CMP_LT
              else if lb == LB_GT then CMP_GT
              else if lb == LB_LTE then CMP_LE else CMP_GE

      -- ---- control transfer ----
      .LB_DISP =>
        main_uop.kind = UOP_PREP_JUMP
        if arg2 == EP1_DISP_OFF then
          main_uop.jump_kind = JT_REL_REG
        else
          if arg2 == EP1_DISP_ABS then
            main_uop.jump_kind = JT_ABS_REG
          else
            main_uop = uop_fault(FAULT_ILLEGAL_OPCODE, cp)

      .LB_DISPI =>
        main_uop.kind = UOP_PREP_JUMP
        main_uop.jump_kind = JT_REL_IMM
        main_uop.imm = disp_bytes

      -- ---- escapes ----
      .LB_EP1 =>
        if is_ep2 then
          if ep2_op == EP2_HALT then
            main_uop.kind = UOP_HALT
          else
            if ep2_op == EP2_DNO then
              main_uop.kind = UOP_NOP
            else
              if ep2_op == EP2_TC then
                -- A predicated TC is rejected. The predicated-branch idiom
                -- puts its condition on the PRIME, so an unprimed TC is the
                -- defined no-op (spec 6.5); predicating the TC itself adds
                -- nothing and makes the jump depend on state the front end
                -- cannot see.
                if pfx.cond != CCK_NONE then
                  main_uop = uop_fault(FAULT_ILLEGAL_PREFIX_COMBO, cp)
                else
                  main_uop.kind = UOP_PERFORM_JUMP
              else
                main_uop = uop_fault(FAULT_ILLEGAL_OPCODE, cp)
        else
          if (ep1_op == EP1_NOT) | (ep1_op == EP1_NEG) then
            main_uop.kind = UOP_UNARY
            main_uop.unary_op = if ep1_op == EP1_NOT then UNARY_NOT else UNARY_NEG
            -- Negating a single bit is meaningless (spec 5.6).
            if pfx.flag & (ep1_op == EP1_NEG) then
              main_uop = uop_fault(FAULT_ILLEGAL_PREFIX_COMBO, cp)
          else
            if (ep1_op == EP1_LLC_B8) | (ep1_op == EP1_LLC_B16) then
              main_is_llc = 1'b1
              main_llc_kind = if ep1_op == EP1_LLC_B8 then RDT_U8 else RDT_U16
            else
              if ep1_op == EP1_LLC_B32 then
                main_is_llc = 1'b1
                main_llc_kind = RDT_U32
              else
                if ep1_op == EP1_LLC_F32 then
                  main_uop = uop_fault(FAULT_FP_UNIMPLEMENTED, cp)
                else
                  main_uop = uop_fault(FAULT_ILLEGAL_OPCODE, cp)

      _ =>
        main_uop = uop_fault(FAULT_ILLEGAL_OPCODE, cp)

    -- ---- next-state logic --------------------------------------------------
    var out_uop: uop_t = @zeroed()
    var emit: u1 = 1'b0

    out_uop = @zeroed()
    emit = 1'b0

    if cp_valid then
      pfx.bytes = pfx.bytes + 32'd2

      if state == S_PREFIX then
        if is_prefix then
          -- A chain longer than the bound is a fault rather than a hang; the
          -- emulator's loop was previously unbounded (spec 3).
          if pfx.count >= 4'd7 then
            out_uop = uop_fault(FAULT_PREFIX_CHAIN_TOO_LONG, cp)
            emit = 1'b1
          else
            pfx.count = pfx.count + 4'd1
            if pfx_xi then
              pfx.xi_valid = 1'b1
              pfx.xi_zext = lb == LB_XIZEXT
              pfx.xi_value = xi_ext
            if pfx_bmx then
              pfx.bmx_valid = 1'b1
              pfx.bm_start = arg1
              pfx.bm_span = arg2
            if pfx_xc then
              if xc_cond_ok then
                pfx.cond = xc_cond
                pfx.cond_reg = arg1
                pfx.cond_invert = lb == LB_XCN
              else
                out_uop = uop_fault(FAULT_ILLEGAL_PREFIX_COMBO, cp)
                emit = 1'b1
            if pfx_uto then
              pfx.uto_valid = 1'b1
              pfx.uto_reg = arg1
            if pfx_flag then
              pfx.flag = 1'b1
            if pfx_csp then
              pfx.csp = 1'b1
            if pfx_istore then
              pfx.istore = 1'b1
            if pfx_esp then
              pfx.esp = 1'b1
            if pfx_mpd then
              pfx.prefetch_d = 1'b1
            if pfx_mpi then
              pfx.prefetch_i = 1'b1
            if pfx_order then
              pfx.ordering = 1'b1
        else
          if main_is_llc then
            llc_dst = arg1
            llc_kind = main_llc_kind
            state = if main_llc_kind == RDT_U32 then S_LLC_HI else S_LLC_LO
          else
            out_uop = main_uop
            emit = 1'b1
      else
        if state == S_LLC_HI then
          llc_hi = cp
          state = S_LLC_LO
        else
          -- S_LLC_LO: the final literal code point completes the constant.
          out_uop.kind = UOP_PUT_IMM
          out_uop.dst = llc_dst
          out_uop.datakind = llc_kind
          out_uop.imm = if llc_kind == RDT_U32
              then {llc_hi, cp}
              else {16'd0, cp}
          out_uop.cond = pfx.cond
          out_uop.cond_reg = pfx.cond_reg
          out_uop.cond_invert = pfx.cond_invert
          emit = 1'b1
          state = S_PREFIX

    -- The instruction's total length, needed for PC advance and the link
    -- register. Counted here rather than in fetch so the two cannot disagree.
    -- Computed from the register, not from the copy the body may have already
    -- incremented, so it does not depend on `cp_valid`.
    out_uop.size_bytes = bytes_held + 32'd2

    -- A decoder does not produce a micro-op every cycle: a prefix accumulates
    -- and emits nothing. The offer is made only on the cycles that complete an
    -- instruction, and the generated handshake turns that into the write enable
    -- on the output slot.
    if emit then
      @try_send(uop, out_uop)

    -- ---- register update ---------------------------------------------------
    -- Mirrors the always_ff of k2g_decode.sv: `flush` abandons a partially
    -- accumulated instruction on a branch redirect and resets everything;
    -- otherwise nothing moves unless a code point is actually consumed.
    let update: u1 = cp_valid

    if flushing then
      pfx = @zeroed()
      state = S_PREFIX
      llc_dst = @zeroed()
      llc_kind = RDT_U32
      llc_hi = @zeroed()
    else
      if !update then
        pfx = pfx_held
        state = state_held
        llc_dst = llc_dst_held
        llc_kind = llc_kind_held
        llc_hi = llc_hi_held
      else
        -- Instruction complete: start the next one clean. The llc_* registers
        -- keep the values the body left, exactly as the SystemVerilog does.
        if emit then
          pfx = @zeroed()
          state = S_PREFIX
