sequence seq_scope (src: buffer in u16, dst: buffer out u16)
  let a = @rcv(src)
  let x: u16 = 10
  |||
  if a > 16'd5 then
    let x: u32 = 20
  |||
  @send(dst, x)

sequence seq_address (src: buffer in u16, dst: buffer out u16)
  var mem: #[impl(bram)] [u16; 16]
  let a = @rcv(src)
  let addr = @trunc(a, 4)
  |||
  let addr = mem[addr]
  |||
  @send(dst, addr)

sequence seq_read_first (src: buffer in u4, dst: buffer out u16)
  var mem: #[impl(bram)] [u16; 16]
  let addr = @rcv(src)
  let old = mem[addr]
  mem[addr] = 16'd42
  |||
  @send(dst, old)

sequence seq_assert (src: buffer in u16, dst: buffer out u16)
  let a = @rcv(src)
  @assert(a != 16'd0)
  |||
  @assert(a != 16'd0)
  |||
  @assert(a != 16'd0)
  @send(dst, a)

sequence seq_literal (src: buffer in u16, dst: buffer out u16)
  let a = @rcv(src)
  |||
  @send(dst, 42)

sequence seq_join (a: buffer in u16, b: buffer in u16, s: buffer out u16, d: buffer out u16)
  let x = @rcv(a)
  let y = @rcv(b)
  |||
  @send(s, x + y)
  @send(d, x - y)

sequence seq_optional (a: buffer in u16, b: buffer in u16, o: buffer out u16)
  let x = @rcv(a)
  let (y, ok) = @try_rcv(b)
  var t: u16 = x
  if ok then
    t = x + y
  |||
  @send(o, t)

sequence seq_early (src: buffer in u16, x: buffer out u16, y: buffer out u16)
  let a = @rcv(src)
  @send(x, a)
  |||
  let b: u16 = a + 16'd1
  |||
  @send(y, b + 16'd1)
