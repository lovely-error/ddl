-- Blocking channel operations, scheduled into a state machine.
--
-- `@rcv` and `@send` stall until the transfer happens, so this reads as a
-- sequential program and the wait points become the states. Three of them
-- here: receive, receive, send.
--
-- Nothing in the source mentions valid, ready or a state register. What the
-- compiler generates is what k3g_fetch_seq.sv and k2g_cache_maint.sv write by
-- hand -- a state register, per-state handshake, and a register for each value
-- that has to survive a clock edge (`a` is received in state 0 and used in
-- state 2, so it cannot be a wire).
--
-- Channel rule 3 still holds: in the send state `valid` is `state == 2`, and
-- state is a register, so `valid` never depends combinationally on `ready`.

process fsm_adder (src: buffer in u32, dst: buffer out u32)
  loop
    let a = @rcv(src)
    let b = @rcv(src)
    @send(dst, a + b)
