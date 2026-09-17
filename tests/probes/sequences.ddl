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

sequence seq_side (src: buffer in u16, b: buffer in u16, o: buffer out u32)
  let x = @rcv(src)
  |||
  let (y, ok) = @try_rcv(b)
  var t: u16 = 16'd0
  if ok then
    t = y
  |||
  @send(o, {t, x})

sequence seq_peek_drop (src: buffer in u16, b: buffer in u16, o: buffer out u32)
  let x = @rcv(src)
  |||
  let (y, here) = @peek(b)
  var h: u16 = 16'd0
  if here & (y == x) then
    @drop(b)
    h = 16'd1
  @send(o, {h, x})

sequence seq_route (src: buffer in u16, odd: buffer out u16, all: buffer out u16)
  let a = @rcv(src)
  |||
  if a[0] == 1'd1 then
    @send(odd, a)
  |||
  @send(all, a)

sequence seq_blocked_early (src: buffer in u16, x: buffer out u16, y: buffer out u16)
  let a = @rcv(src)
  @send(x, a)
  |||
  let b: u16 = a
  |||
  @send(y, b)

sequence seq_offer (src: buffer in u16, side: buffer out u16, log: buffer out u32)
  let x = @rcv(src)
  |||
  let ok = @try_send(side, x)
  let f: u16 = if ok then 16'd1 else 16'd0
  |||
  @send(log, {f, x})
