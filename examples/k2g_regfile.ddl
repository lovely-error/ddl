-- The K2G register file: 32 registers, three read ports, one write port.
--
-- This is the module `#[impl(...)]` exists for. Four arrays, all written
-- through ONE port each, all read asynchronously -- which is exactly the shape
-- GowinSynthesis infers SSRAM from, and the reason the hand-written version
-- (rtl/k2g_regfile.sv:18-24) is so emphatic that the value array has exactly
-- one write port: an earlier revision added a second for the widening
-- multiply's high half, no RAM was inferred at all, and 32 primitives plus
-- ~100 LUTs became 1120 flip-flops and ~3700 LUTs of read muxing.
--
-- DDL cannot make that mistake. A memory has one write port by construction,
-- and two writes on different branches mux onto it instead of asking for a
-- second one -- so the `if` chains below cost one enable, not one port each.
--
-- READ-BEFORE-WRITE, and not by accident. Reads see the array as of the start
-- of the cycle and the write lands at the edge, so a read and a write of the
-- same register in one cycle give the OLD value. k2g_regfile.sv:118 records
-- why a write-first bypass is wrong here rather than merely different: execute
-- is a single cycle, so the write being presented is this instruction's own
-- result, computed from these very reads -- bypassing closes a combinational
-- loop through the register file, and it hung simulation.
--
-- The reset loops are deliberate too, and they are not free: about 85 LUTs and
-- a few extra RAM primitives. Without them the arrays power up undefined, the
-- emulator's zeroed registers disagree, and cosimulation cannot compare a
-- register until something writes it.

process k2g_regfile (
    -- Two read ports for the operands. Port A doubles as the predicate read
    -- during a prefix cycle, which costs no address mux because the predicate
    -- register index sits at the same field position as arg1.
    ra_addr: i5,
    rb_addr: i5,
    -- Third read port, for the register named by an XCP/XCN prefix. The VALUE
    -- is needed because VALUE_ZERO / VALUE_NEGATIVE / VALUE_POSITIVE test the
    -- register's contents rather than a flag.
    rc_addr: i5,

    -- One write port, with an independent enable per field. Per-field enables
    -- are why the tag and flag arrays are separate from the value array: a
    -- comparison writes only `flag`, an arithmetic op writes only `overflow`,
    -- and a copy writes everything.
    we_value: i1,
    we_tag: i1,
    we_overflow: i1,
    we_flag: i1,
    w_addr: i5,
    w_value: i32,
    w_tag: rdt_e,
    w_overflow: i1,
    w_flag: i1,

    ra_value: out i32,
    rb_value: out i32,
    rc_value: out i32,
    ra_tag: out rdt_e,
    rb_tag: out rdt_e,
    rc_tag: out rdt_e,
    ra_overflow: out i1,
    rb_overflow: out i1,
    rc_overflow: out i1,
    ra_flag: out i1,
    rb_flag: out i1,
    rc_flag: out i1)

  var values: #[impl(lutram)] [i32; 32] = @zeroed()
  -- The reset tag matches the emulator's, so a register that has never been
  -- written still compares equal.
  var tags: #[impl(lutram)] [rdt_e; 32] = RDT_U32
  var overflow: #[impl(lutram)] [i1; 32] = @zeroed()
  var flagbit: #[impl(lutram)] [i1; 32] = @zeroed()

  ra_value = values[ra_addr]
  rb_value = values[rb_addr]
  rc_value = values[rc_addr]

  ra_tag = tags[ra_addr]
  rb_tag = tags[rb_addr]
  -- The predicate port's tag is read for one reason: F32 is reserved, and a
  -- predicate register carrying it faults like any other read.
  rc_tag = tags[rc_addr]

  ra_overflow = overflow[ra_addr]
  rb_overflow = overflow[rb_addr]
  rc_overflow = overflow[rc_addr]

  ra_flag = flagbit[ra_addr]
  rb_flag = flagbit[rb_addr]
  rc_flag = flagbit[rc_addr]

  if we_value then
    values[w_addr] = w_value

  if we_tag then
    -- RDT_UNCLAIMED_7 is a reserved encoding: nothing produces it, and a tag
    -- carrying it would make store width and comparison signedness undefined.
    -- Written INSIDE the `if`, so it says nothing about cycles that do not
    -- write a tag -- and the testbench drives that very encoding when the
    -- enable is low, which is what makes the guard load-bearing rather than
    -- decorative.
    @assert(w_tag != RDT_UNCLAIMED_7, "the reserved tag encoding was written")
    tags[w_addr] = w_tag

  if we_overflow then
    overflow[w_addr] = w_overflow

  if we_flag then
    flagbit[w_addr] = w_flag
