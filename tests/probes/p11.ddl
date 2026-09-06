-- Probe: a register may silently take the name of a pipe's generated port.
--
-- A pipe `ans` generates `ans_data`, `ans_wsalt`, `ans_rsalt`. A register
-- called `ans_data` is accepted by DDL and emits all three of these into one
-- module:
--
--   output [15:0] ans_data          -- the port
--   reg    [7:0]  ans_data;         -- the register
--   assign ans_data = {ans_e1, ans_e0};
--
-- which is a width mismatch, a duplicate declaration and a continuous
-- assignment to a reg.
process p11 (i: buffer in u8, ans: buffer out u8)
  var ans_data: u8 = @zeroed()
  loop
    let x = @rcv(i)
    ans_data = x + 8'd1
    @send(ans, ans_data)
