# Assessment of the sequence-lowering report

Assessed 2026-09-07 against compiler commit `85f0537`. The verdicts below
describe that baseline. Reproducers, emitted Verilog, compiler logs,
and Questa 2025.2 transcripts are in `target/sequence-assessment/`.

## Repair status

The six confirmed cases (#1, #3, #4, #5, #6, #7) are repaired in the working tree:

| Case | Compiler correction | Regression evidence |
|---|---|---|
| #1 | Gate assertions, including assertions in an inlined tail-send expression, with stage validity and pipeline advance. Preserve branch guards. | IR execution tests cover all three stages, bubbles, stalls, failing transactions, `@assert`, `@fatal`, and branch guards. Questa checks empty stages and real transfers. |
| #3 | Reject duplicate head receives and tail sends before replacing their descriptors. | Direct compiler API tests reject both cases. Existing misplaced-stage diagnostics remain intact. |
| #4 | Reject unsupported nonblocking **buffer** operations with source diagnostics before accessing process-only transfer metadata. | Direct API tests cover `@peek`, `@try_rcv`, `@drop`, and `@try_send`; a panic fails the test. Nonblocking `port` operations still compile and execute. |
| #5 | Resolve lexical binding identities before splitting the sequence into stages. Cuts do not introduce scopes. | Both same-type and different-type branch shadows leave the outer value at 10. Questa observes 10, 10 for input values 6, 2. |
| #6 | Resolve the BRAM address expression before introducing the new result binding; crossing/dependency analysis uses those distinct identities. | Same-name BRAM reads compile and return 123, 456 in Questa. Using the read result without its required cut remains rejected. |
| #7 | Lower tail-send expressions with the output payload type, including fitting constant coercion. | Unsized 0 and 42 and contextual `@zeroed()` execute correctly; an overflowing constant remains rejected. |

Case #4 is a diagnostic fix, not an extension of the sequence communication
model. Sequences still use one blocking head receive and one blocking tail
send. The README states that restriction explicitly.

Case #2 remains a target-dependent synthesis concern. The regression suite
preserves source-order read-before-write behavior: two same-address requests
return 7 and then 42. No physical FPGA or synthesized memory primitive was
validated in this repair.

Reproducible checks:

- `cargo test --locked`: full suite passed (553 reported passes before the
  final two added regression tests; the final eight sequence tests also passed).
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `tests/run_probe_regressions.ps1`: all three Questa benches passed, including
  the new `tb_sequence`; emitted `.v` files are compiled as Verilog.
- Verilator lint is unavailable on this machine. Its optional Rust harness
  reports a pass after printing `SKIP`; this is not counted as RTL validation.
- K2G generated outputs were regenerated; `build.sh --check` passes for every
  implemented target (`k2g_cycles` remains unwritten and is explicitly skipped).
- K2G Questa RAM comparison: 4,213 responses, zero mismatches.
- K2G architectural comparisons against the emulator pass for `fib`,
  `fault_port`, `hello`, `hazard`, and `selfmod`.

Regression sources are `tests/probe_regressions.rs`,
`tests/probes/sequences.ddl`, and `tests/probes/tb_sequence.sv`. Questa outputs
are generated under `target/probe-regressions/`.

## Verdicts

| # | Verdict | Measured evidence |
|---|---|---|
| 1 | Confirmed simulation defect | The supplied assertion fires four times at 35, 45, 55, and 65 ns with no input item. Both channel salts remain zero. |
| 2 | Target-dependent implementation risk; not an established RTL miscompilation | With memory location 0 initialized to 7 by the testbench, two same-address transactions read 7 then 42 while writing 42. The emitted RTL preserves read-before-write order. No synthesized primitive or physical device was tested. |
| 3 | Confirmed silent elimination | The supplied sequence compiles. Two offered input items produce two outputs of 20; the first send of 10 is absent. Source inspection confirms the earlier receive descriptor is also overwritten. |
| 4 | Confirmed internal compiler errors | All four buffer operations reproduce a panic. The CLI catches the compilation-thread panic, reports a compiler bug, and exits 1. This is not an uncontrolled termination of the whole host application. |
| 5 | Confirmed scope violation | Inputs 6 and 2 produce 20 and 10. The outer immutable `x` should remain 10 for both. Changing the shadow to `u32` produces the claimed spurious type conflict. |
| 6 | Confirmed name-analysis defect after correcting the reproducer | A correctly declared BRAM and explicitly four-bit address still produce the same-name read-use error. Giving the read result a distinct name compiles. |
| 7 | Confirmed constant-typing inconsistency | `@send(dst, 0)` is rejected with `dst carries u16 but u1 was sent`. The report's `u0` detail is incorrect. |

## What needs correction in the report

### Assertions

The concrete example checks `src_item` in stage 0, not a downstream pipeline
register. Zeroed empty input data is enough to reproduce the bug; it does not
depend on uninitialized registers. The emitted assertion has only a reset
guard and its source predicate, with no transaction-valid qualification.

Stage validity is mandatory. Whether a valid but stalled stage should check
an assertion every cycle is a separate execution-policy choice. Consistency
with execution-triggered process assertions suggests checking on
`stage_valid && shift`, while preserving the existing branch-path guard.
Tests must distinguish empty bubbles, valid stalled items, and actual advances.

### Read before write is not missing write-forwarding

For `let old = mem[a]; mem[a] = new`, forwarding `new` into `old` would itself
be a miscompilation. The intended result is the previous stored value. Two
nonblocking assignments in the emitted clocked block provide precisely that
behavior in Verilog simulation, as the directed test confirms.

Hardware support depends on the selected primitive, clock relationship, and
read-during-write configuration. AMD documents reliable old-data reads for
common-clock read/write collisions when the write port uses READ_FIRST, and
recommends that configuration for synchronous simple dual-port RAM:
[AMD PG326, Collision Behavior](https://docs.amd.com/r/en-US/pg326-embedded-memory-generator/Collision-Behavior).
Thus the report's blanket assertion about Xilinx simple dual-port RAM is not
valid. Gowin UG285 documents restrictions, including removal of dual-port
read-before-write support:
[Gowin BSRAM & SSRAM User Guide](https://cdn.gowinsemi.com.cn/UG285E.pdf).

The next decisive hardware test is to synthesize this exact reproducer for a
named target, inspect the inferred primitive and mode parameters, and run its
vendor simulation model through the address collision. If that target cannot
provide old data, the backend must use a supported alternative or reject the
mapping. Merely forwarding the later write is not a repair. This assessment
does not establish which primitive Gowin synthesis would select.

### Duplicate transfers and nonblocking operations

`head_recv = Some(r)` and `tail_send = Some(s)` overwrite earlier descriptors
without checking occupancy. Diagnose duplicates at the second operation's
source position before removing either statement from the stage body.

`@peek` and `@try_rcv` both fail first at the missing input-item field in the
tested compiler; the report's distinct first panic sites are not exact.
`@drop` and `@try_send` fail at missing pipe eligibility.

The crash is a defect regardless of whether a particular operation is legal.
However, adding nonblocking operations to the same channels already used by
the canonical head receive and tail send can mean a duplicate transfer.
Initializing the missing fields alone would not establish correct sequence
semantics. The implementation must define stage ownership, item identity,
transfer budget, and success qualification, and reject unsupported/duplicate
operations with diagnostics. The design notes permit nonblocking reads, but
that does not prove every placement on every channel is legal.

### Lexical identities and BRAM result availability

The process ownership repairs do not automatically apply to sequence lowering.
Sequences still use spelling-based environments and stage dependency sets.
This explains both the branch-shadow leak and the same-name BRAM-result error.

The supplied BRAM declaration is not current DDL syntax. It must be:

```ddl
var mem: #[impl(bram)] [u16; 16]
```

Also, `a & 16'd15` is still sixteen bits wide. The compiler does not infer a
four-bit address type from the mask. A useful isolated reproducer is:

```ddl
sequence MemReadRename (src: buffer in u16, dst: buffer out u16)
  var mem: #[impl(bram)] [u16; 16]
  let a = @rcv(src)
  let addr = @trunc(a, 4)
  |||
  let addr = mem[addr]
  |||
  @send(dst, addr)
```

This still fails with the claimed same-stage-use diagnostic. Renaming the
second binding to `result` and sending `result` compiles. A repair should
resolve declaration identities before stage def/use analysis, resolve each
initializer in its pre-declaration scope, and check availability of the new
read-result identity rather than every occurrence of its spelling.

## Repair order

1. Reject duplicate sequence head/tail operations and turn unsupported buffer
   builtins into diagnostics, preventing silent deletion and internal panics.
2. Apply lexical binding identities across stage cuts, fixing shadowing and
   same-name BRAM dependency analysis together.
3. Qualify assertions with stage execution and test bubbles and backpressure.
4. Reuse destination-directed constant coercion for the tail send.
5. Close the target-specific BRAM inference contract with synthesis and vendor
   model tests before making a physical-hardware correctness claim.

## Evidence files

- `results.json`: compilation exit codes and diagnostics for the initial cases.
- `tb_assert.log`: four expected false assertion reports on an empty pipeline.
- `tb_assess.log`: duplicate outputs `00140014`, shadow outputs `000a0014`,
  and correct read-first RTL outputs `002a0007` (low entry first).
- `address_narrow.ddl`: corrected same-name reproducer.
- `address_fixed_control.ddl`: corrected distinct-name control, which compiles.

The simulation's `ASSESSMENT_COMPLETE` marker means the expected observations
were reproduced, including the bugs. It does not certify those designs as
correct. Assertion failures were intentionally collected rather than counted
as passing compiler tests. No compiler fixes or commits were made here.
