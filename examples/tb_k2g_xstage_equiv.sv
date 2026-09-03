// The DDL execute stage against K2G's own components.
//
// The reference is not logic written for this test: `k2g_xstage_ref.sv`
// instantiates the unmodified `k2g_regfile`, `k2g_alu` and `k2g_shift` and
// wires them the way `k2g_core.sv` does. The DDL side has the register arrays
// INSIDE the process, reached by subscript, which is the whole claim -- that
// the file keeps its asynchronous read and its RAM primitives while living in
// the process that reads it.
//
// Three checks:
//
//   1. The writeback packet sequence, item by item. The DDL registers its
//      output in the channel slot and the reference produces it
//      combinationally, so this compares sequences rather than cycles -- the
//      same arrangement tb_k2g_decode_equiv.sv uses.
//   2. An independent model of the four arrays and the forwarding mux. Both
//      implementations agreeing on a wrong answer still fails. It is checked
//      through UOP_COPY, which writes the forwarded operand straight into the
//      packet, so the model sees the read-and-forward result without having to
//      reimplement the ALU that k2g_alu.ddl already proves.
//   3. READ-BEFORE-WRITE and forwarding, driven deliberately: back-to-back
//      dependent micro-ops, with an anti-vacuous guard on how often the
//      forwarding path actually fired.
`timescale 1ns/1ps
`include "k2g_types.svh"

module tb_k2g_xstage_equiv;
  import k2g_pkg::*;
  import k2g_types::*;

  logic clk = 1'b0;
  logic rst_n;
  always #5 clk = ~clk;

  uop_t uop;
  logic offer;      // the producer has a micro-op available
  logic wb_ready;   // the consumer will take a writeback packet

  logic [83:0] ref_packet;

  // ---- the testbench is the salt producer on `uops` ------------------------
  logic [1:0]   tb_wsalt;
  uop_t         tb_e [0:1];
  logic [1:0]   ddl_uops_rsalt;
  wire  tb_widx = tb_wsalt[0] ^ tb_wsalt[1];
  wire  tb_full = (tb_wsalt == ~ddl_uops_rsalt);
  wire  push    = offer && !tb_full;

  // ---- and the salt consumer on `wb` ---------------------------------------
  logic [1:0]   tb_rsalt;
  logic [1:0]   ddl_wb_wsalt;
  logic [167:0] ddl_wb_pair;
  wire  tb_ridx   = tb_rsalt[0] ^ tb_rsalt[1];
  wire  ddl_empty = (ddl_wb_wsalt == tb_rsalt);
  wire [83:0] ddl_wb = tb_ridx ? ddl_wb_pair[167:84] : ddl_wb_pair[83:0];
  wire  pop = wb_ready && !ddl_empty;

  // THE BRIDGE. Not `push`: an item entering the pipe is not the same event as
  // the DDL occupying X with it, and under salt they can be cycles apart. The
  // reference is driven from the DDL's own consume decision, and reads the
  // entry the DDL is reading.
  wire  xfer = u_ddl.uops_take;
  uop_t taken;
  assign taken = u_ddl.uops_ridx ? tb_e[1] : tb_e[0];

  // Anti-footgun: the entry the reference is handed must be the one the DDL is
  // reading. If these ever differ the two are decoding different micro-ops and
  // every downstream mismatch is noise.
  always @(posedge clk)
    if (rst_n && xfer)
      assert (taken === u_ddl.uops_item)
        else $error("bridge: ref sees a different micro-op than the DDL took");

  k2g_xstage_ref u_ref (
      .clk(clk), .rst_n(rst_n),
      .uop(taken), .uop_valid(xfer), .packet(ref_packet)
  );

  k2g_xstage u_ddl (
      .clk(clk), .rst_n(rst_n),
      .uops_wsalt(tb_wsalt), .uops_rsalt(ddl_uops_rsalt),
      .uops_data({tb_e[1], tb_e[0]}),
      .wb_wsalt(ddl_wb_wsalt), .wb_rsalt(tb_rsalt), .wb_data(ddl_wb_pair)
  );

  always_ff @(posedge clk) begin
    if (!rst_n) begin
      tb_wsalt <= 2'b00;
      tb_rsalt <= 2'b00;
    end else begin
      if (push) begin
        tb_e[tb_widx] <= uop;
        tb_wsalt[tb_widx] <= ~tb_wsalt[tb_widx];
      end
      if (pop) tb_rsalt[tb_ridx] <= ~tb_rsalt[tb_ridx];
    end
  end

  int errors = 0, checks = 0, cycles = 0, compared = 0;
  int stalls = 0, forwarded = 0, copies = 0;

  logic [83:0] ref_q[$];
  logic [83:0] ddl_q[$];
  logic [83:0] mdl_q[$];
  bit          mdl_check[$];

  // The independent model: the four arrays, and the W packet that writes them.
  logic [31:0] m_value    [0:31];
  logic [2:0]  m_tag      [0:31];
  logic        m_overflow [0:31];
  logic        m_flag     [0:31];

  logic        mw_we_value, mw_we_tag, mw_we_overflow, mw_we_flag;
  logic [4:0]  mw_addr;
  logic [31:0] mw_value;
  logic [2:0]  mw_tag;
  logic        mw_overflow, mw_flag;

  int predicated = 0, faults = 0, suppressed = 0, narrowed = 0;

  logic        held_valid = 1'b0;
  logic [83:0] held_wb;
  bit          armed = 1'b0;

  task automatic drain();
    while (ref_q.size() > 0 && ddl_q.size() > 0) begin
      automatic logic [83:0] a = ref_q.pop_front();
      automatic logic [83:0] b = ddl_q.pop_front();
      automatic logic [83:0] m = mdl_q.pop_front();
      automatic bit          use_model = mdl_check.pop_front();
      checks++;
      compared++;
      if (a !== b) begin
        errors++;
        if (errors <= 5)
          $display("MISMATCH packet #%0d @%0d:\n  ref=%021x\n  ddl=%021x\n  mdl=%021x use_model=%0d", compared, cycles, a, b, m, use_model);
      end
      // The model only claims to know the packets whose value comes straight
      // from a forwarded register read.
      if (use_model) begin
        checks++;
        if (m !== b) begin
          errors++;
          if (errors <= 5)
            $display("MODEL packet #%0d @%0d:\n  model=%021x\n  ddl  =%021x", compared, cycles, m, b);
        end
      end
    end
  endtask

  task automatic step(logic v, logic r);
    offer = v; wb_ready = r;
    #1;

    // Rule 2: an offer may not be withdrawn or altered before it is taken.
    checks++;
    if (held_valid && !pop) begin
      if (ddl_empty) begin
        errors++;
        if (errors <= 5) $display("RULE2 withdrawn @%0d", cycles);
      end else if (ddl_wb !== held_wb) begin
        errors++;
        if (errors <= 5) $display("RULE2 data changed @%0d", cycles);
      end
    end

    if (offer && tb_full) stalls++;

    // Everything in here is about the micro-op the DDL CONSUMED, which is
    // `taken` -- the entry its own read index names -- and not whatever the
    // testbench happens to be offering. Under valid/ready those were the
    // same value; with two entries between the sides they are not.
    if (xfer) begin
      // ---- the model, as of the start of the cycle ----------------------
      automatic logic [4:0]  ra = taken.dst;
      automatic logic [4:0]  rb = taken.src;
      automatic logic mwriting = mw_we_value | mw_we_tag | mw_we_overflow | mw_we_flag;
      automatic logic ma = mwriting && (mw_addr == ra);
      automatic logic mb = mwriting && (mw_addr == rb);
      automatic logic [31:0] xb_value = (mb && mw_we_value)    ? mw_value    : m_value[rb];
      automatic logic [2:0]  xb_tag   = (mb && mw_we_tag)      ? mw_tag      : m_tag[rb];
      automatic logic        xb_ovf   = (mb && mw_we_overflow) ? mw_overflow : m_overflow[rb];
      automatic logic        xb_flg   = (mb && mw_we_flag)     ? mw_flag     : m_flag[rb];
      // The model knows about forwarding, not about predication or the
      // reserved tag, so it only claims a COPY that neither of those touches.
      automatic bit is_copy = (taken.kind == UOP_COPY)
                              && (taken.cond == CCK_NONE)
                              && (xb_tag != RDT_F32);

      if (taken.cond != CCK_NONE) predicated++;
      // The DDL implements `rdt_normalize` itself and the reference calls the
      // one in k2g_types.svh, so the comparison checks it -- but only on the
      // packets where it is not the identity. Count those.
      if ((taken.kind == UOP_PUT_IMM)
          && (rdt_normalize(taken.imm, taken.datakind) !== taken.imm)) narrowed++;
      if (ref_packet[37]) faults++;
      // A COPY commits all four fields or none, so a COPY that wrote nothing
      // is predication doing its job rather than an opcode that writes little.
      if ((taken.cond != CCK_NONE) && (taken.kind == UOP_COPY)
          && (ref_packet[83:80] == 4'd0)) suppressed++;

      if (ma || mb) forwarded++;
      if (is_copy) begin
        copies++;
        mdl_q.push_back({1'b1, 1'b1, 1'b1, 1'b1, ra, xb_value, xb_tag, xb_ovf, xb_flg,
                         1'b0, FAULT_NONE, 32'd0});
      end else begin
        mdl_q.push_back(84'd0);
      end
      mdl_check.push_back(is_copy);

      ref_q.push_back(ref_packet);

      // ---- the model's arrays take W's write, at the edge ---------------
      if (mw_we_value)    m_value[mw_addr]    = mw_value;
      if (mw_we_tag)      m_tag[mw_addr]      = mw_tag;
      if (mw_we_overflow) m_overflow[mw_addr] = mw_overflow;
      if (mw_we_flag)     m_flag[mw_addr]     = mw_flag;

      // ...and the packet the reference just produced becomes the model's W.
      mw_we_value    = ref_packet[83];
      mw_we_tag      = ref_packet[82];
      mw_we_overflow = ref_packet[81];
      mw_we_flag     = ref_packet[80];
      mw_addr        = ref_packet[79:75];
      mw_value       = ref_packet[74:43];
      mw_tag         = ref_packet[42:40];
      mw_overflow    = ref_packet[39];
      mw_flag        = ref_packet[38];
    end

    if (pop) ddl_q.push_back(ddl_wb);
    if (armed) drain();

    held_valid = !ddl_empty && !pop;
    held_wb    = ddl_wb;
    @(posedge clk);
    cycles++;
  endtask

  // Rule 3, BOTH WAYS: driving one side's salt to an arbitrary value within a
  // cycle must not move anything the other side publishes. The second half has
  // no counterpart under valid/ready.
  task automatic check_rule3();
    logic [1:0]   w0, r0, save_r, save_w;
    logic [167:0] d0;
    save_r = tb_rsalt; save_w = tb_wsalt;

    tb_rsalt = 2'b00; #1; w0 = ddl_wb_wsalt; d0 = ddl_wb_pair;
    tb_rsalt = 2'b11; #1;
    checks++;
    if (ddl_wb_wsalt !== w0 || ddl_wb_pair !== d0) begin
      errors++;
      if (errors <= 5) $display("RULE3 producer moved with rsalt @%0d", cycles);
    end
    tb_rsalt = save_r;

    tb_wsalt = 2'b00; #1; r0 = ddl_uops_rsalt;
    tb_wsalt = 2'b11; #1;
    checks++;
    if (ddl_uops_rsalt !== r0) begin
      errors++;
      if (errors <= 5) $display("RULE3 consumer moved with wsalt @%0d", cycles);
    end
    // Restored before the edge: arbitrary salts make full/empty meaningless.
    tb_wsalt = save_w;
    #1;
    wb_ready = 1'b0;
  endtask

  function automatic uop_kind_e a_kind();
    case ($urandom_range(12))
      0: return UOP_PUT_IMM;
      1: return UOP_SET_TAG;
      2: return UOP_COPY;
      3: return UOP_ARITH;
      4: return UOP_LOGIC;
      5: return UOP_UNARY;
      6: return UOP_CMP;
      7: return UOP_SHIFT;
      8: return UOP_BEXT;
      9: return UOP_BINS;
      10: return UOP_LOAD;
      11: return UOP_STORE;
      default: return UOP_FAULT;
    endcase
  endfunction

  // A tag the design is allowed to produce. RDT_UNCLAIMED_7 is reserved and
  // the DDL asserts it is never written.
  function automatic rdt_e a_tag();
    return rdt_e'($urandom_range(6));
  endfunction

  task automatic put(uop_kind_e k, logic [4:0] d, logic [4:0] s);
    uop           = '0;
    uop.kind      = k;
    uop.dst       = d;
    uop.src       = s;
    uop.cond_reg  = $urandom_range(31);
    uop.cond        = CCK_NONE;
    uop.cond_invert = 1'b0;
    uop.jump_kind   = jump_e'($urandom_range(2));
    // Only read when `kind` is UOP_FAULT, and then it is the whole answer.
    uop.fault       = fault_e'($urandom_range(11));
    uop.imm       = $urandom;
    uop.use_imm   = $urandom_range(1);
    uop.arith_op  = arith_e'($urandom_range(3));
    uop.logic_op  = logic_e'($urandom_range(2));
    uop.shift_op  = shift_e'($urandom_range(2));
    uop.cmp_op    = cmp_e'($urandom_range(5));
    uop.unary_op  = unary_e'($urandom_range(1));
    // The FLAG prefix: a logic or unary op on flag bits rather than values.
    uop.on_flags  = $urandom_range(1);
    uop.datakind  = a_tag();
    uop.bm_start  = $urandom_range(31);
    uop.bm_span   = $urandom_range(31);
  endtask

  // Predication on top of whatever `put` just built (spec 7). Separate from
  // `put` so the phases that exist to exercise forwarding still commit.
  task automatic predicate();
    uop.cond        = cond_kind_e'($urandom_range(4) + 1);
    uop.cond_invert = $urandom_range(1);
  endtask

  int i;
  initial begin
    rst_n = 1'b0;
    put(UOP_NOP, 5'd0, 5'd0);
    offer = 1'b0; wb_ready = 1'b0;
    for (i = 0; i < 32; i = i + 1) begin
      m_value[i]    = 32'd0;
      m_tag[i]      = 3'b010;  // RDT_U32, the emulator's reset tag
      m_overflow[i] = 1'b0;
      m_flag[i]     = 1'b0;
    end
    mw_we_value = 1'b0; mw_we_tag = 1'b0;
    mw_we_overflow = 1'b0; mw_we_flag = 1'b0;
    mw_addr = 5'd0; mw_value = 32'd0; mw_tag = 3'd0;
    mw_overflow = 1'b0; mw_flag = 1'b0;

    repeat (3) @(posedge clk);
    rst_n = 1'b1;
    @(posedge clk);
    armed = 1'b1;

    // 1. Fill every register with something, then read it all back through
    //    UOP_COPY so the model has an opinion about every entry.
    for (i = 0; i < 32; i = i + 1) begin
      put(UOP_PUT_IMM, i[4:0], 5'd0);
      step(1'b1, 1'b1);
    end
    for (i = 0; i < 32; i = i + 1) begin
      put(UOP_COPY, 5'd0, i[4:0]);
      step(1'b1, 1'b1);
    end

    // 2. Back-to-back dependents: every micro-op reads what the one before it
    //    wrote, which is the forwarding path and nothing else.
    for (i = 0; i < 4000; i = i + 1) begin
      automatic logic [4:0] r = $urandom_range(31);
      put(a_kind(), r, r);
      step(1'b1, 1'b1);
      put(UOP_COPY, $urandom_range(31), r);
      step(1'b1, 1'b1);
    end

    // 3. Random traffic over a small register set, so read/write collisions
    //    and forwarding happen constantly, with both sides back-pressured.
    for (i = 0; i < 40000; i = i + 1) begin
      put(a_kind(), $urandom_range(7), $urandom_range(7));
      step($urandom_range(2) != 0, $urandom_range(2) != 0);
    end

    // 4. Sink jammed shut: the slot fills and back-pressure reaches the
    //    producer, then releases.
    for (i = 0; i < 200; i = i + 1) begin
      put(a_kind(), $urandom_range(7), $urandom_range(7));
      step(1'b1, 1'b0);
    end
    for (i = 0; i < 2000; i = i + 1) begin
      put(a_kind(), $urandom_range(7), $urandom_range(7));
      step(1'b1, $urandom_range(3) == 0);
    end

    for (i = 0; i < 200; i = i + 1) begin
      put(a_kind(), $urandom_range(7), $urandom_range(7));
      step(1'b1, 1'b0);
      check_rule3();
    end

    // 5. Predication and faults. Separate from the phases above because it is
    //    a different question: those ask whether a committed result is right,
    //    this asks whether the thing committed at all.
    for (i = 0; i < 6000; i = i + 1) begin
      put(a_kind(), $urandom_range(7), $urandom_range(7));
      if ($urandom_range(2) != 0) predicate();
      step(1'b1, $urandom_range(3) != 0);
    end
    // Predicated copies specifically. A COPY commits all four fields or none,
    // so it is the cleanest place to watch a predicate suppress a commit --
    // and at one kind in thirteen the loop above does not produce enough of
    // them to prove the suppression ever happened.
    for (i = 0; i < 3000; i = i + 1) begin
      put(UOP_COPY, $urandom_range(7), $urandom_range(7));
      predicate();
      step(1'b1, 1'b1);
    end

    // Directed: a misaligned and an out-of-range access on every width, which
    // random immediates hit but not reliably per width.
    for (i = 0; i < 200; i = i + 1) begin
      put(UOP_LOAD, $urandom_range(7), $urandom_range(7));
      uop.datakind = a_tag();
      uop.imm      = (i[0]) ? 32'd1 : 32'h00FF_FFFF;
      step(1'b1, 1'b1);
      put(UOP_STORE, $urandom_range(7), $urandom_range(7));
      uop.imm = (i[0]) ? 32'd2 : 32'h7FFF_FFFF;
      step(1'b1, 1'b1);
    end

    // Drain, so a packet one side produced and the other swallowed cannot
    // hide in a queue.
    for (i = 0; i < 16; i = i + 1)
      step(1'b0, 1'b1);

    if (ref_q.size() != 0 || ddl_q.size() != 0) begin
      errors++;
      $display("LEFTOVER ref=%0d ddl=%0d", ref_q.size(), ddl_q.size());
    end
    // A run where forwarding never fired, or nothing stalled, proves nothing
    // about the two things this module is for.
    if (compared < 20000 || forwarded < 5000 || stalls < 500 || copies < 5000
        || predicated < 2000 || faults < 500 || suppressed < 500
        || narrowed < 500) begin
      $display({"TB_FAIL  vacuous: %0d packets, %0d forwarded, %0d stalls, ",
                "%0d copies, %0d predicated, %0d faults, %0d suppressed, %0d narrowed"},
               compared, forwarded, stalls, copies, predicated, faults, suppressed,
               narrowed);
      $finish;
    end

    if (errors == 0)
      $display({"TB_PASS  %0d cycles, %0d comparisons, %0d packets, %0d forwarded, ",
                "%0d stalls, %0d predicated, %0d faults, %0d narrowed, 0 mismatches"},
               cycles, checks, compared, forwarded, stalls, predicated, faults, narrowed);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
