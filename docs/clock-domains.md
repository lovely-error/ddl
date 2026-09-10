# Clock Domains

DDL compiles to **exactly one clock domain**. This document is about what that
means at the edges of a compiled design, what happens if you cross a domain
without saying so, the module to use when you need to, and the flag that makes
the compiler wire it in for you.

- [What a boundary guarantees](#what-a-boundary-guarantees)
- [What happens if you cross one anyway](#what-happens-if-you-cross-one-anyway)
- [The module](#the-module)
- [Wiring it](#wiring-it)
- [Constraints](#constraints)
- [Reset](#reset)
- [Letting the compiler wire it](#letting-the-compiler-wire-it)
- [Why the compiler does not do this by default](#why-the-compiler-does-not-do-this-by-default)

---

## What a boundary guarantees

Every module DDL emits has `clk` and `rst_n`, and **every port on it is
synchronous to that `clk`**. That holds for all three faces the compiler
presents:

| face | ports | where |
|---|---|---|
| Show-Ahead FIFO, consumer | `p_can_receive` / `p_receive_en` / `p_data_write_in` | an `--export` target's `buffer in`, or an `extern` target's `buffer out` |
| Show-Ahead FIFO, producer | `p_has_data` / `p_drop_item` / `p_data_read_out` | an `--export` target's `buffer out`, or an `extern` target's `buffer in` |
| the salt protocol | `p_wsalt` / `p_rsalt` / `p_data` | `--bare-export`, and between two compiled modules |

The salt protocol has a property that reads as if it should survive a crossing,
and does not. [`salt-protocol.md`](salt-protocol.md) says a slot is stable
for as long as it is offered: the producer writes the entry and advances its
`wsalt` on the same edge, and does not rewrite that slot until the consumer's
`rsalt` says it may. **That is a same-domain guarantee.** It rests on the
producer and the consumer agreeing about when a cycle is, and two clocks do not
agree about that.

Gray coding protects the *pointer* — one bit changes per step, so a pointer
sampled mid-transition reads as the old value or the new one, never a mixture.
Nothing protects the *decision made from it*, and nothing protects the payload.

## What happens if you cross one anyway

Not a rare glitch. Measured on a Tang Nano 9K with the compiler's own emitted
link between two PLL domains — the full experiment is in
[`cdc-demo/`](../cdc-demo/README.md):

| arrangement | throughput | corrupt items |
|---|---|---|
| one clock (control) | 100% of ceiling | **0** |
| two clocks, wired directly | 67% | **100%** — 54,000,000 errors/second |
| two clocks, `ddl_cdc_fifo` wired by hand | **100%** | **0** in 1.14 billion items |
| two clocks, the same FIFO **wired by `--async-export`** | **100%** | **0** in 156 billion items, at three clock ratios |

The corrupted data is not late or reordered. At one clock ratio the payload was
statistically indistinguishable from random noise: exactly 1 item in 65,536
matched, which is 2⁻¹⁶ — the rate at which a *uniformly random* 16-bit value
coincides with a 16-bit expectation.

Three properties of this failure are worth knowing before you decide you do not
have the problem:

* **It is invisible in simulation.** Verilog samples old-or-new and models no
  metastability, so an unsynchronized crossing simulates perfectly. A clean
  testbench says nothing about it.
* **Its severity depends on clock alignment.** At 81→108 MHz it corrupted 100%
  of items; at 81→94.5 MHz, 26%. The milder presentation is the more dangerous
  one — a design delivering three-quarters of its data intact passes a smoke
  test and fails in the field.
* **The mechanism is not what people usually expect.** It is not primarily
  metastability. `full` and `empty` are combinational functions of an
  asynchronous input, and each fans out to several registers that can disagree
  about the flag *within one cycle* — one advances the pointer while another
  does not capture the item. Metastability MTBF is measured in years; this was
  measured in tens of nanoseconds.

## The module

[`lib/ddl_cdc_fifo.v`](../lib/ddl_cdc_fifo.v) — a gray-pointer asynchronous FIFO
with two-flop synchronizers, a Show-Ahead read side, and portable synthesis
attributes. It is the module that produced the zero-error row above.

```verilog
ddl_cdc_fifo #(
    .WIDTH (32),   // payload bits
    .AW    (3)     // depth is 2**AW; AW >= 2
) u_cross ( ... );
```

**Choosing `AW`.** The credit loop is about six cycles — three in each
direction, for a synchronizer pair and the act that follows it — so sustained
throughput is roughly `depth / 6` of the slower clock's cycle rate, capped at 100% (1.0 item/cycle). Measured:

| depth | throughput |
|---|---|
| 2 (salt equivalent) | 33% of the slower clock — *unsupported by `ddl_cdc_fifo` (`AW >= 2`); listed for context* |
| 4 (`AW` 2) | 67% |
| **8 (`AW` 3)** | **100%** |
| 16 (`AW` 4) | 100% |

`AW = 3` is the smallest that streams at the full rate of the slower clock at
any ratio. Smaller is correct but slower. Larger buys burst buffering, at a
flip-flop per bit per entry.

`AW` rather than `DEPTH` because `$clog2` in a part-select bound makes
GowinSynthesis exit with an empty log.

## Wiring it

The glue is one AND gate in each direction. These follow the same shape as the
AXI4-Stream snippets in [`docs/salt-protocol.md`](salt-protocol.md).

**Into an exported `buffer in p`** — foreign logic writes, DDL consumes:

```verilog
wire        rempty;
wire [15:0] rd;

ddl_cdc_fifo #(.WIDTH(16), .AW(3)) u_in (
    .wclk  (io_clk), .wrst_n(io_rst_n),
    .wpush (io_valid), .wdata(io_data), .wfull(io_full),
    .rclk  (clk),    .rrst_n(rst_n),
    .rpop  (p_receive_en), .rdata(rd), .rempty(rempty)
);

assign p_receive_en    = !rempty && p_can_receive;   // the only glue
assign p_data_write_in = rd;
```

**Out of an exported `buffer out p`** — DDL produces, foreign logic reads:

```verilog
wire wfull;

ddl_cdc_fifo #(.WIDTH(16), .AW(3)) u_out (
    .wclk  (clk),    .wrst_n(rst_n),
    .wpush (p_drop_item), .wdata(p_data_read_out), .wfull(wfull),
    .rclk  (io_clk), .rrst_n(io_rst_n),
    .rpop  (io_ready), .rdata(io_data), .rempty(io_empty)
);

assign p_drop_item = p_has_data && !wfull;           // the only glue
```

Both faces are Show-Ahead on the read side, so `rdata` and `p_data_read_out` are
valid whenever the corresponding empty flag is low. There is no read latency to
account for and no separate `valid` to track.

An `extern` presents the mirror of these, so the same two snippets apply with
the roles swapped. See
[Guide 3](guides/3-interfacing-and-integration.md).

## Constraints

**Without these the tool will try to time the crossing**, and may report it
closed — at which point the design is one placement change away from breaking,
and the timing report will not warn you.

```tcl
# Vivado (XDC), Quartus (SDC) and Gowin (SDC) all spell it the same way.
set_clock_groups -asynchronous -group [get_clocks clk] -group [get_clocks io_clk]
```

Two practical notes, both learned the hard way in `cdc-demo/`:

* Gowin's SDC parser accepts **no line continuation** — `set_clock_groups` must
  be one line — and a constraint file **replaces** rather than adds to another,
  so each file must create every clock it names or you get "Cannot get clock with
  name".
* A PLL output should be declared as a **generated** clock chained from its
  source, not as a base clock. A base clock has no master, so the tool stops
  knowing the two share an origin and invents a skew between them.

For a nextpnr flow, constrain each clock with `--freq` or a per-clock constraint
in the project file; there is no `set_clock_groups`, and unrelated clocks are
already treated as unrelated.

To confirm it took, check that no paths were analysed between the two domains.
`cdc-demo/build.sh` does this automatically and prints either

```
crossing: no inter-domain paths analysed (the domains are declared asynchronous)
```

or a warning that the tool is timing the crossing.

## Reset

*(Note: If you use the compiler flags `--async-export` or `--async-extern`, the compiler handles this automatically using `ddl_rst_cross` as described [below](#letting-the-compiler-wire-it). This section explains the physical requirements when hand-wiring the crossing.)*

**One reset source, synchronized into each domain separately.** Not two
independent resets.

```verilog
// one source, gated on whatever must be ready first
wire por = rst_n_pin & pll_locked;

// then two flops in EACH domain, on that domain's own clock
reg m_a, s_a;  always @(posedge clk)    begin m_a <= por; s_a <= m_a; end
reg m_b, s_b;  always @(posedge io_clk) begin m_b <= por; s_b <= m_b; end
```

Why it matters: resetting one side alone zeroes one pointer and leaves the other
mid-lap. The FIFO then reports occupancy that was never written, or space that is
not there, and stays wrong forever. It is the one failure that survives adding
synchronizers.

The release skew between the two domains is harmless, and this was measured
rather than assumed: **both pointer sets start at zero**, so whichever side comes
up first either stalls on full or idles on empty until the other joins it. In
`cdc-demo/` the two domains released 0.79 µs apart and the link ran a billion
items with zero errors.

Hold the reset for at least three cycles of the **slower** clock — two for the
synchronizer to propagate and one to act on it. The compiler emits *synchronous*
reset (`if (!rst_n)` inside the clocked block), so a pulse shorter than one clock
period of a domain can be missed by that domain entirely, which is a single-side
reset by another name.

## Letting the compiler wire it

Everything above is the hand-wired route, and it stays the default: with no
flag, the compiler emits exactly what it emitted before this existed, byte for
byte. Naming a boundary makes the compiler write the crossing into the `.v` and
wire it up for you.

```
--async-export  <module>.<pipe>[=<domain>][:<depth>]
--async-extern  <graph>.<instance>.<pipe>[=<domain>][:<depth>]
```

[**Guide 6**](guides/6-clock-domain-crossings.md) is the step-by-step version of
this section: worked examples for both flags, the port lists they produce, how to
check the constraint took, and a troubleshooting table. What follows here is the
summary.

```bash
ddl build filt.ddl --export filt --async-export filt.rx=io,filt.tx=io -o filt.v
```

That emits `ddl_cdc_fifo` and `ddl_rst_cross` into the file **verbatim** — the
same bytes as `lib/`, which are the bytes measured on hardware — plus a thin
per-shape shell holding the parameter override, one AND gate of glue and the
reset handshake. The module gains one `io_clk` port, and the banner names every
crossing and writes out the `set_clock_groups` line ready to paste.

* **`<domain>` is the grouping the compiler cannot infer.** Two pipes given the
  same domain share one clock port; two different domains get two. It defaults
  to the pipe's own name.
* **`<depth>` is per boundary**, a power of two at least 4, default 8.
* **An extern keeps its own `clk` and `rst_n`.** A crossed pipe adds
  `<pipe>_clk` and `<pipe>_rst_n` to the `<pipe>_<suffix>` ABI its face already
  uses — so `req_can_receive` is joined by `req_clk`, and the compiler never
  has to guess what the module's clock is called. Two pipes of one extern may
  sit on two different clocks.

The reset story is handled for you: `rst_n` stays the only reset the design
takes, and `ddl_rst_cross` carries it across so both halves of every crossing
leave zero together, at any ratio and however short the pulse.

**This is measured, not asserted.** `cdc-demo/` arm G is arm E's experiment run
again against compiler output — one `ddl build` command is the whole difference,
and nothing edits what it emits. Both shells are on the board: a `buffer out`
crossing outward and a `buffer in` crossing inward, each taking one reset and
deriving the far side itself.

| clocks | ceiling | outward crossing | inward crossing | errors | items |
|---|---|---|---|---|---|
| 81 → 108 MHz | 0.750 | **0.750000000** | 0.500000000 | **0** | 138.4 G |
| 81 → 94.5 MHz | 0.857 | **0.857142857** | 0.500000000 | **0** | 8.9 G |
| 67.5 → 108 MHz | 0.625 | **0.625000000** | 0.500000000 | **0** | 8.3 G |

156 billion items, not one error, every rate exactly its ceiling. The inward
column reads 0.500 because the process behind it forwards one item every two
cycles — the same figure it reaches on a *single* clock, which is the sharper
way to say the crossing costs nothing. Post-synthesis the FIFO's pointers, both
synchronizer pairs and all six handshake flops survive by name, and the tool
analysed no path between the two domains.

The reset rule above was tested the way it actually gets violated: the board's
reset button pressed **seven times at unplanned intervals**, asynchronously,
while items were in flight. Every release returned both pointer sets to zero
together and every one of the 323 report periods afterwards ran at exactly the
ceiling, with the error counters never leaving zero. A desync would have shown
as one error and then a permanently wrong occupancy; none appeared. The
single-cycle-`rst_n` case a board cannot produce — the one a plain two-flop
synchronizer misses entirely — is covered in simulation at ratios from 1:1 to
1:128.

What the flag cannot do is notice that you needed it. A boundary you do not name
is a boundary with no crossing, and that is the failure this page opens with.

## Why the compiler does not do this by default

It will, for a boundary you name. What it will not do is put one at *every*
boundary, and an earlier design did exactly that. That was dropped for reasons
worth stating, since it is easy to ask for:

* **The compiler does not know your clocks.** It knows neither their
  frequencies, nor their phase relationships, nor which faces share a domain.
  These are system-level integration facts, and DDL's design philosophy draws
  the boundary in the same place: *compute in DDL, I/O in Verilog* (see
  [Design Philosophy](overview.md#design-philosophy)).
* **Depth is application-specific.** 8 streams at full rate, but a bursty slow
  boundary may want 64 and a tight one 4.
* **It would be a promise DDL cannot verify.** Synthesis attributes are a
  request; the only guarantee is a post-synthesis netlist check, and that is
  per-toolchain. A vendor primitive — Xilinx `XPM_CDC_FIFO`, Intel `DCFIFO` —
  comes with the vendor's own verification and appears in the vendor's own CDC
  report. Use one instead of this module if you prefer; the wiring is the same.
* **Everyone would pay.** Four cycles of latency each way and a few hundred
  flip-flops per face, in designs that mostly have one clock.

The cost of that choice is that **nothing will warn you.** A design that crosses
a domain without a FIFO compiles, lints and simulates clean. That is why this
page leads with the measurement rather than the advice.
