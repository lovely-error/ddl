-- The K2G register file, behind a channel.
--
-- The hand-written module (rtl/k2g_regfile.sv) has 24 flat data ports and no
-- protocol at all: three addresses in, values out combinationally, four write
-- enables. That is not a thing DDL can express, and the reason is the rule
-- rather than a gap -- a process takes data through pipes, so a register file
-- as a MODULE is a category error here. The arrays belong to whichever process
-- reads them, reached by subscript, and what crosses the boundary is the
-- request and the answer.
--
-- So this is the same four arrays with the same one-write-port shape, wrapped
-- in the channel a caller would actually use: one request in, one reply out.
-- The arrays and their inference shape are unchanged, which is the point --
-- `#[impl(lutram)]` still gets 96 RAM primitives out of GowinSynthesis, and
-- the port list is three signals instead of twenty-four.
--
-- THE VALUE ARRAY HAS EXACTLY ONE WRITE PORT, and in DDL it cannot have two.
-- k2g_regfile.sv:18-24 records what a second one cost when it was tried: no
-- RAM was inferred at all, and 32 primitives plus ~100 LUTs became 1120
-- flip-flops and ~3700 LUTs of read muxing. Here the four writes below are
-- four `if`s onto one port, and the compiler turns them into write enables.
--
-- READ-BEFORE-WRITE, deliberately. Reads see the arrays as of the start of the
-- cycle and writes land at the edge, so a read and a write of the same
-- register in one cycle give the OLD value. k2g_regfile.sv:118 records why
-- that is correctness rather than taste: execute is a single cycle, so the
-- write being presented is this instruction's own result, computed from these
-- very reads -- bypassing closes a combinational loop through the file, and it
-- hung simulation rather than producing a wrong answer.

struct rf_req_t
  -- Two read ports for the operands, plus a third for the register named by
  -- an XCP/XCN prefix: VALUE_ZERO / VALUE_NEGATIVE / VALUE_POSITIVE test the
  -- register's contents rather than a flag.
  ra_addr: i5
  rb_addr: i5
  rc_addr: i5

  -- One write port, with an independent enable per field. Per-field enables
  -- are why the tag and flag arrays are separate from the value array: a
  -- comparison writes only `flag`, an arithmetic op writes only `overflow`,
  -- and a copy writes everything.
  we_value: i1
  we_tag: i1
  we_overflow: i1
  we_flag: i1
  w_addr: i5
  w_value: i32
  w_tag: rdt_e
  w_overflow: i1
  w_flag: i1

struct rf_rsp_t
  ra_value: i32
  rb_value: i32
  rc_value: i32
  ra_tag: rdt_e
  rb_tag: rdt_e
  rc_tag: rdt_e
  ra_overflow: i1
  rb_overflow: i1
  rc_overflow: i1
  ra_flag: i1
  rb_flag: i1
  rc_flag: i1

process k2g_regfile (req: buffer in rf_req_t, rsp: buffer out rf_rsp_t)
  var values: #[impl(lutram)] [i32; 32] = @zeroed()
  -- The reset tag matches the emulator's, so a register that has never been
  -- written still compares equal. The reset loops are not free -- about 85
  -- LUTs and a few extra RAM primitives -- but without them the arrays power
  -- up undefined and cosimulation cannot compare a register until something
  -- writes it.
  var tags: #[impl(lutram)] [rdt_e; 32] = RDT_U32
  var overflow: #[impl(lutram)] [i1; 32] = @zeroed()
  var flagbit: #[impl(lutram)] [i1; 32] = @zeroed()

  -- The process is a program: it runs once and stops. `loop` is what makes
  -- it repeat, once per cycle, for as long as the design runs.
  loop

    let (r, got) = @try_rcv(req)

    var out: rf_rsp_t = @zeroed()
    out.ra_value = values[r.ra_addr]
    out.rb_value = values[r.rb_addr]
    out.rc_value = values[r.rc_addr]

    out.ra_tag = tags[r.ra_addr]
    out.rb_tag = tags[r.rb_addr]
    -- The predicate port's tag is read for one reason: F32 is reserved, and a
    -- predicate register carrying it faults like any other read.
    out.rc_tag = tags[r.rc_addr]

    out.ra_overflow = overflow[r.ra_addr]
    out.rb_overflow = overflow[r.rb_addr]
    out.rc_overflow = overflow[r.rc_addr]

    out.ra_flag = flagbit[r.ra_addr]
    out.rb_flag = flagbit[r.rb_addr]
    out.rc_flag = flagbit[r.rc_addr]

    @try_send(rsp, out)

    if r.we_value then
      values[r.w_addr] = r.w_value

    if r.we_tag then
      -- RDT_UNCLAIMED_7 is a reserved encoding: nothing produces it, and a tag
      -- carrying it would leave store width and comparison signedness
      -- undefined. Written inside the `if`, so it says nothing about cycles
      -- that do not write a tag.
      @assert(r.w_tag != RDT_UNCLAIMED_7, "the reserved tag encoding was written")
      tags[r.w_addr] = r.w_tag

    if r.we_overflow then
      overflow[r.w_addr] = r.w_overflow

    if r.we_flag then
      flagbit[r.w_addr] = r.w_flag
