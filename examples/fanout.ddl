-- Combinators, and a module DDL did not compile.
--
-- Two producers share one pipe and one producer reaches two consumers. Both
-- shapes could be written as a `process`, and both would then cost a cycle per
-- hop: a process is a state machine and a state is a cycle, which for
-- something whose whole job is to pass an item along is the wrong price. So
-- `@merge` and `@split` are modules the compiler writes -- a datapath, no
-- states -- and the graph instantiates them like anything else.
--
-- WHAT `@merge` IS NOT. It is not two instances naming one pipe. That would be
-- two drivers on one salt, which the simulator resolves to `x` and the graph
-- refuses. `@merge` grants one input per cycle, with a rotating priority so a
-- busy `lo` cannot starve `hi`, and each input's read pointer advances only on
-- its own transfer.
--
-- WHAT `@split` COSTS. A pair of entries per sink. The alternative is ANDing
-- the sinks' readys together, which is smaller and rebuilds exactly the
-- combinational coupling between two consumers that the salt protocol exists
-- to remove -- each one's back-pressure would then sit in the other's timing
-- path. The input waits for the slowest sink and nothing is ever dropped.
--
-- `sink_ext` is an `extern`: a module written by hand somewhere else, named
-- here so this graph can be the top level rather than a guest inside a
-- SystemVerilog one. Its three ports per pipe are the ones every other module
-- here has, so the hand-written file has one spelling to match.

extern sink_ext (a: buffer in u32)

sequence scale (src: buffer in u32, dst: buffer out u32)
  let x = @rcv(src)
  |||
  let doubled: u32 = x + x
  @send(dst, doubled)

graph fanout (hi: buffer in u32, lo: buffer in u32, out_a: buffer out u32)
  let picked: buffer u32
  let scaled: buffer u32
  let copy: buffer u32

  @merge(hi, lo, picked)
  scale(picked, scaled)
  @split(scaled, out_a, copy)
  sink_ext(copy)
