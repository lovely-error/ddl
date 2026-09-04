-- K2G core types, in DDL.
--
-- The counterpart of rtl/k2g_types.svh. The ISA opcode enums (lb_e, ep1_e,
-- ep2_e, cc_e, rdt_e) are NOT here -- they are generated into k2g_pkg.ddl from
-- the emulator by emu/src/ddl_gen.rs, because a hand-maintained copy of an
-- opcode map is exactly what this project already paid for once.
--
-- Widths are written explicitly on every enum so they match the SystemVerilog
-- declaration rather than the narrowest tag that would hold the variants. A
-- struct field's width is part of the layout, not an implementation detail.

import "k2g_pkg.ddl"   -- generated from the emulator; -I $K2G/rtl to find it

enum fault_e: u5
  FAULT_NONE                  = 5'h00
  FAULT_ILLEGAL_OPCODE        = 5'h01
  FAULT_ILLEGAL_PREFIX_COMBO  = 5'h02
  FAULT_PREFIX_CHAIN_TOO_LONG = 5'h03
  FAULT_DIV_UNIMPLEMENTED     = 5'h04
  FAULT_FP_UNIMPLEMENTED      = 5'h05
  FAULT_MISALIGNED_LOAD       = 5'h06
  FAULT_MISALIGNED_STORE      = 5'h07
  FAULT_MISALIGNED_FETCH      = 5'h08
  FAULT_ADDR_OUT_OF_RANGE     = 5'h09
  FAULT_BAD_CSP_PORT          = 5'h0A
  FAULT_BEXT_ZERO_SPAN        = 5'h0B
  FAULT_HALT_INSN             = 5'h1F

-- One per arm of the emulator's CanonInsn. Flat, so execute is a single match
-- rather than a nest of prefix conditionals -- every reinterpretation
-- (BMX+SHL becoming an extract, CSP+LD a port read) is resolved during decode.
enum uop_kind_e: u5
  UOP_NOP
  UOP_PUT_IMM
  UOP_SET_TAG
  UOP_COPY
  UOP_LOAD
  UOP_STORE
  UOP_CSP_LOAD
  UOP_CSP_STORE
  UOP_ARITH
  UOP_LOGIC
  UOP_SHIFT
  UOP_BEXT
  UOP_BINS
  UOP_CMP
  UOP_PREP_JUMP
  UOP_PERFORM_JUMP
  UOP_UNARY
  UOP_HALT
  UOP_FAULT

enum cond_kind_e: u3
  CCK_NONE
  CCK_OVERFLOW
  CCK_FLAG
  CCK_ZERO
  CCK_NEGATIVE
  CCK_POSITIVE

enum arith_e: u2
  ARITH_ADD
  ARITH_SUB
  ARITH_MUL
  ARITH_DIV

enum logic_e: u2
  LOGIC_AND
  LOGIC_OR
  LOGIC_XOR

enum shift_e: u2
  SHIFT_LL
  SHIFT_LR
  SHIFT_AR

enum cmp_e: u3
  CMP_EQ
  CMP_NE
  CMP_LT
  CMP_GT
  CMP_LE
  CMP_GE

enum unary_e: u2
  UNARY_NOT
  UNARY_NEG

enum jump_e: u2
  JT_REL_IMM
  JT_REL_REG
  JT_ABS_REG

-- The decoded instruction. Field order is the SystemVerilog declaration order,
-- so the packed layout matches uop_t in k2g_types.svh bit for bit and the two
-- can meet at a module boundary.
struct uop_t
  kind: uop_kind_e

  dst: u5              -- arg1 for most forms
  src: u5              -- arg2 for most forms
  imm: u32             -- immediate / offset / displacement
  use_imm: u1          -- src2 is `imm` rather than register `src`

  -- Predication (spec 7). Resolved during the prefix's own decode cycle.
  cond: cond_kind_e
  cond_reg: u5
  cond_invert: u1

  -- Operation selectors, valid per `kind`.
  arith_op: arith_e
  logic_op: logic_e
  shift_op: shift_e
  cmp_op: cmp_e
  unary_op: unary_e
  jump_kind: jump_e

  -- Modifiers.
  -- ISTORE + ST (spec 3.2.1): this store writes instruction memory. A FLAG on
  -- the store rather than a kind of its own, because everything else about it
  -- -- address, width from the source tag, predication, faults -- is a
  -- store's and stays a store's. What it adds is where the bytes have to
  -- become visible.
  is_insn: u1
  on_flags: u1         -- FLAG prefix: operate on flag_bit
  uto_reg: u5          -- link register / widening-multiply high half
  uto_valid: u1
  datakind: rdt_e      -- for loads / put-constant / retag
  bm_start: u5         -- BMX
  bm_span: u5

  -- Set when decode itself failed; `fault` says why.
  fault: fault_e
  size_bytes: u32

fun uop_nop (u: out uop_t)
  u = @zeroed()

-- `cp` is the code point being decoded when the fault was detected, and it
-- rides in `imm` because a fault uop has no immediate of its own.
fun uop_fault (cause: fault_e, cp: u16, u: out uop_t)
  var f: uop_t = @zeroed()
  f.kind = UOP_FAULT
  f.fault = cause
  f.imm = @concat(16'd0, cp)
  u = f
