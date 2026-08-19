-- A tagged union: one pipe carrying requests that are not the same shape.
--
-- The alternative is a struct with every field of every request in it and a
-- kind field saying which ones are live. That works, and it is bigger than it
-- needs to be, and nothing stops a reader from taking `addr` out of a request
-- that never set one. The tag and the payload being one type is what makes
-- that unreachable rather than merely discouraged.
--
-- THE LAYOUT IS `{tag, payload}`, tag in the high bits, the same way a struct
-- puts its first field there. Every variant is the same width -- a narrower
-- payload is padded below itself -- so the tag is always in the same place and
-- a `match` reads it without knowing yet what it is looking at.
--
-- The width is derived and cannot be written down. `enum req_e: i2` would read
-- as "two bits wide" and the value is 18; reading the annotation as the TAG
-- width instead would mean the same syntax names the whole value on an enum
-- with no payloads and part of it here, and a struct field budgeted from the
-- declaration would be short by sixteen bits.
--
--   Nop        18'b00_0000000000000000
--   Read(a)    18'b01_pppppppp_oooooooo
--   Write(d)   18'b10_dddddddd_00000000
--   Halt       18'b11_0000000000000000
--
-- WHAT THE COMPILER WILL NOT LET YOU DO. Read a payload without going through
-- the tag: `match` is the only way in, and it binds the payload at the width
-- and type that variant declared. Compare one with `==`: that would take in
-- the padding, so `r == Nop` would mean "a Nop whose payload bits happen to be
-- zero", which is true of one this compiler built and says nothing about one
-- that arrived through a port. And write `Read` on its own, which would be a
-- read of an address nobody supplied.

struct addr_t
  page: i8
  off: i8

enum req_e
  Nop
  Read(addr_t)
  Write(i8)
  Halt

-- The payload is 16 bits wide because `addr_t` is the widest of them; `Write`
-- uses eight of those and the rest is padded with zeros rather than left
-- undefined. `x` propagates through a comparison in simulation and shows up as
-- a bug somewhere else entirely.
fun build_read (page: i8, off: i8, r: out req_e)
  r = Read(addr_t(page, off))

fun build_write (d: i8, r: out req_e)
  r = Write(d)

-- One `case` on the tag, and each arm reaching only for what its own variant
-- carries. `a` is 16 bits and `d` is 8, and neither exists on the other's arm.
fun serve (r: req_e, is_store: out i1, page: out i8, data: out i8)
  var store: i1 = 1'b0
  var p: i8 = @zeroed()
  var v: i8 = @zeroed()

  match r
    .Nop =>
      store = 1'b0
    .Read a =>
      p = a.page
    .Write d =>
      store = 1'b1
      v = d
    .Halt =>
      store = 1'b0

  is_store = store
  page = p
  data = v
