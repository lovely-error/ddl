-- DUT-2: the compiler's own salt link, reduced to its two endpoints.
--
-- A pure producer and a pure consumer rather than two relays, so the only
-- object crossing between the two clock domains is ONE pipe. `gen` owns the
-- write salt and both entries; `chk` owns the read salt. Put the two on
-- different clocks and the wire between them is exactly what a `graph` emits
-- today for two instances -- src/ir_graph.rs:762-764 pushes `o_wsalt`,
-- `o_rsalt` and `o_data` straight across with nothing in between.
--
-- Built with --bare-export so the salt ports stay raw. Wrapping these in the
-- FIFO adapter would put an adapter's own state between the crossing and the
-- checker, which is the thing being measured.
--
-- `chk` forwards rather than checks: a `process` cannot carry a `wire out`, so
-- the counter comparison lives in the top where it can reach a pin.

process gen (o: buffer out u16)
  var n: u16 = 0
  loop
    @send(o, n)
    n += 1

process chk (src: buffer in u16, dst: buffer out u16)
  loop
    let v = @rcv(src)
    @send(dst, v)
