-- A plain Verilog module: ports, a register, and no pipes anywhere.
--
-- This is what `port` is for. A `buffer` is the right answer between two
-- things DDL compiled, and it costs two entries and a pair of salts to get
-- back-pressure that neither side of a boundary like this one can use. At the
-- EDGE of the program -- a pin, a PLL, a bus master that does not take `ready`
-- for an answer -- there is nothing to push back on, and README's rule for
-- that case is that the sink ties `ready` high AND SAYS SO AT THE BOUNDARY.
-- `port` is how it is said, and the saying is what a reader can check.
--
-- A PORT IS STILL A PIPE, so it is reached the way every other pipe is. `tick`
-- is not a wire this body can name: `@try_rcv` asks what is there this cycle
-- and answers with a pair, and the second half of that pair is the whole
-- reason not to bind the name to the wire. `tick` alone would be "the value",
-- and there is no cycle on which that means anything by itself.
--
-- What comes out is a module with a clock, a reset, one register and four
-- ports. No salt, no entries, no state register -- there is nothing
-- for them to do here, and a language that emitted them anyway would not be
-- usable for the ordinary sequential blocks a design needs beside the
-- dataflow ones.
--
-- `count_en` is high on exactly the cycles the body sent, which here is every
-- cycle a tick arrived. A consumer that samples `count` without looking at
-- `count_en` is reading a value that was true at some point, which is the
-- distinction the enable exists to keep.

process pulse_counter (tick: port in u1, clear: port in u1, count: port out u16)
  var n: u16 = @zeroed()

  loop
    let (t, got_tick) = @try_rcv(tick)
    let (c, got_clear) = @try_rcv(clear)

    -- A clear beats a tick on the cycle both arrive: the count after a clear
    -- is zero, not one, whatever else happened that cycle.
    if got_clear & c then
      n = @zeroed()
    else
      if got_tick & t then
        n += 16'd1

    -- Published every cycle a tick was seen. Sending unconditionally would
    -- make the enable a constant, which is a wire that says nothing.
    if got_tick & t then
      let _sent = @try_send(count, n)
