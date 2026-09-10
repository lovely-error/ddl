// 8N1 UART transmitter, from KAMASUTRA2G/rtl/board/k2g_uart.sv, reduced to the
// transmitter and rewritten in the Verilog-2001 subset this build uses.
//
// DIVISOR is the report clock over the baud rate. The report is generated in
// the RECEIVER's clock domain, deliberately: every counter worth reporting
// lives there, and moving them to another domain to be printed would put a
// clock crossing inside the instrument measuring a clock crossing.

module cdc_uart_tx #(
    parameter DIVISOR = 703
) (
    input        clk,
    input        rst_n,
    input        valid,      // pulse to send `data`
    input  [7:0] data,
    output       ready,      // idle and able to accept
    output       tx
);

  reg [9:0]  shifter;
  reg [3:0]  bits_left;
  reg [15:0] tick;
  // Registered one cycle early. As a bare `tick == DIVISOR-1` this compare drove
  // the clock enable of the shifter and appeared in the critical path of the
  // whole receiving domain; the report clock has to meet timing or the counters
  // it prints are not the counters that were counted.
  reg        tick_end;
  // `bits_left == 0` as a register. As a compare it fed the shifter's D input
  // and was one of the last paths keeping the report clock from closing.
  reg        busy;

  assign ready = !busy;
  assign tx    = shifter[0];

  always @(posedge clk) begin
    if (!rst_n) begin
      shifter   <= 10'h3FF;                 // idle high
      bits_left <= 4'd0;
      tick      <= 16'd0;
      tick_end  <= 1'b0;
      busy      <= 1'b0;
    end else if (!busy) begin
      tick     <= 16'd0;
      tick_end <= 1'b0;
      if (valid) begin
        shifter   <= {1'b1, data, 1'b0};    // stop, data, start
        bits_left <= 4'd10;
        busy      <= 1'b1;
      end else begin
        shifter <= 10'h3FF;
      end
    end else if (tick_end) begin
      tick      <= 16'd0;
      tick_end  <= 1'b0;
      shifter   <= {1'b1, shifter[9:1]};
      bits_left <= bits_left - 4'd1;
      if (bits_left == 4'd1) busy <= 1'b0;
    end else begin
      tick     <= tick + 16'd1;
      tick_end <= (tick == (DIVISOR - 2));
    end
  end

endmodule
