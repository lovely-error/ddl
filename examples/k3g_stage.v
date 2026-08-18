// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build E:/Code/ddl/examples/k3g_stage.ddl -o E:/Code/ddl/examples/k3g_stage.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module k3g_stage (
    input         clk,
    input         rst_n,
    input         iops_valid,
    output        iops_ready,
    input  [31:0] iops_data,
    output        uops_valid,
    input         uops_ready,
    output [48:0] uops_data
);

  reg uops_busy;
  reg [48:0] uops_hold;

  wire _taken = (!uops_busy) | uops_ready;
  wire iops_xfer = iops_valid & _taken;
  wire [48:0] out = 49'd0;
  wire [48:0] n14 = {iops_data[31:29], out[45:0]};
  wire [48:0] n19 = {n14[48:46], iops_data[28:26], n14[42:0]};
  wire [48:0] n23 = {n19[48:43], iops_data[25:21], n19[37:0]};
  wire [48:0] n27 = {n23[48:38], iops_data[20:16], n23[32:0]};
  wire [48:0] n33 = {n27[48:33], {16'd0, iops_data[15:0]}, n27[0]};
  wire [2:0] n35 = iops_data[28:26];
  reg [48:0] n42;

  always @* begin
    case (n35)
      3'd3, 3'd4: n42 = {n33[48:1], 1'b1};
      default: n42 = {n33[48:1], 1'b0};
    endcase
  end

  assign iops_ready = _taken;
  assign uops_valid = uops_busy;
  assign uops_data = uops_hold;

  always @(posedge clk) begin
    if (!rst_n) begin
      uops_busy <= 1'b0;
      uops_hold <= 49'd0;
    end else begin
      uops_busy <= (iops_xfer ? 1'b1 : (uops_ready ? 1'b0 : uops_busy));
      uops_hold <= (iops_xfer ? n42 : uops_hold);
    end
  end

endmodule
