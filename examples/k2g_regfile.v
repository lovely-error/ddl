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
    input          clk,
    input          rst_n,
    input          req_valid,
    output         req_ready,
    input  [60:0]  req_data,
    output         rsp_valid,
    input          rsp_ready,
    output [110:0] rsp_data
);

  reg rsp_busy;
  reg [110:0] rsp_hold;

  reg [31:0] values [0:31];
  integer values_ix;
  reg [2:0] tags [0:31];
  integer tags_ix;
  reg overflow [0:31];
  integer overflow_ix;
  reg flagbit [0:31];
  integer flagbit_ix;

  wire n23 = (!rsp_busy) | rsp_ready;
  wire req_xfer = req_valid & n23;
  wire [110:0] out = 111'd0;
  wire [31:0] n29 = values[req_data[60:56]];
  wire [110:0] n31 = {n29, out[78:0]};
  wire [31:0] n33 = values[req_data[55:51]];
  wire [110:0] n36 = {n31[110:79], n33, n31[46:0]};
  wire [31:0] n38 = values[req_data[50:46]];
  wire [110:0] n41 = {n36[110:47], n38, n36[14:0]};
  wire [2:0] n43 = tags[req_data[60:56]];
  wire [110:0] n46 = {n41[110:15], n43, n41[11:0]};
  wire [2:0] n48 = tags[req_data[55:51]];
  wire [110:0] n51 = {n46[110:12], n48, n46[8:0]};
  wire [2:0] n53 = tags[req_data[50:46]];
  wire [110:0] n56 = {n51[110:9], n53, n51[5:0]};
  wire n58 = overflow[req_data[60:56]];
  wire [110:0] n61 = {n56[110:6], n58, n56[4:0]};
  wire n63 = overflow[req_data[55:51]];
  wire [110:0] n66 = {n61[110:5], n63, n61[3:0]};
  wire n68 = overflow[req_data[50:46]];
  wire [110:0] n71 = {n66[110:4], n68, n66[2:0]};
  wire n73 = flagbit[req_data[60:56]];
  wire [110:0] n76 = {n71[110:3], n73, n71[1:0]};
  wire n78 = flagbit[req_data[55:51]];
  wire [110:0] n81 = {n76[110:2], n78, n76[0]};
  wire n83 = flagbit[req_data[50:46]];
  wire n92 = req_data[44];

  assign req_ready = n23;
  assign rsp_valid = rsp_busy;
  assign rsp_data = rsp_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      rsp_busy <= 1'b0;
      rsp_hold <= 111'd0;
    end else begin
      rsp_busy <= (req_xfer ? 1'b1 : (rsp_ready ? 1'b0 : rsp_busy));
      rsp_hold <= (req_xfer ? {n81[110:1], n83} : rsp_hold);
    end
  end

  // values [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (values_ix = 0; values_ix < 32; values_ix = values_ix + 1) values[values_ix] <= 32'd0;
    end else if (req_data[45]) begin
      values[req_data[41:37]] <= req_data[36:5];
    end
  end

  // tags [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (tags_ix = 0; tags_ix < 32; tags_ix = tags_ix + 1) tags[tags_ix] <= 3'd2;
    end else if (n92) begin
      tags[req_data[41:37]] <= req_data[4:2];
    end
  end

  // overflow [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (overflow_ix = 0; overflow_ix < 32; overflow_ix = overflow_ix + 1) overflow[overflow_ix] <= 1'b0;
    end else if (req_data[43]) begin
      overflow[req_data[41:37]] <= req_data[1];
    end
  end

  // flagbit [0:31] -- distributed RAM: one sync write port, async reads
  always @(posedge clk) begin
    if (!rst_n) begin
      for (flagbit_ix = 0; flagbit_ix < 32; flagbit_ix = flagbit_ix + 1) flagbit[flagbit_ix] <= 1'b0;
    end else if (req_data[42]) begin
      flagbit[req_data[41:37]] <= req_data[0];
    end
  end

`ifdef SIMULATION
  always @(posedge clk) begin
    if (rst_n) begin
      if (!(((!n92) | (req_data[4:2] != 3'd7)))) $error("%m: the reserved tag encoding was written");
    end
  end
`endif

endmodule
