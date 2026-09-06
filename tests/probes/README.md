# Compiler regression inputs

The p-numbered DDL sources are preserved copies of the K2G `probe/` inputs.
Their comments describe the original failures, before the compiler repairs.
`lifetimes.ddl` adds repeated dynamic initialization, shadowing and mutable
receive bindings; `arm_scopes.ddl` checks different payload types and mutable
locals under the same source names in separate waiting match arms.

`cargo test --test probe_regressions` executes the lowered circuits and checks
observable transfers and values under backpressure, including negative
assertion cases and scope/type diagnostics. It requires no external simulator.

On Windows, `./tests/run_probe_regressions.ps1 -SimTool <Questa-bin-directory>`
also compiles every input's emitted `.v` in Verilog mode and runs `tb_fixed.sv`.
Outputs and transcripts go to `target/probe-regressions/`. The runner requires
the pass marker and zero simulator errors; a successful process exit alone
is insufficient.

`p3` is a successful memory feature probe. `p4` compiles, but its original
`consumed == (stale | fire)` assertion assumes output availability; it is not
a valid invariant under arbitrary backpressure and is not used as a compiler
correctness oracle. The positive simulator test deliberately exercises the
compiler defects and lifetimes, not that unsupported source assumption.
