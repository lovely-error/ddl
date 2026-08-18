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
(
    input  logic       clk,
    input  logic       rst_n,

    input  uop_t       uop,
    input  logic       uop_valid,   // an instruction occupies X this cycle

    // The packet X computed, packed the way the DDL packs `wb_t`: first field
    // in the high bits.
    output logic [45:0] packet
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
  wire logic        xa_flg   = (ma && w_we_flag)     ? w_flag     : ra_flag;
  wire logic        xb_flg   = (mb && w_we_flag)     ? w_flag     : rb_flag;

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
        n_we_value = uop_valid;
        n_we_tag   = uop_valid;
        n_value    = uop.imm;
        n_tag      = uop.datakind;
      end
      UOP_SET_TAG: begin
        n_we_tag = uop_valid;
        n_tag    = uop.datakind;
      end
      UOP_COPY: begin
        n_we_value    = uop_valid;
        n_we_tag      = uop_valid;
        n_we_overflow = uop_valid;
        n_we_flag     = uop_valid;
        n_value       = xb_value;
        n_tag         = xb_tag;
        n_overflow    = xb_ovf;
        n_flag        = xb_flg;
      end
      UOP_ARITH: begin
        n_we_value    = uop_valid;
        n_we_overflow = uop_valid;
        n_value       = arith_result;
        n_overflow    = arith_overflow;
      end
      UOP_LOGIC: begin
        n_we_value = uop_valid;
        n_value    = logic_result;
      end
      UOP_UNARY: begin
        n_we_value = uop_valid;
        n_value    = unary_result;
      end
      UOP_CMP: begin
        n_we_flag = uop_valid;
        n_flag    = cmp_result;
      end
      UOP_SHIFT: begin
        n_we_value = uop_valid;
        n_value    = shift_result;
      end
      UOP_BEXT: begin
        n_we_value = uop_valid;
        n_value    = bext_result;
      end
      UOP_BINS: begin
        n_we_value = uop_valid;
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
                   n_addr, n_value, n_tag, n_overflow, n_flag};

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
