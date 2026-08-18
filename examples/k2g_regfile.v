// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build E:/Code/ddl/target/verify/k2g_regfile/src.ddl -o E:/Code/ddl/examples/k2g_regfile.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module uop_nop (
    output [126:0] u
);

  assign u = 127'd0;

endmodule

module uop_fault (
    input  [4:0]   cause,
    input  [15:0]  cp,
    output [126:0] u
);

  wire [126:0] f = 127'd0;
  wire [126:0] n5 = {5'h12, f[121:0]};
  wire [126:0] n8 = {n5[126:37], cause, n5[31:0]};

  assign u = {n8[126:112], {16'd0, cp}, n8[79:0]};

endmodule

module k2g_regfile (
    input         clk,
    input         rst_n,
    input  [4:0]  ra_addr,
    input  [4:0]  rb_addr,
    input  [4:0]  rc_addr,
    input         we_value,
    input         we_tag,
    input         we_overflow,
    input         we_flag,
    input  [4:0]  w_addr,
    input  [31:0] w_value,
    input  [2:0]  w_tag,
    input         w_overflow,
    input         w_flag,
    output [31:0] ra_value,
    output [31:0] rb_value,
    output [31:0] rc_value,
    output [2:0]  ra_tag,
    output [2:0]  rb_tag,
    output [2:0]  rc_tag,
    output        ra_overflow,
    output        rb_overflow,
    output        rc_overflow,
    output        ra_flag,
    output        rb_flag,
    output        rc_flag
);

  reg [31:0] values [0:31];
  integer values_ix;
  reg [2:0] tags [0:31];
  integer tags_ix;
  reg overflow [0:31];
  integer overflow_ix;
  reg flagbit [0:31];
  integer flagbit_ix;

  wire [31:0] n31 = values[ra_addr];
  wire [31:0] n32 = values[rb_addr];
  wire [31:0] n33 = values[rc_addr];
  wire [2:0] n34 = tags[ra_addr];
  wire [2:0] n35 = tags[rb_addr];
  wire [2:0] n36 = tags[rc_addr];
  wire n37 = overflow[ra_addr];
  wire n38 = overflow[rb_addr];
  wire n39 = overflow[rc_addr];
  wire n40 = flagbit[ra_addr];
  wire n41 = flagbit[rb_addr];
  wire n42 = flagbit[rc_addr];

  assign ra_value = n31;
  assign rb_value = n32;
  assign rc_value = n33;
  assign ra_tag = n34;
  assign rb_tag = n35;
  assign rc_tag = n36;
  assign ra_overflow = n37;
  assign rb_overflow = n38;
  assign rc_overflow = n39;
  assign ra_flag = n40;
  assign rb_flag = n41;
  assign rc_flag = n42;

  // values [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (values_ix = 0; values_ix < 32; values_ix = values_ix + 1) values[values_ix] <= 32'd0;
    end else if (we_value) begin
      values[w_addr] <= w_value;
    end
  end

  // tags [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (tags_ix = 0; tags_ix < 32; tags_ix = tags_ix + 1) tags[tags_ix] <= 3'd2;
    end else if (we_tag) begin
      tags[w_addr] <= w_tag;
    end
  end

  // overflow [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (overflow_ix = 0; overflow_ix < 32; overflow_ix = overflow_ix + 1) overflow[overflow_ix] <= 1'b0;
    end else if (we_overflow) begin
      overflow[w_addr] <= w_overflow;
    end
  end

  // flagbit [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (flagbit_ix = 0; flagbit_ix < 32; flagbit_ix = flagbit_ix + 1) flagbit[flagbit_ix] <= 1'b0;
    end else if (we_flag) begin
      flagbit[w_addr] <= w_flag;
    end
  end

`ifdef SIMULATION
  always @(posedge clk) begin
    if (rst_n) begin
      if (!(((!we_tag) | (w_tag != 3'd7)))) $error("%m: the reserved tag encoding was written");
    end
  end
`endif

endmodule
