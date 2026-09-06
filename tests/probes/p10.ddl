-- Probe: a `var` declared inside a loop body of a blocking process is not a
-- register. It is re-initialised on every pass, so it folds to its
-- initialiser:
--
--   wire branch_s0 = (4'd0 + 4'd1) == 4'd8;
--
-- which is a constant 0, so the inner loop never ends. README documents that
-- a LEADING run of `var`s becomes registers; what this produces is not an
-- error but a counter that does not count.
process p10 (o: buffer out u1)
  var bits: u8 = 8'h41
  loop
    var n: u4 = @zeroed()
    loop
      @send(o, bits[0])
      n += 4'd1
      if n == 4'd8 then
        break
