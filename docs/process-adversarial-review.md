# Process lowering: verified claims and repairs

Verified on 2026-09-07 using the compiler, its lowered-circuit interpreter,
Rust compile checks, and Questa 2025.2 simulation of emitted Verilog.
The working-tree changes from the preceding communication repair were
preserved. Baseline artifacts are in `target/adversarial-baseline/`.

## Findings

| Claim | Observed before repair | Resolution |
|---|---|---|
| A shared bare branch is destroyed by absorption | Confirmed: the supplied `cond1=0, cond2=1` case produced no `o2` transfer. | Copy the continuation's statements and transition when folding it into a predecessor. Leave the original available to other predecessors; remove it only through reachability pruning. Exclude synchronous-read states from bare-branch folding. |
| Process BRAM reads lack same-state write forwarding | Confirmed with a changing-value reproducer; the supplied receive/write/read example itself passed. | Capture pending-write hit/data alongside the synchronous read, then select the registered forwarding data or RAM result. Last preceding write wins, including conditional writes. |
| Nonblocking and blocking sends to the same output port collide silently | Confirmed: `@try_send(dst,1)` followed by `@send(dst,2)` compiled. | Reject a scheduled port send if the state already contains a nonblocking send to that port. Both source orders have regression tests. |
| Exclusive nonblocking sends are rejected | Confirmed for the supplied blocking-process example. | Track the control-flow path of each request. Opposite `if` arms and disjoint `match` arms share one transfer, with muxed data and combined enable. Each operation's returned success still refers to its own path. Overlapping requests remain errors. The same accounting covers receive/drop claims. |
| `anumspan_to_str` exposes an unconstrained lifetime | Confirmed by compiling the safe-caller exploit; no dangling read was executed on the baseline. | Identifier and string-literal spans own reference-counted text. Safe text access borrows the span, never its raw location pointer. Cloned/extracted AST nodes retain ownership. A compile-fail test rejects a caller-selected `'static` borrow. |
| Reads inside `for` bypass crossing registers | Confirmed: input entries 3 and 7 produced 28 instead of 12 after a wait on another channel. | Visit the `for` target and body during read analysis, allowing the existing crossing-register analysis to retain the original received value. |
| Scheduler inspection misses waits/control flow inside `for` | Confirmed: a wait-only `for` reached an unrelated combinational-lowering error. | Traverse `for` bodies in barrier/state inspection and reject stateful bodies explicitly. `for` remains a combinational unrolling construct; sequential `for` scheduling is not implemented by this repair. Use a process `loop` for waits and `break`. |
| A state-selected buffer payload mux without a request guard corrupts output | Not established. The alleged mux is an intermediate write payload, not the published buffer entries. | Retain the state mux. Entry writes and pointer advancement are gated by the actual transfer. Inactive-state payloads cannot update an entry. The exclusive-send RTL tests verify the resulting payload selection. Reading an absent item is outside the buffer contract. |

The two reports of exclusive-arm accounting describe the same defect.

## Small reproducers

The full supplied branch and crossing examples are preserved in
`tests/probes/adversarial.ddl`. The branch test sends no result before the
repair; afterward its second conditional executes on every predecessor.

The port collision is:

```ddl
process p (dst: port out u8)
  loop
    @try_send(dst, 8'd1)
    @send(dst, 8'd2)
```

This now reports that `dst` is sent to more than once in one cycle.

The exclusive send is valid:

```ddl
process p (c: buffer in u1, dst: buffer out u8)
  loop
    let flag = @rcv(c)
    if flag then
      @try_send(dst, 8'd1)
    else
      @try_send(dst, 8'd2)
```

It attempts exactly one send. It does not retry a failed send automatically;
that is the ordinary nonblocking contract.

For BRAM, the original `@rcv; write; read` example does not force the write and
read into the same state: the write can run in the receive state's post work.
The following case does expose the missing forwarding:

```ddl
process conditional_forward (o: buffer out u8)
  var tbl: #[impl(bram)] [u8; 16]
  var choose: u1 = 1'b0
  loop
    tbl[0] = 8'd10
    if choose then
      tbl[0] = 8'd20
    let value = tbl[0]
    @send(o, value)
    choose = !choose
```

The first two items must be 10, 20. The saved baseline compiler emits 10, 10;
Questa reports `expected=140a actual=0a0a` for the two packed entries.
The repaired compiler emits 10, 20. A constant-only write test is insufficient:
the original memory write enable can be active during reset, priming the array
with the same constant and concealing the read-during-write difference.

## Ownership boundary

`AlphanumSpan` is no longer `Copy`. Use `clone()` to retain a span and
`AlphanumSpan::new(text)` to construct one in Rust. Its raw pointer is retained
as source-location metadata; safe text access and `Debug` use owned text.
`StrSpan::as_str()` follows the same ownership rule. The raw lexer still uses
unsafe pointer scanning while a source buffer is alive; this repair does not
claim to eliminate all unsafe Rust in the parser.

An extracted `Parsed.decls` can now safely outlive its source map because its
text is owned. Keeping that operation legal is deliberate. The old wrapper's
`PhantomData` was insufficient to make borrowed raw fields safe once extracted.

## Reproducible validation

```powershell
cargo test --locked
cargo clippy --all-targets -- -D warnings
./tests/run_probe_regressions.ps1
```

`tests/probe_regressions.rs` covers both branch outcomes, branch and switch
continuations, crossing storage, exclusive send data/success under full and
available outputs, and negative duplicate/stateful-for cases.
`tests/ast_ownership.rs` executes safe text access after dropping the source
and after cloning/extracting AST nodes. The `anumspan_to_str` doctest checks
that an unbounded returned borrow is rejected by Rust.

The strict Questa runner executes both `tb_fixed` and `tb_adversarial`, checks
pass markers, and rejects simulator errors. Logs are in
`target/probe-regressions/`. The optional Verilator test still skips when that
tool is unavailable; a Rust harness pass for that test is not Verilator
validation. No FPGA synthesis, timing, or physical-device validation is claimed.
