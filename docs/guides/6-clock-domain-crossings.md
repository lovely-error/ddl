# Guide 6: Crossing a Clock Domain with `--async-export` and `--async-extern`

This guide is about naming a boundary that runs on a different clock and letting
the compiler write and wire the crossing for you. It is the practical companion
to [Clock Domains](../clock-domains.md), which carries the measurements and the
reasoning; this one is the recipe.

---

## What You Will Learn

- How to tell whether you need a crossing at all.
- The two flags, and what each part of the spec means.
- What appears on your module's port list, and what appears in the `.v`.
- Wiring an exported crossing and an `extern` crossing, with real port names.
- The constraint you must add, and how to check the tool honoured it.
- What the reset does for you, and the one thing that can hang.
- Choosing a depth, and what each entry costs.

---

## 1. Do you need one?

**Every port on every module DDL emits is synchronous to that module's `clk`.**
That holds for all three faces the compiler presents — the Show-Ahead FIFO on an
`--export` target, its mirror on an `extern`, and the raw salt ports of
`--bare-export`.

So you need a crossing exactly when logic on the far side of one of those faces
runs on a different clock: a PLL output, a clock recovered from a link, a slower
peripheral domain, vendor IP with its own `aclk`.

**Nothing will tell you if you get this wrong.** A face wired directly to another
clock compiles, lints clean under `verilator -Wall`, and simulates perfectly —
Verilog samples old-or-new and models no metastability. On hardware the same
design corrupted **100% of items at 54 million errors per second**
([`cdc-demo/`](../../cdc-demo/README.md)). That asymmetry is the whole reason
this flag exists, and the reason a clean testbench is not evidence.

> **If everything is on one clock, stop here.** With no flag the compiler emits
> exactly what it emitted before this feature existed, byte for byte. You pay
> nothing for a crossing you did not ask for.

## 2. The two flags

```text
--async-export  <module>.<pipe>[=<domain>][:<depth>]
--async-extern  <graph>.<instance>.<pipe>[=<domain>][:<depth>]
```

Both flags are repeatable and accept comma-separated lists. Each crossing specification consists of three components:

| part | meaning |
|---|---|
| the path | which boundary crosses: an export target and one of its pipes, or a graph, an `extern` instance in it, and one of that instance's pipes |
| `=<domain>` | **the grouping the compiler cannot infer.** Two pipes given the same domain share one clock port; two different domains get two. Defaults to the pipe's own name |
| `:<depth>` | entries, a power of two at least 4. Default 8 |

**A crossing is named per pipe, never per module or per instance.** Vendor IP
with `s_axi_aclk` and `m_axi_aclk` puts two pipes of one `extern` on two
different clocks, and that is two independent FIFOs. Conversely, four pipes that
really do share a clock should all be given the same `=<domain>` so they share
one port.

## 3. Exporting a face onto another clock

`cdc-demo/gen_chk.ddl` has a two-pipe relay, which is enough to show both
directions:

```ddl
process chk (src: buffer in u16, dst: buffer out u16)
  loop
    let v = @rcv(src)
    @send(dst, v)
```

Cross only `src`, leaving `dst` on the core clock:

```bash
ddl build cdc-demo/gen_chk.ddl --export chk --async-export chk.src=tx -o chk.v
```

The module gains **one port**:

```verilog
module chk (
    input         clk,
    input         rst_n,
    output        src_can_receive,
    input         src_receive_en,
    input  [15:0] src_data_write_in,
    input         tx_clk,                  // <- the crossed face's clock
    output        dst_has_data,
    input         dst_drop_item,
    output [15:0] dst_data_read_out
);
```

`src_*` is now synchronous to `tx_clk`; `dst_*` is still synchronous to `clk`.
Drive each face exactly as [Guide 3](3-interfacing-and-integration.md) describes
— the handshake rules do not change — just on the right clock:

```verilog
chk u_relay (
    .clk(core_clk), .rst_n(rst_n),
    // this face lives on tx_clk
    .tx_clk            (tx_clk),
    .src_can_receive   (src_room),
    .src_receive_en    (src_room && tx_valid),
    .src_data_write_in (tx_data),
    // this one is still on core_clk
    .dst_has_data      (out_valid),
    .dst_drop_item     (out_valid && out_ready),
    .dst_data_read_out (out_data)
);
```

**There is no `tx_rst_n`, and that is deliberate.** `rst_n` stays the only reset
the design takes; the crossing derives its own far-side reset internally. See
section 6.

### Two pipes, one clock

Naming the same domain twice shares the port:

```bash
ddl build gen_chk.ddl --export chk --async-export chk.src=io,chk.dst=io -o chk.v
```

gives **one** `io_clk` for both faces, with two independent FIFOs behind it.
Different domain names would give `src_clk` and `dst_clk`.

## 4. Crossing to an `extern`

`examples/fanout.ddl` ends by handing a pipe to hand-written Verilog:

```ddl
extern sink_ext (a: buffer in u32)

graph fanout (hi: buffer in u32, lo: buffer in u32, out_a: buffer out u32)
  ...
  sink_ext(copy)
```

**Instance names are generated, not written by you.** The first instantiation of
a declaration is `u_<declaration>`, the second `u_<declaration>_1`, then `_2` —
stable, and readable in a waveform. So:

```bash
ddl build examples/fanout.ddl --export fanout \
    --async-extern fanout.u_sink_ext.a=io -o fanout.v
```

Here the graph gains **two** ports, a clock and a reset:

```verilog
module fanout (
    ...
    input         io_clk,
    input         io_rst_n
);
```

and the extern instance is wired like this:

```verilog
  sink_ext u_sink_ext (
    .clk             (clk),        // UNCHANGED -- the extern's own core clock
    .rst_n           (rst_n),      // UNCHANGED
    .a_can_receive   (u_sink_ext_a_can_receive),
    .a_receive_en    (u_sink_ext_a_receive_en),
    .a_data_write_in (u_sink_ext_a_data_write_in),
    .a_clk           (io_clk),     // added by the crossing
    .a_rst_n         (io_rst_n)    // added by the crossing
  );
```

Three things to take from that:

* **The instance keeps `.clk(clk)` and `.rst_n(rst_n)`.** Crossing a pipe does
  not take away the module's core clock, and you are not asked to declare one.
* **The crossed pipe's clock is named the way its face is named.** `a_clk` and
  `a_rst_n` join `a_can_receive`, `a_receive_en` and `a_data_write_in` on the
  same `<pipe>_<suffix>` pattern, so your hand-written module has one spelling to
  match and the compiler never has to guess a clock name.
* **`io_rst_n` exists here only because your `extern` needs one.** It is a
  pass-through to the instance; the crossing does not use it and derives its own.
  That is why the export form in section 3 has no reset port — there is no
  foreign module there to reset.

Two instantiations of the same `extern` are two separate targets, and naming one
crosses only that one:

```bash
--async-extern two.u_sink_ext_1.a=io      # u_sink_ext keeps clk; u_sink_ext_1 crosses
```

If you are unsure what an instance is called, build once without the flag and
read the instance names out of the emitted `.v`.

## 5. What lands in the `.v`

Three things, and only when at least one crossing is asked for:

| | |
|---|---|
| **`ddl_cdc_fifo`** | [`lib/ddl_cdc_fifo.v`](../../lib/ddl_cdc_fifo.v) **verbatim**, via `include_str!` — the emitted bytes are the reviewed bytes, and the ones measured on hardware. Emitted once however many crossings you have |
| **`ddl_rst_cross`** | [`lib/ddl_rst_cross.v`](../../lib/ddl_rst_cross.v) verbatim; the reset handshake, six flops |
| **`ddl_cdc_<kind>_<W>x<D>`** | a ~15–20 line shell per *shape*: the parameter override, one AND gate of glue, one `ddl_rst_cross`. `in`/`out` for an export face, `to_ext`/`from_ext` for an extern |

Two crossings of the same width, depth and direction share one shell. The part
that is hard to get right is the part nobody rewrites; the generated part is
fifteen obvious lines.

The banner names every crossing and hands you the constraint:

```verilog
// CLOCK-DOMAIN CROSSINGS in this file:
//   fanout.u_sink_ext.a crosses into `io_clk`
//
// Your .sdc/.xdc MUST declare these asynchronous, or the tool will time
// the crossing. One line, no continuation -- Gowin's parser takes none:
//
//   set_clock_groups -asynchronous -group [get_clocks clk] -group [get_clocks io_clk]
```

## 6. Reset: what you get, and the one thing that hangs

**You drive one reset.** `rst_n` is synchronous to `clk`, and `ddl_rst_cross`
carries it into the far domain and waits for an acknowledgement before releasing
either half. Both of a FIFO's pointer sets therefore leave zero together — at any
clock ratio, and however short the pulse on `rst_n`.

That matters because a pointer FIFO reset on one side only is the one failure
that *survives* adding synchronizers: one pointer zeroes while the other is
mid-lap, the FIFO reports occupancy that was never written, and it stays wrong
forever. Doing it by hand means a documented minimum pulse width, and a width
rule that gets violated fails silently and permanently.

Verified: single-cycle `rst_n` at foreign ratios 1:1, 1:8, 1:32, 1:128 and 8:1 in
`cdc-demo/sim/tb_rst_cross.sv`, and on a Tang Nano 9K with the reset button
pressed seven times at unplanned intervals mid-stream — 323 report periods
afterwards, every one at exactly the expected rate, error counters never leaving
zero.

**The one failure mode: a foreign clock that is not running holds that boundary.**
If `io_clk` is stopped — PLL not locked, clock gated, board still coming up — the
acknowledgement never returns and that pipe never moves. That is true of any
crossing, and it is the right trade: **when the clock does start, the ack returns
and both halves release from zero, so it self-heals.** A plain synchronizer that
had missed the reset pulse comes up desynchronised and stays wrong permanently.
The design that can hang is the one that recovers.

> **Bring-up rule: if a crossed pipe never moves, check that its clock is running
> before suspecting anything else.** The banner says this too.

Ordering does not matter. `rst_n` and any `<domain>_rst_n` may be asserted and
released in any order, together or apart; nothing generated depends on their
relative timing, and the compiler never clocks anything with a foreign reset.

## 7. Choosing a depth

The credit loop is about six cycles — three each way, for a synchronizer pair and
the act that follows it — so sustained throughput is roughly `depth / 6` of the
slower clock, capped at 1. Measured, not estimated:

| depth | throughput |
|---|---|
| 2 | 33% of the slower clock — *not selectable, listed for context* |
| 4 | 67% |
| **8** (default) | **100%** |
| 16 | 100% |

**8 is the smallest that streams at the full rate of the slower clock at any
ratio.** Depth 2 is what the salt protocol itself carries, which is why it is in
the table; the flag's minimum is 4, because the full test compares the top two
pointer bits and needs an address bit beneath them. Smaller is correct but slower; larger buys burst buffering at a
flip-flop per bit per entry. A crossing costs `depth × W` flip-flops of storage
plus about 40 for the pointers, the two synchronizer pairs and the reset
handshake, and four cycles of latency each way — so a 32-bit pipe at the default
depth is about 296 flops per face.

Ask for something else per boundary:

```bash
--async-export filt.rx=io:64        # bursty and slow on the far side
--async-extern top.u_ip.cmd=axi:4   # tight; latency matters more than rate
```

## 8. Checking it actually worked

Three checks, in the order they can fail.

**1. The constraint took.** Without `set_clock_groups -asynchronous` the tool
will try to time the crossing and may report it closed — at which point the
design is one placement change away from breaking and the timing report will not
warn you. Confirm no paths were analysed between the two domains;
`cdc-demo/build.sh` reads this out of `project_tr_content.html` and prints either

```
crossing: no inter-domain paths analysed (the domains are declared asynchronous)
```

or a warning that the tool is timing it. Two Gowin-specific traps:
`set_clock_groups` must be **one line** (the SDC parser takes no continuation),
and a constraint file **replaces** rather than adds to another, so each file must
create every clock it names or you get "Cannot get clock with name".

**2. The synchronizer flops survived.** Attributes are a request, not a
guarantee. Grep the post-synthesis netlist to confirm that critical synchronizer and handshake registers were preserved:

```bash
grep -E "wgray_meta|wgray_sync|rgray_meta|rgray_sync|resetting|f_meta|f_sync|ack_meta|ack_sync" netlist.v
```

A duplicated or merged synchronizer flop is two flops that can resolve
differently from one metastable input. `ddl_cdc_fifo` carries the portable
attribute set — `ASYNC_REG`, `keep`, `PRESERVE`, `altera_attribute`,
`syn_preserve` — but **only Vivado also constrains placement**, and the netlist
check is the only real guarantee. It has been verified on Gowin; a first build on
another vendor should re-run the equivalent grep rather than assume it carried.

**3. It moves at the rate you expect.** Depth 8 should give you the slower
clock's full rate. Anything lower means a shallower depth than you think, or
backpressure from your own logic.

## 9. Troubleshooting

| what you see | what it means |
|---|---|
| `depth 6 is not usable: it must be a power of two and at least 4` | The full test compares the top two pointer bits and needs an address bit beneath them |
| `chk has no pipe called nope` (plus note listing real pipes) | Typo in the pipe name |
| `--async-export names gen, which is not being wrapped` | That module is not an `--export` target. `--bare-export` keeps raw salt ports and **cannot** carry a crossing — a crossing goes on a face, and a bare export has none |
| `chk.src= is not a --async-export target: the domain after = is empty` | `=` with nothing after it |
| `instance u_<name>_core refers to unknown port <domain>_clk` | **A mistyped graph, instance, or pipe in `--async-extern`.** The build fails, but the message points at compiler internals rather than at your flag; check the three dotted parts against the instance names in the emitted `.v` |
| The crossed pipe never moves | Its clock is not running. Check that before anything else |
| It moves, but below the ceiling | Depth limit or downstream backpressure — not the crossing |

## 10. What this does not do

* **It does not notice that you needed it.** A boundary you do not name is a
  boundary with no crossing, and that is the failure this guide opens with. The
  flag is load-bearing for correctness while living in a build script, so keep it
  next to the `--export` it belongs to.
* **It does not touch `--bare-export`.** Raw salt ports stay raw.
* **It does not let two compiled modules inside one `graph` sit on different
  clocks.** Only the boundaries — export faces and `extern` instances.
* **It is not a vendor primitive.** Xilinx `XPM_CDC_FIFO` and Intel `DCFIFO` come
  with the vendor's own verification and appear in the vendor's own CDC report.
  Use one instead if you prefer; the wiring is identical, and
  [Clock Domains](../clock-domains.md) has the hand-wired snippets.

---

## Evidence

Everything above is measured on a Tang Nano 9K rather than argued.
[`cdc-demo/`](../../cdc-demo/README.md) is the experiment and stays in the tree
as a regression harness. Arm G is this feature: the same board, the same
checkers, with the crossing written and wired by `ddl build`.

| clocks | ceiling | outward crossing | inward crossing | errors |
|---|---|---|---|---|
| 81 → 108 MHz | 0.750 | **0.750000000** | 0.500000000 | **0** |
| 81 → 94.5 MHz | 0.857 | **0.857142857** | 0.500000000 | **0** |
| 67.5 → 108 MHz | 0.625 | **0.625000000** | 0.500000000 | **0** |

156 billion items, not one error, every rate exactly its ceiling. The inward
column reads 0.500 because the process behind it forwards one item every two
cycles — the same figure it reaches on a *single* clock, which is the sharper way
to say the crossing costs nothing.

## See also

- [Clock Domains](../clock-domains.md) — the measurements, the mechanism, and the hand-wired route
- [Guide 3: Interfacing and Integration](3-interfacing-and-integration.md) — the faces themselves
- [FIFO Boundary Adapters & Export Architecture](../fifo-boundaries-and-export.md)
- [The Gray-Code Salt Protocol](../salt-protocol.md) — why the internal protocol is single-clock
