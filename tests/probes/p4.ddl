struct it_t
  a: u8
  e: u1

process p4 (src: buffer in it_t, dst: buffer out u8, o2: buffer out u8)
  var busy: u1 = 1'b0
  var acc: u8 = @zeroed()
  var mem: #[impl(lutram)] [u8; 32] = @zeroed()
  var ix: u5 = @zeroed()
  loop
    for k in mem
      @assert(mem[k] != 8'hFF, "the poison value reached the array")

    let (cand, present) = @peek(src)
    let stale: u1 = present & cand.e
    let fire: u1 = present & !stale & !busy
    var consumed: u1 = 1'b0
    if stale | fire then
      consumed = @drop(src)
    @assert(consumed == (stale | fire), "the drop did not take what peek offered")

    var ok: u1 = 1'b0
    if fire then
      ok = @try_send(dst, cand.a)
      acc = mem[cand.a[4..0]]
      mem[ix] = cand.a
      ix += 5'd1
    busy = !ok

    let _s2 = @try_send(o2, acc)
