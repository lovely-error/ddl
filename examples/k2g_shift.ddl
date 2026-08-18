-- K2G shifter and bit-field unit, in DDL.
--
-- A direct port of rtl/k2g_shift.sv. Purely combinational, so it lowers as a
-- `fun`: the three results leave through `out` parameters, which is how a
-- function expresses a module with more than one output.
--
-- shift_op is `shift_e`, shared with k2g_types.ddl.
-- until then the encoding is written out, and an SV `shift_e` signal connects
-- to a `[1:0]` port without a cast.

fun k2g_shift (
    value: i32,          -- shift operand, and the BINS destination
    src: i32,            -- BEXT source, and the BINS insert source
    amount: i5,          -- shift amount, already masked to 5 bits
    shift_op: shift_e,
    bm_start: i5,
    bm_span: i5,

    shift_result: out i32,
    bext_result: out i32,
    bins_result: out i32)

  -- ---- shifts ------------------------------------------------------------
  --
  -- SHRA needs no special case for sub-word tags: a register's bits always
  -- match its tag (spec 1.1.1), so a signed value is already sign-filled to 32
  -- bits and an unsigned one is non-negative.
  --
  -- `>>` is arithmetic when its left operand is signed and logical when it is
  -- unsigned, so @signed is what picks SHRA -- the same job `value_s` does in
  -- the SystemVerilog.
  if shift_op == SHIFT_LL then
    shift_result = value << amount
  else
    if shift_op == SHIFT_LR then
      shift_result = value >> amount
    else
      shift_result = @unsigned(@signed(value) >> amount)

  -- ---- bit-field mask ----------------------------------------------------
  -- A span of 32 would overflow a 5-bit field, so span is 0..31 by
  -- construction; span 0 yields an all-zero mask, which makes BINS a no-op --
  -- the defined behaviour (spec 5.8). BEXT with span 0 is caught in decode.
  let span_mask: i32 = if bm_span == 5'd0
      then 32'd0
      else (32'd1 << bm_span) - 32'd1

  -- Extract reads the SOURCE register (arg2), not the destination. Extracting
  -- from `value` instead was an actual bug, caught by randomized lockstep
  -- cosimulation -- the unit test had encoded the same misunderstanding, so it
  -- passed against wrong RTL.
  bext_result = (src >> bm_start) & span_mask

  let placed_mask: i32 = span_mask << bm_start
  let placed_src: i32 = (src & span_mask) << bm_start

  bins_result = (value & ~placed_mask) | placed_src
