# ddl

A dataflow description language that compiles to Verilog-2005.

`desc.md` is the design: what the language is meant to become, including
pieces that do not exist yet. This file is the opposite — what the compiler in
this repository accepts today, and what it refuses.

```bash
cargo build
```

```bash
./target/debug/ddl build examples/mul3.ddl -o examples/mul3.v
```

The compiler needs nightly Rust; `rust-toolchain.toml` pins which one.

## The idea

Verilog makes you write the handshake. Every FIFO, every `valid`/`ready` pair,
every state register that holds a value across a wait — by hand, every time,
and each one is a chance to close a combinational loop through `ready` that
nothing catches until the design hangs on real silicon.

DDL takes the position that this is compiler work. You write what happens to
the data; the handshake, the state register and the pipeline registers are
generated. There is no global memory: a declaration touches its own state and
nothing else, and everything crossing a boundary goes through a pipe.

```
sequence mul3 (src: buffer in i16, dst: buffer out i32)
  let a = @rcv(src)
  let doubled: i16 = a + a
  |||
  let wide: i32 = @zext(doubled, 32)
  |||
  let scaled: i32 = wide + wide
  @send(dst, scaled)
```

`|||` is a stage cut. What comes out is a three-stage pipeline with a validity
bit per stage, back-pressure from the sink, and `dst_valid` driven by a
register — see [examples/mul3.v](examples/mul3.v). Nothing in the source
mentions `valid`, `ready`, or a clock.

A `graph` connects blocks like that one to each other, and a `process` turns a
sequential program with wait points into the state machine that implements it:

```
process reg_port (cmd: buffer in i8, din: buffer in i32, dout: buffer out i32)
  var cell: i32 = @zeroed()
  loop
    let c = @rcv(cmd)
    if c[0] then
      let d = @rcv(din)
      cell = d
    else
      @send(dout, cell)
```

Three states, and the branch is decided in the cycle the command arrives — see
[examples/reg_port.v](examples/reg_port.v).

## What compiles today

**Declarations.** `fun` (combinational, becomes a module with `out`
parameters as extra ports), `sequence` (becomes a pipeline, cut by `|||`),
`process` (becomes a state machine, cut at blocking channel operations), and
`graph` (structural composition: instantiates the others and wires their pipes
together). `struct` and `enum` lay out the same way SystemVerilog packs them,
so a DDL type and its `.svh` counterpart meet at a module boundary without a
cast.

**Types.** `iN` and `sN` at any width, structs, enums — including tagged
unions, where a variant carries a payload and `match` is the only way to reach
it — and arrays backed by `lutram` (asynchronous reads) or `bram` (synchronous:
the read costs a state, and is emitted inside the memory's own clocked block so
it infers as a block RAM rather than as distributed RAM with a flop on it).
A `[T; n]` without an `#[impl(...)]` is not storage but a packed value, so it
can be a struct field, a pipe payload or a parameter; `a[k]` selects an
element and `a[hi..lo]` a run of them, laid out the way SystemVerilog packs an
array, and an index past the end is an error rather than a bit somewhere else.
Widths are checked and never silently adjusted: mixed widths are an
error naming the `@zext`/`@trunc` that fixes them, and an unsized literal takes
its width from the other operand.

```
enum req_e
  Nop
  Read(addr_t)
  Write(i8)
  Halt
```

Laid out as `{tag, payload}` with the tag in the high bits, every variant the
same width. A tagged union's width is derived, not declared -- `enum e: iN`
sets the width of an enum whose variants carry nothing, and would name only the
tag here while reading as the whole value. See
[examples/tagged.ddl](examples/tagged.ddl).

**Statements.** `let`, `var`, assignment (plain and compound: `+=`, `<<=`,
`^=`, …), `if`/`else` — including with a blocking `@rcv`/`@send` inside, which
becomes a fork in the state machine — `match` with exhaustiveness checking,
`loop` and `break` in a process, `for i in 0..n` which unrolls, and calls to
`fun`, which are inlined.

**Parameters.** By value, `out` (write-only, which is how a function returns
more than one thing), and `inout` (by reference and readable: it updates the
caller's variable, and at a module boundary becomes `x` plus `x_out`).

**Pipes.** `buffer`, in and out, with `@rcv`, `@send`, `@try_rcv`,
`@try_send`, `@peek` and `@drop`.

**A pipe is claimed where the program asks for it, under the condition it
asks.** One rule, and it is the same in a process that blocks and one that does
not. A `@try_rcv` or `@drop` written inside an `if` takes `ready` down on every
other branch, so a stage that is busy finishing something declines its input by
saying so where it is busy — there is no separate way to stall. An offer is
made on the branch its `@try_send` is written on and no other, so a cycle that
produces nothing publishes nothing, and a cycle that owes a result is free to
produce one whether or not anything arrived.

A pipe gets one transfer per cycle, and `@peek` is how you look without
spending it: `let (v, present) = @peek(p)` reads the offer and takes nothing,
so a `@try_rcv` or a `@drop` of the same pipe in the same cycle is still
available. `present` is the offer where `got` is the transfer, and on a cycle
the process is not accepting they differ — which is the whole reason to peek.
`@drop(p)` is a `@try_rcv` that binds nothing, for when the answer is
"whatever that was, not this". There is one kind and it never drops anything: the producer waits.
A `buffer` is two entries deep — a head and a skid — which is what makes
`ready` a register rather than a wire through to the sink's: the producer
learns about a stall a cycle late and the second entry is where the item it
had already committed to goes. A chain of blocks is therefore a chain of
registers, not one combinational path as long as the chain. The depth cannot
be changed.

There was a second kind, `stream`, whose producer never waited because the
oldest item was overwritten instead. It is gone. Overwriting is a dropped
transfer, and a dropped transfer is not visible where it happens — it surfaces
later and somewhere else as a machine one item out of step. Where a sink
genuinely cannot refuse, the sink ties `ready` high and says so at the
boundary, which puts the claim somewhere a reader can check.

**Multiple files.** Either several inputs on the command line, or one input
naming the others:

```
import "k2g_types.ddl"
```

An import is resolved against the importing file's directory, then against
each `-I` directory. There are no namespaces — an import means "this file is
part of the program too", and every declaration is visible to every other one
regardless of which file it is in. A file reached twice is included once, so a
diamond is fine and so is a cycle.

## What it refuses, and says so

Deliberately, with a diagnostic rather than a wrong answer:

- assigning an output on only one branch of an `if`, which would be a latch
- a `match` that does not cover every variant, for the same reason
- mixing widths, or mixing signedness, in one operator
- a pipe in a `graph` with two producers, or with none
- a `bram` read inside an expression, or in a process with no states to spend
  a cycle in; and a `bram` with a reset, which cannot be inferred as one
- a `for` whose trip count is not known at compile time
- a blocking `@rcv`/`@send` inside a `match` (an `if` works)
- `==` on an enum that carries payloads, which would compare the padding too
- a width annotation on an enum that carries payloads, which would name the tag
  and read as the value
- reading a variant's payload without matching on its tag first
- `stream` pipes, which the language had and no longer does
- `bkram` memories, `~=`

`desc.md` lists more that is designed but not built — `io process`, `pin`,
`clock`.

## The backend

Verilog-2005, and specifically the subset GowinSynthesis survives. No
`$clog2`, no width casts in expressions, no function calls: all three make it
exit with an empty log. There are tests asserting none of them can appear in
the output. Generated files carry a banner with the exact command that
reproduces them, and `ddl build --check` verifies a checked-in file is current
without writing it.

## Tests

```bash
cargo test
```

```bash
cargo clippy --all-targets
```

The suite runs under Miri, which checks the parser's pointer arithmetic for
undefined behaviour rather than merely for crashing -- a read one past the end
usually does not crash:

```bash
MIRIFLAGS=-Zmiri-disable-isolation cargo miri test --test fuzz
```

Tests that touch the filesystem are skipped there; Miri has no Windows path
shims.

The suite includes a fuzzer, seeded so a failure is reproducible. It runs a
short pass on every `cargo test`; the soak is an environment variable:

```bash
DDL_FUZZ_ITERS=1000000 cargo test --release --test fuzz
```

Clippy is clean. The two lints this codebase deliberately does not follow are
in `Cargo.toml` with a reason each; rustfmt is deliberately NOT used, because
the house style predates it and adopting it would rewrite every file.

Unit and integration tests: the compiler's own behaviour, `import` against
real files on disk, that every example still compiles, and that the checked-in
`.v` files match what the compiler produces now.

Equivalence and area against the hand-written SystemVerilog these examples
replace is a separate script, because it needs Questa and the Gowin toolchain:

```bash
bash examples/verify.sh
```

It instantiates each generated module beside its reference, drives both with
the same stimulus, and compares primitive counts after synthesis. Modules
whose reference lives in another repository are skipped when it is absent.

## Formatting

```bash
ddl fmt examples/*.ddl          # in place
ddl fmt --check examples/*.ddl  # exit 1 if any needs it
```

Whitespace hygiene only: trailing space, line endings, runs of blank lines, the
final newline. **It does not touch indentation.** A DDL block is delimited by
indentation, and only the parser knows which deeper lines are blocks -- it
discards that as trivia, so nothing downstream can tell a nested block from the
continuation line of a wrapped parameter list. Every file it writes is parsed
before and after and refused if the syntax tree changed.

## Reading a design

```bash
ddl build examples/pipeline_graph.ddl --emit=dot | dot -Tsvg > scaler.svg
```

A `graph` is the one declaration whose meaning is its shape, and neither the
source nor the Verilog shows it: one lists instances and leaves the reader to
match pipe names, the other lists them again with three wires per pipe in
between. `--emit=dot` draws one edge per pipe, with the graph's own parameters
as the boundary, so a feedback path looks like one.

## Editor support

`editors/vscode/` is a syntax-highlighting extension for `.ddl`. To use it
without packaging, symlink or copy it into your extensions directory:

```bash
cp -r editors/vscode ~/.vscode/extensions/ddl
```

The grammar is checked against the compiler by `tests/editor.rs`: a builtin or
keyword the compiler resolves and the grammar does not is a test failure. A
syntax file is otherwise the one part of a toolchain nothing verifies, so it
goes stale in silence.

## Layout

| | |
|---|---|
| `src/lex.rs`, `src/parse.rs` | tokens and the AST, then precedence resolution |
| `src/source.rs` | which files a compilation is made of |
| `src/symbols.rs`, `src/ty.rs` | the symbol table and the type rules |
| `src/ir.rs` | typed SSA lowering, shared by every declaration kind |
| `src/ir_pipe.rs` | `sequence` → pipeline |
| `src/ir_fsm.rs` | `process` → state machine (a graph, not a chain) |
| `src/ir_graph.rs` | `graph` → instances and wires |
| `src/ir_match.rs` | `match` → case |
| `src/verilog.rs` | the backend |
| `src/diag.rs` | source locations and the caret rendering |
| `src/driver.rs` | the passes, in order |

`src/main.rs` is argument parsing and exit codes over the library in
`src/lib.rs`; anything that wants to compile DDL without spawning a process
links the library.
