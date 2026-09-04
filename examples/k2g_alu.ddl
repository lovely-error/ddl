-- K2G arithmetic, logic and comparison unit, in DDL.
--
-- A direct port of rtl/k2g_alu.sv. Purely combinational; the multiplier lives
-- elsewhere because it is multi-cycle and wants the DSP primitives.
--
-- Two details here are load-bearing and easy to get subtly wrong:
--
--   * `overflow` has one meaning across ADD, SUB and MUL -- "the true result
--     did not fit". For SUB that is a borrow. The emulator originally formed
--     the two's complement in 32 bits and took *that* carry, which made the
--     flag a not-borrow flag and left it wrong for `b == 0` (spec 5.4).
--
--   * Comparisons widen both operands to 33 bits by their OWN tag before
--     comparing. i32 spans [-2^31, 2^31-1] and u32 spans [0, 2^32-1]; their
--     union does not fit in 32 bits, so a 32-bit comparator must misorder some
--     mixed-tag pairs -- the same trap as C's usual arithmetic conversions
--     making `-1 < 1u` false. The extra bit costs about one LUT since the
--     comparator is already a subtractor (spec 5.9).

-- These mirror k2g_types.svh. Discriminants are implicit because the
-- SystemVerilog leaves them implicit too, so the encodings stay in step.
-- The operation enums live in k2g_types.ddl, which this is compiled with.
-- They were duplicated here while this was the only module that needed them;
-- two copies of an opcode map is the exact failure this project already paid
-- for once, so there is one copy now.

import "k2g_types.ddl"

fun rdt_is_signed (t: rdt_e, signed_: out u1)
  signed_ = t[2]

fun k2g_alu (
    a: u32,
    b: u32,
    a_tag: rdt_e,        -- interpretation of the left operand
    b_tag: rdt_e,        -- interpretation of the right operand
    arith_op: arith_e,
    logic_op: logic_e,
    cmp_op: cmp_e,
    unary_op: unary_e,

    arith_result: out u32,
    arith_overflow: out u1,
    logic_result: out u32,
    cmp_result: out u1,
    unary_result: out u32)

  -- ---- add / subtract ----------------------------------------------------
  -- One adder shared between both directions. Widening to 33 bits is explicit
  -- here, exactly as the SystemVerilog writes `{1'b0, a}`; the extra bit is
  -- the carry out for ADD and the borrow for SUB.
  let a33: u33 = @concat(1'b0, a)
  let b33: u33 = @concat(1'b0, b)
  let sum: u33 = a33 + b33
  let diff: u33 = a33 - b33

  -- arith_e fills its two-bit tag exactly, so naming all four variants makes
  -- this exhaustive with no wildcard: add a fifth and the compiler says so,
  -- where `default` would have swallowed it.
  match arith_op
    .ARITH_SUB =>
      arith_result = diff[31..0]
      arith_overflow = diff[32]
    .ARITH_ADD | .ARITH_MUL | .ARITH_DIV =>
      arith_result = sum[31..0]
      arith_overflow = sum[32]

  -- ---- logic -------------------------------------------------------------
  match logic_op
    .LOGIC_AND =>
      logic_result = a & b
    .LOGIC_OR =>
      logic_result = a | b
    .LOGIC_XOR =>
      logic_result = a ^ b
    -- logic_e declares three variants in a two-bit tag, so 2'b11 names no
    -- variant. The decoder never emits it (k2g_types.svh:74).
    _ => @unreachable

  -- ---- unary -------------------------------------------------------------
  -- Truncation to the destination tag happens in the writeback path, not here.
  match unary_op
    .UNARY_NOT =>
      unary_result = ~a
    .UNARY_NEG =>
      unary_result = (~a) + 32'd1
    _ => @unreachable

  -- ---- comparison --------------------------------------------------------
  -- Each operand widens by its own tag: replicate bit 31 if signed, prepend 0
  -- if unsigned. Then one signed 33-bit comparison covers all four tag
  -- combinations exactly.
  let a_signed: u1 = rdt_is_signed(a_tag)
  let b_signed: u1 = rdt_is_signed(b_tag)
  let a_top: u1 = a_signed & a[31]
  let b_top: u1 = b_signed & b[31]
  let a_wide = @signed(@concat(a_top, a))
  let b_wide = @signed(@concat(b_top, b))

  match cmp_op
    .CMP_EQ =>
      cmp_result = a_wide == b_wide
    .CMP_NE =>
      cmp_result = a_wide != b_wide
    .CMP_LT =>
      cmp_result = a_wide < b_wide
    .CMP_GT =>
      cmp_result = a_wide > b_wide
    .CMP_LE =>
      cmp_result = a_wide <= b_wide
    .CMP_GE =>
      cmp_result = a_wide >= b_wide
    _ => @unreachable
