-- A full unused output does not prevent explicitly requested input work.
process polling_drain (src: buffer in u8, blocked: buffer out u8, observed: buffer out u1)
  loop
    let took = @drop(src)
    @try_send(observed, took)

-- Safe forwarding: the head is consumed only after its send succeeds.
process polling_forward (src: buffer in u8, o: buffer out u8, unrelated: buffer out u8)
  loop
    let (x, present) = @peek(src)
    if present then
      let sent = @try_send(o, x)
      if sent then
        let took = @drop(src)
        @assert(took)

process polling_once (src: buffer in u8, o: buffer out u8)
  let (x, got) = @try_rcv(src)
  @assert(got)
  if got then
    let sent = @try_send(o, x)
    @assert(sent)

process polling_peek (src: buffer in u8, observed: buffer out u1)
  loop
    let (x, present) = @peek(src)
    @try_send(observed, present)
