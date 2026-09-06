-- Probe: an `@assert` inside a `match` arm of a BLOCKING process.
--
-- The arm is a state, and the assertion is emitted with no state guard at all:
--
--   if (rst_n) begin
--     if (!((a_r != 8'hFF))) $error("%m: a read carried the poison value");
--   end
--
-- `a_r` is latched by `fire_s0` whichever arm was taken, so the check fires
-- every cycle from reset onwards, on stale pipe contents and on `.Wr` items.
enum m8_e
  Rd8(u8)
  Wr8(u8)

process p8 (q: buffer in m8_e, o: buffer out u8, p: buffer out u8)
  loop
    let r = @rcv(q)
    match r
      .Rd8 a =>
        @assert(a != 8'hFF, "a read carried the poison value")
        @send(o, a)
      .Wr8 b =>
        @send(p, b)
