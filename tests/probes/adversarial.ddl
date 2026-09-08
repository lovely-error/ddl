process joined_if (c1: buffer in u1, c2: buffer in u1, i: buffer in u8, o1: buffer out u8, o2: buffer out u8)
  loop
    let cond1 = @rcv(c1)
    let cond2 = @rcv(c2)
    if cond1 then
      let x = @rcv(i)
      @send(o1, x)
    if cond2 then
      @send(o2, 8'd1)
    else
      @send(o2, 8'd2)

process for_read2 (src: buffer in u8, other: buffer in u8, dst: buffer out u8)
  loop
    let x = @rcv(src)
    let y = @rcv(other)
    var acc: u8 = 8'd0
    for i in 0..4
      acc += x
    @send(dst, acc)

process store_and_load (i: buffer in u8, o: buffer out u8)
  var tbl: #[impl(bram)] [u8; 16]
  loop
    let v = @rcv(i)
    tbl[0] = v
    let read_back = tbl[0]
    @send(o, read_back)

process store_before_read (o: buffer out u8)
  var tbl: #[impl(bram)] [u8; 16]
  loop
    tbl[0] = 8'd9
    let read_back = tbl[0]
    @send(o, read_back)

process conditional_forward (o: buffer out u8)
  var tbl: #[impl(bram)] [u8; 16]
  var choose: u1 = 1'b0
  loop
    tbl[0] = 8'd10
    if choose then
      tbl[0] = 8'd20
    let value = tbl[0]
    @send(o, value)
    choose = !choose

enum adversarial_choice: u1
  No
  Yes

process exclusive_match (c: buffer in adversarial_choice, o: buffer out u8)
  loop
    let flag = @rcv(c)
    match flag
      .No =>
        @try_send(o, 8'd2)
      .Yes =>
        @try_send(o, 8'd1)

process exclusive_port (c: buffer in u1, o: buffer out u8)
  loop
    let flag = @rcv(c)
    if flag then
      @try_send(o, 8'd1)
    else
      @try_send(o, 8'd2)
