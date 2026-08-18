-- A three-stage pipeline. `|||` is the stage cut.
--
-- Everything inside a stage is combinational; every cut becomes a register
-- bank with a validity bit riding alongside the data -- desc.md:51's "implicit
-- is_valid condition at each stage". The whole pipeline shifts together when
-- the sink has a slot (desc.md:48).
--
-- `|||` has been parsed and thrown away since before this work started; this
-- is the first time it means anything.
--
-- Latency is three cycles, throughput one item per cycle while the sink keeps
-- up. Channel rule 3 holds as everywhere else: `dst_valid` is the last
-- validity bit, which is a register.

sequence mul3 (src: buffer in i16, dst: buffer out i32)
  let a = @rcv(src)
  let doubled: i16 = a + a
  |||
  let wide: i32 = @zext(doubled, 32)
  |||
  let scaled: i32 = wide + wide
  @send(dst, scaled)
