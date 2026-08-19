-- Composition: three stages wired into one design.
--
-- Every other declaration kind in DDL flattens. A `fun` call is inlined, and
-- one `process` or `sequence` becomes one module with no hierarchy inside it.
-- A `graph` is the other direction and the only place an instance comes from:
-- it computes nothing, and what it emits is wires and instantiations.
--
-- What is worth noticing is what is NOT written here. There is no `valid`, no
-- `ready`, no FIFO, and no depth. `let stage1: buffer i16` says two blocks
-- are connected and what travels between them; the handshake on both ends was
-- generated when those blocks were compiled, and connecting them is three
-- wires because both ends already agree on what the three wires mean.
--
-- The rule the checker enforces is one producer and one consumer per pipe.
-- Two producers would be two drivers on one net, which Verilog resolves to `x`
-- rather than diagnosing; a pipe with no producer sits at `z` and reads, in
-- simulation, as an intermittent hang. Neither is a mistake worth making at
-- three in the morning on a board.

import "mul3.ddl"

sequence add_one (src: buffer in i16, dst: buffer out i16)
  let a = @rcv(src)
  |||
  let b: i16 = a + 16'd1
  @send(dst, b)

sequence saturate (src: buffer in i32, dst: buffer out i32)
  let a = @rcv(src)
  |||
  -- Clamp to 16 bits' worth, so the stage does something a mux can show.
  let too_big: i1 = a > 32'd65535
  let b: i32 = if too_big then 32'd65535 else a
  @send(dst, b)

-- `mul3` comes from mul3.ddl: i16 in, i32 out, three pipeline stages.
--
-- Latency here is add_one's 1, plus mul3's 3, plus saturate's 1 -- and
-- throughput is still one item per cycle, because every stage is its own
-- pipeline and the pipes between them carry the back-pressure.
graph scaler (src: buffer in i16, dst: buffer out i32)
  let bumped: buffer i16
  let scaled: buffer i32

  add_one(src, bumped)
  mul3(bumped, scaled)
  saturate(scaled, dst)
