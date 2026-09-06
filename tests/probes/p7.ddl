-- Probe: `@sext` of a part select, and `@trunc` of an expression.
-- Both lower to a part select whose base is not a name, which Verilog-2005
-- does not have. The DDL side is accepted; the Verilog side does not parse.

-- emits: wire signed [31:0] n2 = {{24{v[7:0][7]}}, v[7:0]};
fun p7a (v: u32, o: out u32)
  o = @unsigned(@sext(v[7..0], 32))

-- emits: assign o = ((v >> k)[3:0]);
fun p7b (v: u32, k: u5, o: out u4)
  o = @trunc(v >> k, 4)
