// The execute stage as k2g_core.sv wires it, for comparison against the DDL.
//
// This is deliberately NOT logic written for the test. It instantiates the
// unmodified `k2g_regfile`, `k2g_alu` and `k2g_shift` out of the K2G tree and
// connects them the way `k2g_core.sv:403-600` connects them -- the same read
// addresses, the same per-field forwarding mux, the same operand selection,
// the same address adder. So the comparison is "the DDL process with the
// arrays inlined" against "K2G's real components wired the way K2G wires
// them", which is the strongest reference available and the same trick
// `rtl/tb/tb_datapath.sv` uses for the ALU/shifter/regfile trio.
//
// The writeback packet leaves combinationally, as it does in K2G, where X
// computes it and the W registers latch it (k2g_core.sv:1111). The DDL side
// registers it in its output channel slot instead, which is why the testbench
// compares sequences rather than cycles.
`timescale 1ns/1ps
`include "k2g_types.svh"

module k2g_xstage_ref
  import k2g_pkg::*;
  import k2g_types::*;
#(
    // Where ADDR_OUT_OF_RANGE begins (spec 8). The same number the DDL folds
    // in as `mem_bytes`, and the same one the emulator carries.
    parameter int MEM_BYTES = 8 * 1024 * 1024
) (
    input  logic       clk,
    input  logic       rst_n,

    input  uop_t       uop,
    input  logic       uop_valid,   // an instruction occupies X this cycle

    // The packet X computed, packed the way the DDL packs `wb_t`: first field
    // in the high bits.
    output logic [83:0] packet
);

  // ---- register file -----------------------------------------------------
  logic [4:0]  ra_addr, rb_addr, rc_addr;
  logic [31:0] ra_value, rb_value, rc_value;
  logic [2:0]  ra_tag, rb_tag, rc_tag;
  logic        ra_overflow, rb_overflow, rc_overflow;
  logic        ra_flag, rb_flag, rc_flag;

  // The W stage: what X computed last cycle. It drives the array write port
  // and the forwarding mux, exactly as in k2g_core.
  logic        w_we_value, w_we_tag, w_we_overflow, w_we_flag;
  logic [4:0]  w_addr;
  logic [31:0] w_value;
  logic [2:0]  w_tag;
  logic        w_overflow, w_flag;

  k2g_regfile u_rf (
      .clk(clk), .rst_n(rst_n),
      .ra_addr(ra_addr), .rb_addr(rb_addr), .rc_addr(rc_addr),
      .ra_value(ra_value), .rb_value(rb_value), .rc_value(rc_value),
      .ra_tag(ra_tag), .rb_tag(rb_tag), .rc_tag(rc_tag),
      .ra_overflow(ra_overflow), .rb_overflow(rb_overflow), .rc_overflow(rc_overflow),
      .ra_flag(ra_flag), .rb_flag(rb_flag), .rc_flag(rc_flag),
      .we_value(w_we_value), .we_tag(w_we_tag),
      .we_overflow(w_we_overflow), .we_flag(w_we_flag),
      .w_addr(w_addr), .w_value(w_value), .w_tag(rdt_e'(w_tag)),
      .w_overflow(w_overflow), .w_flag(w_flag)
  );

  // Read addresses stay raw uop fields (k2g_core.sv:422-429): forwarding muxes
  // read DATA, never addresses.
  assign ra_addr = uop.dst;
  assign rb_addr = uop.src;
  assign rc_addr = uop.cond_reg;

  // ---- forwarding W -> X -------------------------------------------------
  wire logic writing = w_we_value | w_we_tag | w_we_overflow | w_we_flag;
  wire logic ma = writing && (w_addr == ra_addr);
  wire logic mb = writing && (w_addr == rb_addr);
  wire logic mc = writing && (w_addr == rc_addr);

  wire logic [31:0] xa_value = (ma && w_we_value)    ? w_value    : ra_value;
  wire logic [31:0] xb_value = (mb && w_we_value)    ? w_value    : rb_value;
  wire logic [31:0] xc_value = (mc && w_we_value)    ? w_value    : rc_value;
  wire logic [2:0]  xa_tag   = (ma && w_we_tag)      ? w_tag      : ra_tag;
  wire logic [2:0]  xb_tag   = (mb && w_we_tag)      ? w_tag      : rb_tag;
  wire logic [2:0]  xc_tag   = (mc && w_we_tag)      ? w_tag      : rc_tag;
  wire logic        xa_ovf   = (ma && w_we_overflow) ? w_overflow : ra_overflow;
  wire logic        xb_ovf   = (mb && w_we_overflow) ? w_overflow : rb_overflow;
  wire logic        xc_ovf   = (mc && w_we_overflow) ? w_overflow : rc_overflow;
  wire logic        xa_flg   = (ma && w_we_flag)     ? w_flag     : ra_flag;
  wire logic        xb_flg   = (mb && w_we_flag)     ? w_flag     : rb_flag;
  wire logic        xc_flg   = (mc && w_we_flag)     ? w_flag     : rc_flag;

  // ---- predication ---------------------------------------------------------
  // k2g_core.sv:477-488. Zero and negative come from the register value on
  // demand; there is no global condition register (spec 7).
  logic cond_raw;
  always_comb begin
    unique case (uop.cond)
      CCK_OVERFLOW: cond_raw = xc_ovf;
      CCK_FLAG:     cond_raw = xc_flg;
      CCK_ZERO:     cond_raw = (xc_value == 32'd0);
      CCK_NEGATIVE: cond_raw = xc_value[31];
      CCK_POSITIVE: cond_raw = (xc_value != 32'd0) && !xc_value[31];
      default:      cond_raw = 1'b1;
    endcase
  end
  wire logic unpredicated = (uop.cond == CCK_NONE);
  wire logic cond_met     = unpredicated ? 1'b1 : (cond_raw ^ uop.cond_invert);

  // ---- reserved F32 operands -----------------------------------------------
  // k2g_core.sv:507-527, and the same A-before-B order as the emulator's
  // `check_no_f32_operands`, so FAULT_INFO agrees when both carry the tag.
  logic reads_a, reads_b;
  always_comb begin
    reads_a = 1'b0;
    reads_b = 1'b0;
    unique case (uop.kind)
      UOP_COPY, UOP_LOAD, UOP_CSP_LOAD, UOP_BEXT: reads_b = 1'b1;
      UOP_STORE, UOP_CSP_STORE, UOP_BINS: begin reads_a = 1'b1; reads_b = 1'b1; end
      UOP_UNARY: reads_a = 1'b1;
      UOP_ARITH, UOP_LOGIC, UOP_SHIFT, UOP_CMP: begin
        reads_a = 1'b1;
        reads_b = !uop.use_imm;
      end
      UOP_PREP_JUMP: reads_a = (uop.jump_kind != JT_REL_IMM);
      default: ;
    endcase
  end

  wire logic f32_a    = reads_a && (xa_tag == RDT_F32);
  wire logic f32_b    = reads_b && (xb_tag == RDT_F32);
  wire logic f32_cond = !unpredicated && (xc_tag == RDT_F32);

  // ---- functional units --------------------------------------------------
  // An immediate carries no tag of its own, so it takes the left operand's
  // interpretation (k2g_core.sv:553-556).
  wire logic [31:0] alu_b     = uop.use_imm ? uop.imm : xb_value;
  wire logic [2:0]  alu_b_tag = uop.use_imm ? xa_tag  : xb_tag;

  logic [31:0] arith_result, logic_result, unary_result;
  logic        arith_overflow, cmp_result;

  k2g_alu u_alu (
      .a(xa_value), .b(alu_b),
      .a_tag(rdt_e'(xa_tag)), .b_tag(rdt_e'(alu_b_tag)),
      .arith_op(uop.arith_op), .logic_op(uop.logic_op),
      .cmp_op(uop.cmp_op), .unary_op(uop.unary_op),
      .arith_result(arith_result), .arith_overflow(arith_overflow),
      .logic_result(logic_result), .cmp_result(cmp_result),
      .unary_result(unary_result)
  );

  wire logic [4:0] amount = uop.use_imm ? uop.imm[4:0] : xb_value[4:0];
  logic [31:0] shift_result, bext_result, bins_result;

  k2g_shift u_shift (
      .value(xa_value), .src(xb_value), .amount(amount),
      .shift_op(uop.shift_op),
      .bm_start(uop.bm_start), .bm_span(uop.bm_span),
      .shift_result(shift_result), .bext_result(bext_result),
      .bins_result(bins_result)
  );

  // ---- the address adder -------------------------------------------------
  // read -> forward -> add, in one cycle (k2g_core.sv:596).
  wire logic        is_load     = (uop.kind == UOP_LOAD);
  wire logic [31:0] load_addr   = xb_value + uop.imm;
  wire logic [31:0] store_addr  = xa_value + uop.imm;
  wire logic [31:0] access_addr = is_load ? load_addr : store_addr;
  wire logic [2:0]  access_tag  = is_load ? uop.datakind : xb_tag;

  // ---- alignment and range -------------------------------------------------
  // k2g_core.sv:626-635. Checked before anything commits, which is what makes
  // faults precise (spec 8).
  wire logic [2:0] access_width = rdt_width_bytes(access_tag);
  wire logic       is_store     = (uop.kind == UOP_STORE);
  wire logic       is_access    = is_load || is_store;

  logic misaligned;
  always_comb begin
    unique case (access_width)
      3'd1:    misaligned = 1'b0;
      3'd2:    misaligned = access_addr[0];
      default: misaligned = |access_addr[1:0];
    endcase
  end
  wire logic out_of_range = (access_addr + {29'd0, access_width}) > MEM_BYTES[31:0];

  // ---- fault detection -----------------------------------------------------
  // k2g_core.sv:815-848, less the CSP and PREP_JUMP arms, which are not in
  // this slice. The order is architecture: the emulator rejects an `F32`
  // operand before the instruction executes, so an `F32` address register
  // faults as FP rather than as whatever address it would have computed.
  fault_e      n_fault;
  logic        n_fault_valid;
  logic [31:0] n_fault_info;

  always_comb begin
    n_fault       = FAULT_NONE;
    n_fault_valid = 1'b0;
    n_fault_info  = 32'd0;

    if (uop_valid && f32_cond) begin
      n_fault       = FAULT_FP_UNIMPLEMENTED;
      n_fault_valid = 1'b1;
      n_fault_info  = {27'd0, uop.cond_reg};
    end else if (uop_valid && cond_met) begin
      if (uop.kind == UOP_FAULT) begin
        n_fault       = uop.fault;
        n_fault_valid = 1'b1;
        n_fault_info  = uop.imm;
      end else if (f32_a || f32_b) begin
        n_fault       = FAULT_FP_UNIMPLEMENTED;
        n_fault_valid = 1'b1;
        n_fault_info  = f32_a ? {27'd0, uop.dst} : {27'd0, uop.src};
      end else if (is_access && misaligned) begin
        n_fault       = is_load ? FAULT_MISALIGNED_LOAD : FAULT_MISALIGNED_STORE;
        n_fault_valid = 1'b1;
        n_fault_info  = access_addr;
      end else if (is_access && out_of_range) begin
        n_fault       = FAULT_ADDR_OUT_OF_RANGE;
        n_fault_valid = 1'b1;
        n_fault_info  = access_addr;
      end
    end
  end

  // Nothing commits behind a fault, and nothing commits on a predicate that
  // did not hold.
  wire logic commits = uop_valid && cond_met && !n_fault_valid;

  // ---- the writeback packet ----------------------------------------------
  logic        n_we_value, n_we_tag, n_we_overflow, n_we_flag;
  logic [4:0]  n_addr;
  logic [31:0] n_value;
  logic [2:0]  n_tag;
  logic        n_overflow, n_flag;

  always_comb begin
    n_we_value    = 1'b0;
    n_we_tag      = 1'b0;
    n_we_overflow = 1'b0;
    n_we_flag     = 1'b0;
    n_addr        = uop.dst;
    n_value       = 32'd0;
    n_tag         = xa_tag;
    n_overflow    = 1'b0;
    n_flag        = 1'b0;

    unique case (uop.kind)
      UOP_PUT_IMM: begin
        n_we_value = commits;
        n_we_tag   = commits;
        n_value    = uop.imm;
        n_tag      = uop.datakind;
      end
      UOP_SET_TAG: begin
        n_we_tag = commits;
        n_tag    = uop.datakind;
      end
      UOP_COPY: begin
        n_we_value    = commits;
        n_we_tag      = commits;
        n_we_overflow = commits;
        n_we_flag     = commits;
        n_value       = xb_value;
        n_tag         = xb_tag;
        n_overflow    = xb_ovf;
        n_flag        = xb_flg;
      end
      UOP_ARITH: begin
        n_we_value    = commits;
        n_we_overflow = commits;
        n_value       = arith_result;
        n_overflow    = arith_overflow;
      end
      UOP_LOGIC: begin
        // The FLAG prefix operates on flag bits instead of values
        // (k2g_core.sv:981).
        if (uop.on_flags) begin
          unique case (uop.logic_op)
            LOGIC_AND: n_flag = xa_flg & xb_flg;
            LOGIC_OR:  n_flag = xa_flg | xb_flg;
            default:   n_flag = xa_flg ^ xb_flg;
          endcase
          n_we_flag = commits;
        end else begin
          n_we_value = commits;
          n_value    = logic_result;
        end
      end
      UOP_UNARY: begin
        if (uop.on_flags) begin
          n_flag    = ~xa_flg;
          n_we_flag = commits;
        end else begin
          n_we_value = commits;
          n_value    = unary_result;
        end
      end
      UOP_CMP: begin
        n_we_flag = commits;
        n_flag    = cmp_result;
      end
      UOP_SHIFT: begin
        n_we_value = commits;
        n_value    = shift_result;
      end
      UOP_BEXT: begin
        n_we_value = commits;
        n_value    = bext_result;
      end
      UOP_BINS: begin
        n_we_value = commits;
        n_value    = bins_result;
      end
      UOP_LOAD, UOP_STORE: begin
        n_we_value = 1'b0;
        n_value    = access_addr;
        n_tag      = access_tag;
      end
      default: n_we_value = 1'b0;
    endcase
  end

  assign packet = {n_we_value, n_we_tag, n_we_overflow, n_we_flag,
                   n_addr, n_value, n_tag, n_overflow, n_flag,
                   n_fault_valid, n_fault, n_fault_info};

  // ---- W -----------------------------------------------------------------
  always_ff @(posedge clk) begin
    if (!rst_n) begin
      w_we_value    <= 1'b0;
      w_we_tag      <= 1'b0;
      w_we_overflow <= 1'b0;
      w_we_flag     <= 1'b0;
      w_addr        <= 5'd0;
      w_value       <= 32'd0;
      w_tag         <= 3'd0;
      w_overflow    <= 1'b0;
      w_flag        <= 1'b0;
    end else begin
      w_we_value    <= n_we_value;
      w_we_tag      <= n_we_tag;
      w_we_overflow <= n_we_overflow;
      w_we_flag     <= n_we_flag;
      w_addr        <= n_addr;
      w_value       <= n_value;
      w_tag         <= n_tag;
      w_overflow    <= n_overflow;
      w_flag        <= n_flag;
    end
  end

endmodule
