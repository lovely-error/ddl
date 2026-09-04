-- A register port: one command pipe, and what arrives decides what happens
-- next.
--
-- This is the shape a blocking `process` could not express until the state
-- machine became a graph. The body reads as a sequential program -- receive a
-- command, then either receive data or send it back -- and the machine that
-- comes out has a fork in it, because the program does.
--
-- WHAT THE BRANCH COSTS: nothing. `is_write` is decoded from `cmd_data` in the
-- cycle the command's handshake completes, so the next state is already
-- decided when the state register clocks. A scheduler that put the decode in
-- its own state would spend a cycle on every command, which for a port that
-- exists to be fast is the whole point missed.
--
-- WHAT EACH STATE ACCEPTS: only what it is waiting for. `cmd_ready` is high in
-- the command state and nowhere else, `din_ready` only on the write path,
-- `dout_valid` only on the read path. A process that accepted an item it was
-- not ready to handle would drop it, and a dropped transfer shows up much
-- later and somewhere else.
--
-- Channel rule 3 (k3g_chan.sv:22) still holds and still holds by
-- construction: `dout_valid` is `state == 2`, and state is a register, so
-- `valid` cannot depend combinationally on `ready` however the branches run.

process reg_port (cmd: buffer in u8, din: buffer in u32, dout: buffer out u32)
  -- The register being read and written. A process reaches its own state and
  -- nothing else (desc.md:26), so this cell is genuinely private -- the only
  -- way to its contents is through the pipes.
  var cell: u32 = @zeroed()

  loop
    let c = @rcv(cmd)

    -- Decoded here rather than in the arms, so both paths share it and it
    -- costs one bit of logic instead of two.
    let is_write: u1 = c[0]

    if is_write then
      let d = @rcv(din)
      cell = d
    else
      @send(dout, cell)
