enum scoped_event
  Narrow(u8)
  Wide(u16)

process arm_scopes (src: buffer in scoped_event, o: buffer out u16)
  loop
    let event = @rcv(src)
    match event
      .Narrow n =>
        var value: u16 = @zext(n, 16)
        @send(o, value)
        value += 16'd1
        @send(o, value)
      .Wide n =>
        var value: u16 = n
        @send(o, value)
        value += 16'd2
        @send(o, value)
