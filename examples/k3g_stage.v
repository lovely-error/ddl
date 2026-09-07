// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/k3g_stage.ddl -o examples/k3g_stage.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module k3g_stage (
    input         clk,
    input         rst_n,
    input  [1:0]  iops_wsalt,
    output [1:0]  iops_rsalt,
    input  [63:0] iops_data,
    output [1:0]  uops_wsalt,
    input  [1:0]  uops_rsalt,
    output [97:0] uops_data
);

  reg [1:0] iops_rsalt_q;
  reg [48:0] uops_e0;
  reg [48:0] uops_e1;
  reg [1:0] uops_wsalt_q;

  wire iops_ridx = iops_rsalt_q[0] ^ iops_rsalt_q[1];
  wire [31:0] iops_item = iops_ridx ? iops_data[63:32] : iops_data[31:0];
  wire uops_full = uops_wsalt_q == (~uops_rsalt);
  wire uops_room = !uops_full;
  wire iops_empty = iops_wsalt == iops_rsalt_q;
  wire iops_xfer = !iops_empty;
  wire iops_empty_1 = iops_wsalt == iops_rsalt_q;
  wire iops_present = !iops_empty_1;
  wire [48:0] out = 49'd0;
  wire [48:0] n28 = {iops_item[31:29], out[45:0]};
  wire [48:0] n33 = {n28[48:46], iops_item[28:26], n28[42:0]};
  wire [48:0] n37 = {n33[48:43], iops_item[25:21], n33[37:0]};
  wire [48:0] n41 = {n37[48:38], iops_item[20:16], n37[32:0]};
  wire [48:0] n47 = {n41[48:33], {16'd0, iops_item[15:0]}, n41[0]};
  wire [2:0] n49 = iops_item[28:26];
  reg [48:0] n56;
  wire taken = uops_room & iops_present;
  wire n58 = iops_present & taken;
  wire consumed = iops_xfer & n58;
  wire iops_take = iops_xfer & n58;
  wire uops_push = uops_room & iops_present;
  wire uops_widx = uops_wsalt_q[0] ^ uops_wsalt_q[1];

  always @* begin
    case (n49)
      3'd3, 3'd4: n56 = {n47[48:1], 1'b1};
      default: n56 = {n47[48:1], 1'b0};
    endcase
  end

  assign iops_rsalt = iops_rsalt_q;
  assign uops_wsalt = uops_wsalt_q;
  assign uops_data = {uops_e1, uops_e0};

  always @(posedge clk) begin
    if (!rst_n) begin
      iops_rsalt_q <= 2'd0;
      uops_e0 <= 49'd0;
      uops_e1 <= 49'd0;
      uops_wsalt_q <= 2'd0;
    end else begin
      iops_rsalt_q <= (iops_take ? (iops_rsalt_q ^ (iops_ridx ? 2'd2 : 2'd1)) : iops_rsalt_q);
      uops_e0 <= ((uops_push & (!uops_widx)) ? n56 : uops_e0);
      uops_e1 <= ((uops_push & uops_widx) ? n56 : uops_e1);
      uops_wsalt_q <= (uops_push ? (uops_wsalt_q ^ (uops_widx ? 2'd2 : 2'd1)) : uops_wsalt_q);
    end
  end

`ifdef SIMULATION
  always @(posedge clk) begin
    if (rst_n) begin
      if (!(((!(iops_present & taken)) | consumed))) $error("%m: @assert failed");
    end
  end
`endif

endmodule
