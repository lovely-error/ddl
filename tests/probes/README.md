# Compiler regression inputs

The p-numbered DDL sources are preserved copies of the K2G `probe/` inputs.
Their comments describe the original failures, before the compiler repairs.
`lifetimes.ddl` adds repeated dynamic initialization, shadowing and mutable
receive bindings; `arm_scopes.ddl` checks different payload types and mutable
locals under the same source names in separate waiting match arms.
`backend_edges.ddl` checks scalar selection/extension and identifier
collisions, including a graph connecting colliding module names.
`adversarial.ddl` checks shared continuations, for-only crossing reads, BRAM
forwarding, and exclusive sends in blocking states. Its separate
`tb_adversarial.sv` is also run by the strict Questa runner.
`communication.ddl` checks local channel availability, observation without
consumption, one-shot execution, and forwarding under backpressure.

`cargo test --test probe_regressions` executes the lowered circuits and checks
observable transfers and values under backpressure, including negative
assertion cases and scope/type diagnostics. It requires no external simulator.

On Windows, `./tests/run_probe_regressions.ps1 -SimTool <Questa-bin-directory>`
also compiles every input's emitted `.v` in Verilog mode and runs `tb_fixed.sv`.
Outputs and transcripts go to `target/probe-regressions/`. The runner requires
the pass marker and zero simulator errors; a successful process exit alone
is insufficient.

`p3` is a successful memory feature probe. With local channel availability,
`p4`'s `consumed == (stale | fire)` assertion holds even with full outputs:
an attempted drop needs only the observed input. The regressions check this
case. Its original forwarding algorithm can still discard fresh data when a
send fails; lossless forwarding must retain the input or store the pending
output until a send succeeds.
