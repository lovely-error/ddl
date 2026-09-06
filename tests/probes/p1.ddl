-- Probe: `@try_rcv` beside a blocking op in one process.
process p1 (a: buffer in u8, b: buffer in u8, o: buffer out u8)
  var acc: u8 = @zeroed()
  loop
    let x = @rcv(a)
    let (y, got) = @try_rcv(b)
    if got then
      acc = x + y
    else
      acc = x
    @send(o, acc)
