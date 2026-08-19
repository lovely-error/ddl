-- A block RAM, and the cycle its read costs.
--
-- `lutram` and `bram` are not two names for storage. They are two primitives
-- with different timing, and the annotation picks which one the synthesizer
-- infers:
--
--   lutram   one synchronous write port, ASYNCHRONOUS reads. The value is
--            there in the same cycle as the address. Cheap for small arrays,
--            expensive per bit, and it is what k2g_xstage's register file
--            needs -- read, forward and address-add all in one cycle.
--
--   bram     one synchronous write port, SYNCHRONOUS reads. The value arrives
--            a cycle after the address. Dense, and the only thing that holds
--            a table of any size.
--
-- WHERE THE CYCLE GOES has to be somewhere the source can point at, and the
-- only place in this language that holds one is a state of a blocking
-- `process`. So a `bram` read is a statement of its own -- `let v = t[a]` --
-- and it costs a state. Three states below, and `--emit=ir` will show them:
-- receive the address, fetch, send. A read written inside an expression is
-- refused, because there is no way to say that the rest of the expression
-- waits.
--
-- WHAT THE BACKEND HAS TO EMIT for this to be a block RAM at all is the read
-- INSIDE the memory's own clocked block:
--
--     always @(posedge clk) begin
--       if (in_s1) t_q <= t[a_r];
--     end
--
-- and not a `wire q = t[a];` with the flop somewhere else. That second shape
-- is a combinational array read plus a register, and a synthesizer infers
-- distributed RAM from it -- exactly what `lutram` already gives you, with a
-- wasted flop on top and a `bram` label on the front. This file exists so that
-- examples/verify.sh counts the RAM primitives and says which one it got.
--
-- NO RESET. A block RAM written on reset cannot be inferred as one: the reset
-- loop is a write to every element, and 256x32 comes out as 8192 flip-flops.
-- The compiler refuses the initialiser rather than emitting that.

process bram_lookup (req: buffer in i8, resp: buffer out i32)
  var t: #[impl(bram)] [i32; 256]

  loop
    let a = @rcv(req)

    -- The address is registered across the state boundary on its way here:
    -- the fetch happens a cycle after the receive, and `req_data` belongs to
    -- whatever the source is offering now, not to the request that was
    -- accepted.
    let v = t[a]

    @send(resp, v)
