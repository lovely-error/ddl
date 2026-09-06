-- Probe: bram with two writes in one stage of a sequence (the --lvt-bram case).
struct op_t
  fill_we: u1
  fill_ix: u6
  fill_tag: u10
  snoop_we: u1
  snoop_ix: u6
  look_ix: u6

sequence p3 (op: buffer in op_t, ans: buffer out u10)
  var tags: #[impl(bram)] [u10; 64]
  let q = @rcv(op)
  if q.fill_we then
    tags[q.fill_ix] = q.fill_tag
  if q.snoop_we then
    tags[q.snoop_ix] = 10'h3FF
  let t = tags[q.look_ix]
  |||
  @send(ans, t)
