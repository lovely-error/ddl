-- (a) a state with only @try_rcv and a break
process p5a (src: buffer in u8, other: buffer in u8, dst: buffer out u8)
  loop
    let a = @rcv(src)
    loop
      let (b, got) = @try_rcv(other)
      if got then
        break
    @send(dst, a)

-- (b) a state with only @try_send and a break
process p5b (src: buffer in u8, dst: buffer out u8)
  loop
    let a = @rcv(src)
    loop
      let sent = @try_send(dst, a)
      if sent then
        break

-- (c) both in one state
process p5c (src: buffer in u8, other: buffer in u8, dst: buffer out u8)
  loop
    let a = @rcv(src)
    loop
      let (b, got) = @try_rcv(other)
      if got then
        break
      let sent = @try_send(dst, a)
      if sent then
        break
