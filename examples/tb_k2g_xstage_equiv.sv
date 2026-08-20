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
  logic        ddl_uops_ready, ddl_wb_valid;
  logic [83:0] ddl_wb;

  // The reference occupies X exactly when the DDL accepts a micro-op.
  wire xfer = offer & ddl_uops_ready;

  k2g_xstage_ref u_ref (
      .clk(clk), .rst_n(rst_n),
      .uop(uop), .uop_valid(xfer), .packet(ref_packet)
  );

  k2g_xstage u_ddl (
      .clk(clk), .rst_n(rst_n),
      .uops_valid(offer), .uops_ready(ddl_uops_ready), .uops_data(uop),
      .wb_valid(ddl_wb_valid), .wb_ready(wb_ready), .wb_data(ddl_wb)
  );

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

  int predicated = 0, faults = 0, suppressed = 0;

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
          $display("MISMATCH packet #%0d @%0d:\n  ref=%021x\n  ddl=%021x", compared, cycles, a, b);
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
    if (held_valid && !(ddl_wb_valid && wb_ready)) begin
      if (!ddl_wb_valid) begin
        errors++;
        if (errors <= 5) $display("RULE2 withdrawn @%0d", cycles);
      end else if (ddl_wb !== held_wb) begin
        errors++;
        if (errors <= 5) $display("RULE2 data changed @%0d", cycles);
      end
    end

    if (offer && !ddl_uops_ready) stalls++;

    if (xfer) begin
      // ---- the model, as of the start of the cycle ----------------------
      automatic logic [4:0]  ra = uop.dst;
      automatic logic [4:0]  rb = uop.src;
      automatic logic mwriting = mw_we_value | mw_we_tag | mw_we_overflow | mw_we_flag;
      automatic logic ma = mwriting && (mw_addr == ra);
      automatic logic mb = mwriting && (mw_addr == rb);
      automatic logic [31:0] xb_value = (mb && mw_we_value)    ? mw_value    : m_value[rb];
      automatic logic [2:0]  xb_tag   = (mb && mw_we_tag)      ? mw_tag      : m_tag[rb];
      automatic logic        xb_ovf   = (mb && mw_we_overflow) ? mw_overflow : m_overflow[rb];
      automatic logic        xb_flg   = (mb && mw_we_flag)     ? mw_flag     : m_flag[rb];
      // The model knows about forwarding, not about predication or the
      // reserved tag, so it only claims a COPY that neither of those touches.
      automatic bit is_copy = (uop.kind == UOP_COPY)
                              && (uop.cond == CCK_NONE)
                              && (xb_tag != RDT_F32);

      if (uop.cond != CCK_NONE) predicated++;
      if (ref_packet[37]) faults++;
      // A COPY commits all four fields or none, so a COPY that wrote nothing
      // is predication doing its job rather than an opcode that writes little.
      if ((uop.cond != CCK_NONE) && (uop.kind == UOP_COPY)
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

    if (ddl_wb_valid && wb_ready) ddl_q.push_back(ddl_wb);
    if (armed) drain();

    held_valid = ddl_wb_valid && !wb_ready;
    held_wb    = ddl_wb;
    @(posedge clk);
    cycles++;
  endtask

  // Rule 3: toggling `ready` within a cycle must not move `valid` or `data`.
  task automatic check_rule3();
    logic        v0;
    logic [83:0] d0;
    wb_ready = 1'b0; #1;
    v0 = ddl_wb_valid; d0 = ddl_wb;
    wb_ready = 1'b1; #1;
    checks++;
    if (ddl_wb_valid !== v0 || ddl_wb !== d0) begin
      errors++;
      if (errors <= 5) $display("RULE3 valid/data moved with ready @%0d", cycles);
    end
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
        || predicated < 2000 || faults < 500 || suppressed < 500) begin
      $display({"TB_FAIL  vacuous: %0d packets, %0d forwarded, %0d stalls, ",
                "%0d copies, %0d predicated, %0d faults, %0d suppressed"},
               compared, forwarded, stalls, copies, predicated, faults, suppressed);
      $finish;
    end

    if (errors == 0)
      $display({"TB_PASS  %0d cycles, %0d comparisons, %0d packets, %0d forwarded, ",
                "%0d stalls, %0d predicated, %0d faults, 0 mismatches"},
               cycles, checks, compared, forwarded, stalls, predicated, faults);
    else
      $display("TB_FAIL  %0d cycles, %0d mismatches", cycles, errors);
    $finish;
  end
endmodule
