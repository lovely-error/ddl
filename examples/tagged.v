// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/tagged.ddl -o examples/tagged.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module build_read (
    input  [7:0]  page,
    input  [7:0]  off,
    output [17:0] r
);

  assign r = {2'd1, {page, off}};

endmodule

module build_write (
    input  [7:0]  d,
    output [17:0] r
);

  assign r = {2'd2, d, 8'd0};

endmodule

module serve (
    input  [17:0] r,
    output        is_store,
    output [7:0]  page,
    output [7:0]  data
);

  wire [1:0] req_e_tag = r[17:16];
  wire [15:0] a = r[15:0];
  wire [7:0] d = r[15:8];
  reg [7:0] n12;
  reg n13;
  reg [7:0] n14;

  always @* begin
    case (req_e_tag)
      2'd1: n12 = a[15:8];
      default: n12 = 8'd0;
    endcase
  end

  always @* begin
    case (req_e_tag)
      2'd0: n13 = 1'b0;
      2'd1: n13 = 1'b0;
      2'd2: n13 = 1'b1;
      default: n13 = 1'b0;
    endcase
  end

  always @* begin
    case (req_e_tag)
      2'd2: n14 = d;
      default: n14 = 8'd0;
    endcase
  end

  assign is_store = n13;
  assign page = n12;
  assign data = n14;

endmodule
