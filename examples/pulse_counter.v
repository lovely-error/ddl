// GENERATED FILE -- DO NOT EDIT BY HAND
//
// Regenerate with: ddl build examples/pulse_counter.ddl -o examples/pulse_counter.v
//
// Verilog-2005. No `$clog2`, no width casts in expressions and no
// function calls: all three make GowinSynthesis exit with an empty log.

module pulse_counter (
    input         clk,
    input         rst_n,
    input         tick,
    input         tick_en,
    input         clear,
    input         clear_en,
    output [15:0] count,
    output        count_en
);

  reg [15:0] n;

  wire [15:0] count_1 = (clear_en & clear) ? 16'd0 : ((tick_en & tick) ? (n + 16'd1) : n);
  wire count_en_1 = tick_en & tick;

  assign count = count_1;
  assign count_en = count_en_1;

  always @(posedge clk) begin
    if (!rst_n) begin
      n <= 16'd0;
    end else begin
      n <= count_1;
    end
  end

endmodule
