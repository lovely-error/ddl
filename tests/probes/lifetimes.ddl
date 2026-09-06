-- Each request re-enters the scopes; an inner shadow must not clobber n.
process lifetimes (src: buffer in u8, o: buffer out u8)
  var outer: u8 = 8'd9
  loop
    let seed = @rcv(src)
    var n: u8 = seed
    loop
      @send(o, n)
      n += 8'd1
      if n == seed + 8'd3 then
        break
    if seed[0] then
      var n: u8 = 8'd100
      loop
        @send(o, n)
        n += 8'd1
        if n == 8'd102 then
          break
    @send(o, n)
    @send(o, outer)

-- A loop-local initializer may itself wait for a fresh value.
process received_local (src: buffer in u8, o: buffer out u8)
  loop
    var n: u8 = @rcv(src)
    loop
      @send(o, n)
      n -= 8'd1
      if n == 8'd0 then
        break
