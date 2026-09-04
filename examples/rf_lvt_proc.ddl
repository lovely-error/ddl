-- The same memory as examples/rf_lvt.ddl, in a `process` instead.
--
-- Not a duplicate. The two constructs reach the ports through different code
-- and come out a different shape, so proving one says nothing about the other:
--
--   WRITE PORTS. A sequence gates its writes on the stage that owns them being
--   live and the pipeline moving. A process settles them per STATE, muxing
--   every state that writes a port onto it and gating on that state firing --
--   so `--lvt-bram` here banks write ports that were assembled by the state
--   machine rather than by the stage loop.
--
--   READ PORTS. A sequence gives every read its own, because its stages are
--   live at once. A process muxes all of them onto one, because one state is
--   current. So this memory is banks with ONE replica each, where rf_lvt's is
--   banks with two -- the replication factor is the thing that differs, and a
--   test of one leaves the other unwritten.
--
--   FORWARDING. A sequence needs it: the array is written on the same edge
--   that fills the read register. A process does not, because a `bram` read is
--   scheduled into a state after any write's and the array has already been
--   updated by the time it looks. That is a claim, and it is what the model in
--   the testbench checks.
--
-- examples/tb_rf_lvt_proc_equiv.sv drives the default build, the `--lvt-bram`
-- build and a behavioural model with the same stimulus, and fails if any of
-- the three disagree.
--
-- WHAT IT MEASURES, on GowinSynthesis for the GW1NR-9C, beside rf_lvt's:
--
--   default build      no netlist. "ERROR (IF0008): The number(8192) of DFF
--                      used to infer `vals` exceeds the resource limit(6693)",
--                      the same wall the pipeline version hits and for the
--                      same reason -- two write ports is a shape this device
--                      has no cell for.
--   --lvt-bram         1532 cells, of which 2 SDPB and 473 DFF.
--
-- TWO SDPB, where rf_lvt is four. Same two banks, but one replica each instead
-- of two, because a process muxes its reads onto one port. That is the shape
-- difference between the constructs, showing up in the netlist -- and the
-- reason both are tested rather than one.
--
-- Equivalence: 16058 items over 64600 cycles, 0 mismatches. The testbench was
-- checked against three deliberate breaks -- the bank select inverted, write
-- priority reversed, and a bank missing one of its writes -- and caught all
-- three.

struct rf_proc_req
  wa0: u8
  wd0: u32
  wa1: u8
  wd1: u32
  ra0: u8
  ra1: u8

process rf_lvt_proc (cmd: buffer in rf_proc_req, rd: buffer out u64)
  var vals: #[impl(bram)] [u32;256]

  loop
    let q = @rcv(cmd)

    -- Two writes in one state: both happen in the cycle it fires, so they are
    -- two ports, and `--lvt-bram` gives each one its own bank.
    vals[q.wa0] = q.wd0
    vals[q.wa1] = q.wd1

    -- Two reads, each costing a state of its own -- and both muxed onto the
    -- memory's single read port, because only one of those states is current.
    let x = vals[q.ra0]
    let y = vals[q.ra1]

    @send(rd, {x, y})
