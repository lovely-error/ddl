-- A memory with several ports, in a pipeline, read across several stages.
--
-- Everything the memory model claims, in one module, so that a simulation can
-- disagree with it:
--
--   TWO WRITE PORTS. `wa0` and `wa1` are both written every item, so they are
--   two ports rather than one: they happen in the same cycle and there is
--   nothing to mux them onto. When they name the same address the later write
--   wins, which the testbench forces often enough to matter.
--
--   TWO READ PORTS. Every stage of a pipeline is live at once, so two reads in
--   one stage are two addresses in one cycle. A process would mux them onto
--   one port; this cannot.
--
--   FORWARDING. The array is written on the same edge that fills the read
--   registers, so a read of an address this item just wrote would otherwise
--   see the value from before it. What each read sees is decided back in the
--   stage that asked -- where both addresses exist -- and carried across the
--   cut as one bit and the data.
--
--   SEVERAL STAGES. `x` and `y` appear in stage 1 and are still needed in
--   stage 2; `sum` is computed in stage 1 and needed in stage 3. A value that
--   crosses two cuts needs two registers, not one, or it arrives beside the
--   item behind it -- and a pipeline that mis-registers a crossing value still
--   agrees with itself, so only a model of what SHOULD come out can catch it.
--
-- examples/tb_rf_lvt_equiv.sv drives the default build, the `--lvt-bram`
-- build and a behavioural model of the memory with the same stimulus, and
-- fails if any of the three disagree. Two builds of one wrong idea agree
-- perfectly, which is why the model is there and not just the A/B.
--
-- WHAT IT MEASURES, on GowinSynthesis for the GW1NR-9C rather than assumed:
--
--   default build      no netlist at all.
--                      "ERROR (IF0008): The number(8192) of DFF used to infer
--                      `vals` exceeds the resource limit(6693) of current
--                      device". Two write ports is a shape this device has no
--                      cell for, so the array falls back to flip-flops -- 256
--                      by 32 of them -- and does not fit.
--   --lvt-bram         2303 cells, of which 4 SDPB and 811 DFF. The four are
--                      2 banks by 2 read replicas; the flip-flops are the
--                      pipeline plus the 256x1 live value table.
--
-- So this is not "the flag makes it smaller". Written as the source means it,
-- the design does not build for this part; the flag is what makes it exist.
-- k2g_regfile.sv:18-24 recorded the same effect one step earlier, on an array
-- small enough that the fallback still fit.
--
-- Equivalence: 26440 items over 45400 cycles, 0 mismatches. The testbench was
-- checked against five deliberate breaks -- a value registered once where it
-- crosses two cuts, an LVT bank select inverted, forwarding dropped on one
-- read, a read enable let loose from `shift`, and write priority reversed --
-- and caught all five.

struct rf_req
  wa0: u8
  wd0: u32
  wa1: u8
  wd1: u32
  ra0: u8
  ra1: u8

sequence rf_lvt (req: buffer in rf_req, resp: buffer out u128)
  var vals: #[impl(bram)] [u32;256]

  -- Stage 0 owns the memory: both writes and both reads are here, so the only
  -- order anything depends on is the order these lines are written in.
  let q = @rcv(req)
  vals[q.wa0] = q.wd0
  vals[q.wa1] = q.wd1
  let x = vals[q.ra0]
  let y = vals[q.ra1]

  |||

  -- Stage 1 is where the reads land. `x` and `y` are used here AND below.
  let sum: u32 = x + y

  |||

  -- Stage 2 uses `x` and `y` again, so both crossed this far; `sum` is only
  -- passing through, and has to arrive beside the item it belongs to.
  let dif: u32 = x - y

  |||

  @send(resp, {x, y, sum, dif})
