-- Probe: the statements of a state are scheduled as one step, and both the
-- barrier payload and the branch condition are lowered from the environment
-- as it stands at the END of the state rather than where they were written.
--
-- Emits:
--   wire [3:0] n18 = k + 4'd1;
--   wire branch_s0 = n18 == 4'd7;                 -- test sees the increment
--   o_e0 <= ((fire_s0 & (!o_widx)) ? n18 : o_e0); -- send sees it too
--
-- Expected: 0 1 2 3 4 5 6 7 then stop. Emitted: 1 2 3 4 5 6 7 then stop --
-- the first value never appears and the loop ends one iteration early.
process p9 (o: buffer out u4)
  var k: u4 = @zeroed()
  loop
    @send(o, k)
    if k == 4'd7 then
      break
    k += 4'd1
