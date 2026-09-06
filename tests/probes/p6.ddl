-- Probe: a variant whose payload is NARROWER than the widest one.
struct wide_t
  a: u32
  b: u32

enum ev_e
  Small(u8)
  Big(wide_t)

fun pack_small (x: u8, o: out ev_e)
  o = Small(x)

fun unpack (e: ev_e, o: out u8)
  var r: u8 = @zeroed()
  match e
    .Small s =>
      r = s
    .Big w =>
      r = w.a[7..0]
  o = r
