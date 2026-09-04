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
sequence mul3 (src: buffer in u16, dst: buffer out u32)
  let a = @rcv(src)
  let doubled: u16 = a + a
  |||
  let wide: u32 = @zext(doubled, 32)
  |||
  let scaled: u32 = wide + wide
  @send(dst, scaled)
```

`|||` is a stage cut. What comes out is a three-stage pipeline with a validity
bit per stage, back-pressure from the sink, and a `dst_wsalt` driven by a
register — see [examples/mul3.v](examples/mul3.v). Nothing in the source
mentions a handshake or a clock.

A `graph` connects blocks like that one to each other, and a `process` turns a
sequential program with wait points into the state machine that implements it:

```
process reg_port (cmd: buffer in u8, din: buffer in u32, dout: buffer out u32)
  var cell: u32 = @zeroed()
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
`process` (becomes a state machine, cut at blocking channel operations),
`graph` (structural composition: instantiates the others and wires their pipes
together), and `extern` (names a module DDL did not compile, so a graph can
instantiate it). `struct` and `enum` lay out the same way SystemVerilog packs them,
so a DDL type and its `.svh` counterpart meet at a module boundary without a
cast.

**Types.** `uN` and `iN` at any width, structs, enums — including tagged
unions, where a variant carries a payload and `match` is the only way to reach
it — and arrays backed by `lutram` (asynchronous reads) or `bram` (synchronous:
the read costs a cycle, and is emitted inside the memory's own clocked block so
it infers as a block RAM rather than as distributed RAM with a flop on it).
That cycle is a state in a `process` and a `|||` cut in a `sequence`, where the
memory's own output register is the pipeline register for that boundary — so
`let v = t[a]` before a cut costs no flop of ours, and `v` means the value from
the stage below on. A read sees the writes above it in source order, forwarded
around the array, which updates only on the edge. A memory a stage **writes**
is read and written in one stage — every item then sees every write of every
item before it, plus its own — while a memory nothing writes has no order to
keep and may be read anywhere.

**Ports follow the source.** One write port per write that can happen in a
cycle: two writes in a row are two ports, the arms of an `if` share one because
they cannot both happen, and several states of a process share one because only
one state is current. Reads are the same — as many as a pipeline stage asks
for, muxed onto one in a process. An FPGA infers no RAM at all from a second
write port while an ASIC memory compiler emits the cell, so the count is the
source's to state and what it costs is the target's to answer: `ddl build
--lvt-bram` builds a multi-write `bram` out of one-write blocks and a live value
table instead of asking for a cell that an FPGA does not have. On a GW1NR-9C
that is the difference between not fitting and four block RAMs —
[examples/rf_lvt.ddl](examples/rf_lvt.ddl) carries the numbers, and
`examples/verify.sh` re-measures them.
A `[T; n]` without an `#[impl(...)]` is not storage but a packed value, so it
can be a struct field, a pipe payload or a parameter; `a[k]` selects an
element and `a[hi..lo]` a run of them, laid out the way SystemVerilog packs an
array, and an index past the end is an error rather than a bit somewhere else.
`a[k] = v` and `s.words[k] = v` assign one, at a constant index or a computed
one; `@slice(x, base, w)` is the read side of the same idea on a plain `uN`,
a `w`-bit window at a base this cycle decides. Going the other way, `{a, b, c}`
joins values high-to-low and is spelled the way Verilog spells it — the result
is `uN` of the summed widths, because a bit with something below it has no sign
left to keep.
Widths are checked and never silently adjusted: mixed widths are an
error naming the `@zext`/`@trunc` that fixes them, and an unsized literal takes
its width from the other operand.

```
enum req_e
  Nop
  Read(addr_t)
  Write(u8)
  Halt
```

Laid out as `{tag, payload}` with the tag in the high bits, every variant the
same width. A tagged union's width is derived, not declared -- `enum e: uN`
sets the width of an enum whose variants carry nothing, and would name only the
tag here while reading as the whole value. See
[examples/tagged.ddl](examples/tagged.ddl).

**Statements.** `let`, `var`, assignment (plain and compound: `+=`, `<<=`,
`^=`, …), `if`/`else`, `match` with exhaustiveness checking, `for i in 0..n`
which unrolls, and calls to `fun`, which are inlined.

**Anything that waits can go in an `if`, a `match` or a `loop`.** A blocking
`@rcv`/`@send`, a `break` and a `bram` read all cost a cycle, and
all three constructs can hold one: an `if` forks the state machine, a `match`
becomes a `case` on the tag choosing the next state, and a `loop` is a back
edge. `loop`s nest to any depth and `break` leaves the innermost one; at the
top of a process it stops the process (desc.md:37).

A `match` mattered here more than it looks. `match` is the only way to reach a
payload and `==` on an enum that carries one is refused, so before this a
tagged union had no way to wait per variant at all — which is the shape of
every dispatch. A name bound by more than one arm is one wire and therefore
one type; two payload types under one name is refused rather than picked
between.

**Parameters.** By value, `out` (write-only, which is how a function returns
more than one thing), and `inout` (by reference and readable: it updates the
caller's variable, and at a module boundary becomes `x` plus `x_out`).

**Pipes.** Two kinds, reached the same way. A `buffer` has back-pressure: two
entries, and a producer that waits. A **`port`** has none at all — a data
port and an enable, and nothing coming back:

```
process relay (src: port in u32, dst: port out u32)
  loop
    let v = @rcv(src)
    @send(dst, v + 32'd1)
```

`port in x: T` is `x` and `x_en` coming in; `port out y: T` is `y` and `y_en`
going out. Both are read and written with the channel operations and not by
name: `@rcv` to wait for one, `@try_rcv` or `@peek` to look at what is there
this cycle, `@send` and `@try_send` to put one out. **A port is not a value.**
Naming one where a value belongs is an error that says which operation to
reach for instead — binding the name to the wire would make it the one
channel in the language you read by naming it, and would lose the distinction
between "the value" and "a value that means something this cycle" that
`@try_rcv`'s pair carries.

The operations mean what they mean on a `buffer`, with only the handshake
underneath differing. `@rcv` waits on the enable rather than on two salts
disagreeing. `@send` never waits at all, because there is no `ready` coming
back to wait for, so its state costs its cycle and no more. `@try_rcv` answers
with the enable itself: on a pipe the question is "did I take one", which
needs this side to have accepted, and a port claims nothing anywhere.
`@try_send` always succeeds. `@drop` is refused — it spends a pipe's one
transfer for the cycle so the next item can arrive, and a port is not holding
one back.

`port` is deliberately not the bare `T` spelling, which already means
something else and quietly — a plain parameter is configuration, folded at
compile time and gone.

**A process of nothing but ports is a plain Verilog module**, which is the
point of it: a register, a clock and the ports the source asked for, with no
salt and no entries anywhere. That is how the ordinary sequential blocks a
design needs get written beside the dataflow ones.

In a `sequence` a `port out` belongs to the STAGE that sent to it. A process
drives one from the state that sent, gated on that state firing; a pipeline
has no states, so what stands in for it is "this stage has a valid item and
the pipeline is moving". Sending in two stages is refused — every stage is
live at once holding a different item, so that would be two answers for one
wire, and unlike a process there is nothing to choose between them.

A `port` is for the EDGE of the program, where the thing on the other side
cannot be made to wait: a pin, a PLL, a bus master that does not take `ready`
for an answer. That is the case the rule below already names — the sink ties
`ready` high and says so at the boundary — and `port` is how it is said.
Between two things DDL compiled, a `buffer` is still the answer.

**Buffer pipes.** `buffer`, in and out, with `@rcv`, `@send`, `@try_rcv`,
`@try_send`, `@peek` and `@drop`.

**Assertions.** `@assert(cond)`, `@assert(cond, "message")` and `@fatal`, which
is the same with `$fatal` instead of `$error`. They are checked in simulation
and absent from synthesis: the emitted block sits inside `` `ifdef SIMULATION ``,
which is the guard this toolchain needs because GowinSynthesis does not define
`SYNTHESIS`. In a clocked module the checks run on the edge and are held off
during reset. A condition written inside an `if` is already guarded by the path
it sits on, so it reads as an implication and is vacuously true elsewhere.

**A pipe is claimed where the program asks for it, under the condition it
asks.** One rule, and it is the same in a process that blocks and one that does
not. A `@try_rcv` or `@drop` written inside an `if` takes `ready` down on every
other branch, so a stage that is busy finishing something declines its input by
saying so where it is busy — there is no separate way to stall. An offer is
made on the branch its `@try_send` is written on and no other, so a cycle that
produces nothing publishes nothing, and a cycle that owes a result is free to
produce one whether or not anything arrived.

**What is on the wire is a pair of gray-coded pointers, not `valid`/`ready`.**
Each pipe flattens to three ports: `<p>_wsalt` (2 bits, the producer's),
`<p>_rsalt` (2 bits, the consumer's) and `<p>_data`, which carries both
entries. Each side publishes only its own salt, and publishes it from a
register:

```verilog
wire src_empty = src_wsalt == src_rsalt_q;      // nothing to take
wire dst_full  = dst_wsalt_q == (~dst_rsalt);   // one lap ahead: both full
```

Empty is the two salts agreeing; full is their differing in both bits, which
in gray code is one lap over a buffer that holds two. Toggling your own salt
IS the transfer — there is no separate `valid` to assert and no `ready` to
answer it in the same cycle, so "`valid` must not depend combinationally on
`ready`" holds by construction rather than by review. A hand-written module
meeting a generated one matches those three ports; `extern` (below) is how to
say so without wiring them by hand.

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

There is no lossy kind, and no way to ask for one. A pipe whose producer never
waited would have to overwrite its oldest item when the sink fell behind, and
an overwrite is a dropped transfer — not visible where it happens, but later
and somewhere else, as a machine one item out of step. Where a sink genuinely
cannot refuse, it ties `ready` high and says so at the boundary, which puts the
claim somewhere a reader can check.

**Combinators.** `@merge` and `@split` are modules the compiler writes:

```
graph top (p: buffer in u32, q: buffer in u32, o1: buffer out u32, o2: buffer out u32)
  let m: buffer u32
  let d: buffer u32
  @merge(p, q, m)      -- two producers onto one pipe, in rotation
  dbl(m, d)
  @split(d, o1, o2)    -- one producer to two consumers, each its own copy
```

`@merge` grants one input per cycle with a rotating priority, so a busy input 0
cannot starve input 1. `@split` takes from its input only when EVERY sink has
room, and gives each sink its own pair of entries — a slot per sink, rather
than ANDing the sinks' readys together, which would rebuild exactly the
combinational coupling the salt protocol removes. Both are a datapath and no
states, because writing either as a `process` would cost a cycle per hop for
something whose whole job is to pass an item along. One module is emitted per
shape (`ddl_merge_2x32`) however many times it is instantiated.

**Modules DDL did not compile.** `extern` names one, so a `graph` can be the
top level instead of a guest inside a hand-written one:

```
extern psram (req: buffer in mem_req_t, rsp: buffer out mem_rsp_t)
```

The connections are checked like any other instance and no module is emitted.
An `extern` declares pipes and nothing else: a graph connects pipes, so a
parameter of any other kind would be a port left floating in the instantiation.

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
- a `bram` read in a process with no states to spend a cycle in, and a `bram`
  with a reset, which cannot be inferred as one
- a `for` whose trip count is not known at compile time
- a blocking `@rcv`/`@send` in the condition of an `if` or the scrutinee of a
  `match`, which would have to be decided before it could be waited on
- one name bound to two different payload types across the arms of a `match`
  that waits, where the arms are states and the name is a single wire
- a computed index anywhere but the last step of an assignment target
- `==` on an enum that carries payloads, which would compare the padding too
- a width annotation on an enum that carries payloads, which would name the tag
  and read as the value
- reading a variant's payload without matching on its tag first
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
shims. So is the fuzzer's hang check, which is a wall-clock timeout and under
Miri measures the interpreter rather than the compiler.

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

Every checked-in `.v` is also linted, if Verilator is installed. That is an
oracle needing no reference: it reads the output as a synthesis tool would and
objects to width mismatches, inferred latches, undriven or multiply-driven
nets, combinational loops and nets flopped on both edges -- none of which an
assertion about the text can see, because it would have to know what to look
for. The output is clean at `-Wall`, so anything not waived in
`examples/lint.vlt` is a change in what the compiler emits. Verilator absent
means the check says so and passes; a check that cannot run must not be a
check that fails.

On Linux and macOS the package manager's `verilator` needs no configuration.
On Windows, MSYS2 has one, but its `verilator` is a Perl wrapper Git Bash's
Perl cannot load and the binary beside it carries an MSYS2-internal data path,
so both want saying:

```bash
pacman -S mingw-w64-x86_64-verilator      # in the MSYS2 shell
export VERILATOR=/c/msys64/mingw64/bin/verilator_bin.exe
export VERILATOR_ROOT=/c/msys64/mingw64/share/verilator
```

Under WSL it is an ordinary Linux install and neither variable applies. The
paths above are environment on the machine that has it, not facts in this
repository: `tests/lint.rs` knows only `$VERILATOR`, and falls back to PATH.

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
| `src/ir_comb.rs` | `@merge` and `@split` → modules the compiler writes |
| `src/ir_match.rs` | `match` → case |
| `src/verilog.rs` | the backend |
| `src/diag.rs` | source locations and the caret rendering |
| `src/driver.rs` | the passes, in order |

`src/main.rs` is argument parsing and exit codes over the library in
`src/lib.rs`; anything that wants to compile DDL without spawning a process
links the library.
