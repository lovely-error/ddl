# Does the 2-entry salt pipe survive a clock-domain crossing?

DDL compiles to exactly one clock domain. `clk`/`rst_n` are implicit ports
(`src/ir.rs:847`) wired name-to-name at every instantiation site, and the backend
writes a single `always @(posedge clk)` per module (`src/verilog.rs:1201`).
Nothing in the IR names a clock. Before changing that, this directory answers the
question the change rests on: **what actually happens if the two ends of a salt
pipe are given different clocks?**

Reading the source says it cannot be safe. `pipe_empty` compares the *raw
incoming* `wsalt` port against `rsalt_q` and `pipe_full` compares `wsalt_q`
against `~rsalt` straight off the port (`src/ir.rs:1635-1653`), so an
asynchronous input fans out combinationally into several registers' next-value
logic. The emitted `chk` module in `gen_chk.v` shows it plainly:

```verilog
wire src_empty = src_wsalt == src_rsalt_q;      // combinational off an async pin
wire fire_s0   = in_s0 & (!src_empty);          // fans out to four registers
```

`fire_s0` reaches `state`, `src_rsalt_q`, `v_r` and the output side. Each sees it
through different routing, so a transition near a clock edge can be seen by one
and not another within a single cycle. That is the failure this measures.

There is prior art for the claim. `KAMASUTRA2G/rtl/k2g_cdc_fifo.sv` states that
its ancestor — `docs/attic/pipe.sv`'s `PipeCDC`, the same protocol DDL emits —
was *"kept for the idea, not the implementation"*, because *"its salt bits cross
domains with no synchronizer at all"*. The same header records why this has to be
settled on hardware: *"THIS IS THE ONE THING COSIMULATION CANNOT CHECK."*

---

## Two devices under test, one bitstream

**Slot 1, `pipe_cdc.v`** — `PipeCDC` transcribed with `Item = u16`, write side on
clock A and read side on clock B. It is the same protocol DDL emits, not merely a
relative: walking its writer from reset gives `00 → 01 → 11 → 10 → 00`, DDL's
gray sequence (`docs/salt-protocol.md:92`), with `write_index` equal to
`salt[0]^salt[1]` at every state — exactly what DDL derives instead of storing.
The flags agree at all three occupancies: `write_salt[wi] != read_salt[wi]` holds
exactly when `wsalt == ~rsalt`, and `read_salt[ri] == write_salt[ri]` exactly when
`wsalt == rsalt`.

**Slot 2, `gen_chk.ddl`** — the compiler's own link, so the claim lands on the
code DDL ships. A producer `process` on clock A and a consumer on clock B, both
`--bare-export`ed, with the three bundles between them exactly what
`src/ir_graph.rs:762-764` emits for two instances in a graph.

Every endpoint feeding or draining a slot is single-domain, where the protocol is
known good. Only the object in the middle crosses.

## The arms

| arm | build | what it settles |
|---|---|---|
| **A** | both sides on clock A | the harness and the checkers are clean |
| **B** | two clocks, salts straight across — what the compiler emits today | whether an unsynchronized salt is a real defect or only a theoretical one |
| **C** | as B, plus 2-flop synchronizers, each into the domain that reads it | that depth 2 cannot stream across a crossing |
| **E** | a proven depth-8 async FIFO in place of the pipes | that the fix works on this board before the compiler is taught to emit it |
| **F** | as B, but both indices **derived** from their own salt instead of stored | how much of B's damage comes from the index drifting out of step with the salt |
| **G** | the crossing **written and wired by the compiler**, from `--async-export` | that what the compiler emits is the thing arm E measured, on the board and not only in a bench |

There is deliberately **no reset arm**. Resetting one side alone desynchronises
the pointers — but that is a two-line argument, not a measurement, and the reset
handshake in Phase 1 gets built either way. It is also not evidence about the
salt crossing: it is a property of *any* pointer FIFO, including the correct
depth-8 one, so it could never stand in for arm B.

## Running it

```bash
bash cdc-demo/build.sh a                # then b, c, e, f, g
bash cdc-demo/build.sh b --pair 1       # the same arm at another clock ratio
bash cdc-demo/build.sh b --flash        # build and load over JTAG into SRAM
bash cdc-demo/sim/run.sh                # throughput across the crossing
bash cdc-demo/sim/run_top.sh b          # the whole top, to check the instrument
```

Then a serial terminal at 115200 8N1. One line a second, all hex:

```
<arm> rx1 err1 idle1_hw rx2 err2 idle2_hw cyc
```

* `rx` — items received, `err` — sequence mismatches, `cyc` — clock-B cycles.
  Throughput is `rx / cyc`; all three start together after a 2048-cycle warm-up.
* `idle_hw` — the bitwise OR of every idle count seen. Its **top set bit** is the
  magnitude of the longest stall; the lower bits are not a number.
* LEDs (active low): 0 = any error ever, 1 = locked up, 2-5 = low bits of `err1`.

**Arm B's verdict is `err` still climbing after ten minutes, not `err > 0`.** The
sinks resync on a mismatch, so a startup or reset desync is a constant offset and
costs exactly one error, while a crossing fault is a recurring race that keeps
accumulating. That distinction is the whole reason the checker resyncs.

---

## What simulation already settles

`sim/run.sh`, in Questa. Arm C's claim is a counting argument about a credit loop,
so it is exact in RTL and needed no bitstream. Items per clock-B cycle, against
the ceiling (the slower clock's rate):

| clocks | ceiling | depth 2, no sync | **depth 2, synchronized** | depth 8, synchronized |
|---|---|---|---|---|
| 81 → 108 (pair 0) | 0.750 | 0.750 | **0.333 — 44% of ceiling** | 0.750 — 100% |
| 81 → 94.5 (pair 1) | 0.857 | 0.857 | **0.357 — 42%** | 0.857 — 100% |
| 67.5 → 108 (pair 2) | 0.625 | 0.625 | **0.312 — 50%** | 0.625 — 100% |
| 108 → 108 (equal) | 1.000 | 1.000 | **0.333 — 33%** | 1.000 — 100% |

The equal-clock figure is exactly `depth / RTT = 2 / 6`, the six-cycle credit loop
the depth-8 decision was derived from: write edge → sync0 → sync1 → the read that
consumes it is 3 reader cycles, and read edge → sync0 → sync1 → the writer reusing
the slot is 3 writer cycles. **A two-entry pipe across a synchronized crossing
runs at one third of the achievable rate. Eight entries runs at all of it.** That
is arm C and arm E, decided without hardware.

**The `depth 2, no sync` column is not evidence that the crossing works.** Verilog
samples old-or-new and models no metastability, so an unsynchronized crossing is
clean in simulation *by construction*. Only its throughput column means anything.
That asymmetry is exactly why arm B is on a board.

## The instrument, checked before it is trusted

`sim/run_top.sh` runs the whole top with a behavioural rPLL and the report
dividers shrunk, and confirms the reset tree releases, both sinks see items in
order, and the report line is well formed with the counters in the right places.
A garbled report on the board would look exactly like a broken DUT.

It also gives the **exact throughput each arm should show**, at pair 0 (81 → 108),
in items per clock-B cycle:

| arm | slot 1 (`pipe_cdc`) | slot 2 (the DDL link) | why |
|---|---|---|---|
| A | **1.000** | **0.500** | one clock; slot 2's `chk` is a two-state process, so it forwards one item every two cycles |
| B | **0.750** | **0.500** | 0.750 is the ceiling — clock A is the slower side, 81/108 |
| C | **0.333** | **0.333** | both throttled by the synchronized depth-2 crossing, below even `chk`'s own limit |
| E | **0.750** | **0.750** | depth 8 reaches the ceiling on both slots |
| G | **0.750** | **0.500** | the compiler's crossing reaches the ceiling too; slot 2 is back to `chk`'s own two-state limit, which is arm A's number on one clock |

`rx/cyc` on the board should match these. A number that does not is a problem
with the harness, not a discovery about the crossing.

## What the board said

Measured on a Tang Nano 9K, pair 0 (81 MHz → 108 MHz).

| arm | rx1/cyc | err1 | rx2/cyc | err2 | duration |
|---|---|---|---|---|---|
| **A** control | **1.000000** (predicted 1.000) | **0** | **0.500000** (predicted 0.500) | **0** | 537 M cycles, 6.6 s |
| **B** as emitted | 0.500000 (predicted 0.750) | **rx1 − 1** | 0.500000 | **rx2 − 86,015** | 2.68 G cycles, 24.9 s |
| **C** synchronized, depth 2 | **0.333333** (predicted 0.333) | **0** | **0.333333** (predicted 0.333) | **0** | 3.09 G cycles, 28.6 s |
| **E** synchronized, depth 8 | **0.750000** (predicted 0.750) | **0** | **0.750000** (predicted 0.750) | **0** | ~23 periods, 28 s |
| **G** compiler-emitted | **0.750000000** (predicted 0.750) | **0** | **0.500000000** (predicted 0.500) | **0** | 43.35 G cycles, 401 s |

The two-clock ceiling is 81/108 = **0.750**, set by the slower side. As a
fraction of what is achievable: arm B 67% (and corrupt), arm C **44%**, arm E
**100%**.

> **Read arm E from the deltas.** Its 32-bit counters wrapped
> (`0xfbfffa01 + 0x06000000 = 0x01fffa01`, and `cyc` likewise), so its absolute
> ratio is meaningless. `rx` gains 0x06000000 per 0x08000000 cycles = 0.750
> exactly. At these rates the counters wrap after about 40 seconds; arms A, B and
> C were short enough to read directly.

**Arm A is exact.** Both slots hit their predicted throughput to nine digits and
neither checker found a single error in 537 million cycles. The harness, the
checkers, the reset tree and the reporter are all sound, so arm B's numbers are
about the crossing and nothing else.

**Arm B fails totally.** Not a rare race — 54 million errors per second, sustained
for 25 seconds, on both DUTs at once. `err1 = rx1 − 1`: every item after the first.
The salt pipe does not survive a clock-domain crossing, and neither the ancestor
protocol nor the compiler's own emitted link is any different.

### The payload is noise, and that was not the prediction

The two slots fail with different signatures, and the difference identifies the
mechanism.

**Slot 2 receives uniform random data.** Of 67,108,864 items per report period,
exactly **1024** matched -- 1 in 65,536, which is 2^-16 to the digit. That is
precisely the rate at which a *uniformly random* 16-bit value coincides with a
16-bit expectation. The items arriving are not late, or skipped, or duplicated.
They are garbage.

**Slot 1's corruption is systematic, not random.** Zero coincidences in
1,342,176,257 items, where random data would have produced about 20,479.

This contradicts the prediction recorded before the measurement, which said
payload corruption would be **absent**: *"a slot is read only once the write salt
says it is committed... and a gray pointer sampled mid-transition reads
old-or-new, both conservative."*

### The mechanism, corrected

The first explanation offered for the above was that the salt wins a race against
the wider data bus and the reader samples a slot still in flight. **That is wrong
and is withdrawn.** The commit is atomic at the source -- storage and salt update
on the same sender edge -- and a gray-coded salt sampled mid-transition reads
old-or-new, both of which are protocol-legal. Wire delays do not explain it.

The mechanism is that `full` and `empty` are **combinational functions of an
asynchronous input, and each fans out to several destination registers**:

* writer: `do_put = put && !full` gates `e0`, `e1`, `write_salt[write_index]` and
  `write_index` -- four registers;
* reader: `do_drop = drop && valid` gates `read_salt[read_index]` and
  `read_index`, plus the item capture.

Those registers sit in different places with different routing from the
comparator. When the asynchronous input moves near a destination clock edge, some
latch the old value of the flag and some latch the new one **within the same
cycle**.

For `PipeCDC` that is catastrophic, because it has an invariant to break:
`write_index` must always equal `write_salt[0] ^ write_salt[1]`. If `write_index`
toggles while `write_salt[write_index]` does not, they diverge -- and from then on
`full` tests the wrong bit, writes land in the wrong slot, and the structure
decoheres. That is why the payload reads as noise.

This also accounts for something in the data. DDL **derives** its index
(`widx = wsalt[0]^wsalt[1]`) and so cannot desynchronize it from the salt;
`PipeCDC` stores it separately and can. Slot 1 was the more corrupt of the two at
both ratios -- 100% against 99.994%, and 75% against 26%. It does not save DDL,
whose `fire_s0` still fans out to `state`, `src_rsalt_q` and `v_r`, but it is a
real structural difference and the measurement shows it.

**Why the synchronizers fix it** is then not about delay. `write_salt_seen` is a
register output *in the reader's own domain*, so `empty` becomes a combinational
function of two registers both clocked by `clk_b`: an ordinary synchronous signal
with a full period to settle, seen identically by every register it feeds.
Disagreement becomes impossible.

### How many synchronizer stages, and what this experiment does not show

`MTBF = e^(t_r/tau) / (T_w * f_c * f_d)`, with `t_r ~ (N-1)*T_clk - t_setup`. At
108 MHz, 54 M asynchronous edges/s and `T_w` = 50 ps:

| tau | N=1 (2 ns slack) | N=2 | N=3 |
|---|---|---|---|
| 20 ps | 1e30 yr | 1e177 yr | 1e378 yr |
| 50 ps | 2.6e4 yr | 1e63 yr | 1e144 yr |
| 100 ps | **1700 s** | 1e25 yr | 1e65 yr |
| 200 ps | **0.08 s** | 1e6 yr | 1e26 yr |

So the stage count is properly a function of clock rate, transition rate and the
flop's `tau`. Two is the default because it buys an enormous exponential margin,
and because N=1 leaves the margin depending on downstream *slack*, which placement
can remove silently -- the 100-200 ps rows are the reason that matters.

**But arm B failed 54 million times per second, and nothing in that table is
within thirty orders of magnitude of it.** Arm B did not measure metastability; it
measured the deterministic incoherence above, which is why its fractions came out
exact (75.00%, 4/7) rather than as statistical scatter. So: **this experiment
establishes that synchronization is necessary. It does not establish that two
stages are.** One flop would have fixed everything observed here, because one flop
already makes the signal synchronous. The second stage rests on the MTBF
arithmetic, not on anything this board could show in 25 seconds.

### Arm F: does a derived index bound the damage?

Both indices are derivable. `read_index = read_salt[0] ^ read_salt[1]`, exactly as
on the write side -- walking the reader from reset gives `00 -> 01 -> 11 -> 10 ->
00` with the index following `0,1,0,1`. DDL already does this
(`src_ridx = src_rsalt_q[0] ^ src_rsalt_q[1]` in the emitted `chk`); `PipeCDC`
stores it in a register of its own.

It is a **logical no-op**: arms B and F simulate bit-identically (0.750 / 0.500,
zero errors), because in a zero-delay simulator a stored index and a derived one
are the same thing. Any difference on the board is therefore purely about how the
structure fails under asynchronous sampling, which is what makes it a clean test
of the mechanism above.

The hypothesis: with a STORED index, a fanout disagreement about `full` can toggle
`write_index` without toggling `write_salt[write_index]`, breaking the invariant
`write_index == write_salt[0]^write_salt[1]`. Once those diverge, `full` tests the
wrong bit, writes land in the wrong slot, and the structure decoheres
**permanently**. With a DERIVED index that invariant holds by construction, so the
same disagreement can only lose or corrupt **one item**.

Predicted, before measuring: slot 1's error rate drops sharply from 100% and its
throughput recovers toward the 0.750 ceiling, while slot 2 -- unchanged, and
therefore a control inside the same bitstream -- reproduces arm B. **Tempered:**
DDL already derives both indices and was still corrupted 99.994% of the time at
pair 0, so the expectation is that this helps and does not rescue. If slot 1 does
not improve at all, the mechanism account above is incomplete.

The netlist confirms the structure: arm B carries 11 index-register instances,
arm F carries **zero**, with both salts intact.

**Measured.** Slot 2 is unchanged in this bitstream and acts as an in-run control:

| | slot 1 rate | slot 1 err | slot 2 rate | slot 2 err |
|---|---|---|---|---|
| **B** indices stored | 0.5000 | **100.00%** | 0.5000 | 99.994% |
| **F** indices derived | **0.8750** | **14.29%** | 0.5000 | 99.997% |

Deriving the index cuts corruption **sevenfold**, and slot 2 reproduces arm B to
three decimals, so the improvement is real and not an artifact of the run.

**The throughput is the tell: 0.875 is ABOVE the 0.750 ceiling.** The producer runs
at 81 MHz and cannot make more than 0.750 items per clock-B cycle, so the reader is
*fabricating* items. The excess is 0.125/cycle, which as a fraction of reads is
0.125/0.875 = **1/7 = 14.2857%** -- and the measured error rate is **1/7**. Every
error is a phantom read, with no free parameter anywhere in the model.

So the failure changed character as well as magnitude:

* **stored index** -- the invariant `index == salt parity` breaks, the structure
  decoheres, items are LOST, and the rate falls *below* the ceiling;
* **derived index** -- the invariant holds by construction, nothing decoheres, and
  the residual fault is a bounded single-item event: the reader occasionally sees
  `valid` when it should not and reads a slot twice, pushing the rate *above* the
  ceiling.

It is still not a fix. 14.29% corruption is catastrophic and a read rate above the
production ceiling is proof of fabricated data. Only synchronization gives zero.

**And slot 2 explains what actually governs the severity.** DDL already derives
both indices, yet sits at 99.997% -- because `chk`'s `fire_s0` gates `state`, `v_r`
AND `src_rsalt_q`, three registers that can disagree, where derived-index
`PipeCDC` has only one salt bit and one storage register under its enable. The
governing quantity is not whether the index is derived; it is **how many registers
hang off an asynchronously-derived enable**. That belongs in `src/ir_cdc.rs` as a
design rule rather than as an accident of how it happens to be written.

### Arm B at a second clock ratio: the severity is alignment-dependent

Pair 1 (81 -> 94.5 MHz, ratio 6:7) against pair 0 (81 -> 108, ratio 3:4):

| | slot 1 rate | slot 1 corrupt | slot 2 rate | slot 2 corrupt |
|---|---|---|---|---|
| **pair 0** | 0.5000 | **100.00%** | 0.5000 | **99.994%** |
| **pair 1** | 0.5714 (4/7) | **75.00%** | 0.5000 | **26.27%** |

**The fractions are exact.** Slot 1 at pair 1 is precisely 75.00% corrupt at
precisely 4/7 items per cycle -- three items wrong out of every four, a periodic
pattern rather than a statistical one. That is the fingerprint of coherent clocks:
the sampling alignment recurs through a fixed set of positions and the salt/data
race is lost at some and won at others. 3:4 gives 12 distinct alignments and loses
at essentially all of them; 6:7 gives 42 and loses at three quarters.

The coherent-clock caveat this experiment was designed around therefore turned out
to be real, but not in the direction that threatened the result: it governs *how
badly* the crossing fails, not *whether* it does. Two independent ratios both
destroy the data, which is what closes the last question about arm B's validity.

**The milder presentation is the more dangerous one.** Slot 2 at pair 1 delivers
74% of its data intact. A design like that passes a smoke test and may survive a
short soak; it looks like it works and then corrupts a quarter of everything in
the field, where pair 0's total garbage is caught in seconds. With genuinely
asynchronous clocks the alignment drifts continuously instead of recurring, so the
rate would sit between these and wander with temperature and voltage. That is the
failure mode that ships.

### Arm E: depth 8 costs nothing that depth 2 was buying

**0.750000 on both slots, zero errors** -- the full ceiling, at the same perfect
correctness arm C achieved for a third of the rate. **2.25x the throughput of
depth 2.**

### Arm G: the compiler writes the same crossing, and it measures the same

Arm E proved a hand-wired FIFO fixes the seam. Arm G is the acceptance test for
Phase 2: **the same board, the same clocks, the same checkers, with the crossing
written and wired by `ddl build` instead of by a person.** One command is the
entire difference between the two arms --

```bash
ddl build gen_chk.ddl --export gen,chk --async-export gen.o=rx --async-export chk.src=tx
```

-- and nothing edits what comes out. The emitted file carries `ddl_cdc_fifo` and
`ddl_rst_cross` verbatim plus the two generated shells, and `cdc_demo_top.v`
connects clocks to it and nothing else. **In particular it passes no second
reset**: `rst_a_n` is the only reset `gen` takes and `rst_b_n` the only one `chk`
takes, and `ddl_rst_cross` carries each into the far domain by itself. That is
the part of Phase 2 no earlier arm exercised.

Both directions of the crossing are on the board rather than only in the bench.
Slot 1 is a `buffer out` crossing outward (`ddl_cdc_out_16x8`, core on clock A,
face read on clock B); slot 2 is a `buffer in` crossing inward
(`ddl_cdc_in_16x8`, core on clock B, `src` face driven from clock A) with its
`dst` left un-crossed where the sink already is.

| pair | clocks | ceiling | slot 1 `rx1/cyc` | slot 2 `rx2/cyc` | errors | items |
|---|---|---|---|---|---|---|
| 0 | 81 -> 108 | 0.750 | **0.750000000** = 3/4 | **0.500000000** = 1/2 | **0** | 54.19 G |
| 1 | 81 -> 94.5 | 0.857 | **0.857142857** = 6/7 | **0.500000000** = 1/2 | **0** | 8.93 G |
| 2 | 67.5 -> 108 | 0.625 | **0.625000000** = 5/8 | **0.500000000** = 1/2 | **0** | 8.30 G |

Longest continuous soak per pair; pair 0 was run four times in all, so the total
behind this table is **156 billion items across three clock ratios, not one
error, and every throughput exactly its ceiling as a ratio of small integers.**

Slot 1 reproduces arm E's 0.750 exactly at pair 0. Slot 2's 0.500 is not a
shortfall -- it is `chk`'s own two-state limit, the same number arm A measures for
that slot on a *single* clock, which is the sharper statement: the crossing costs
nothing at all. Arms E and B were measured at one and two ratios respectively;
arm G is the only arm run at all three, and the ceiling comes out exact at each.

`idle1_hw` reads `0x7f`, `0x3f` and `0xff` at pairs 0, 1 and 2 -- one stall of
64-127, 32-63 and 128-255 clock-B cycles respectively, and nothing after it. That
ordering is not the crossing and is worth naming, because a stall that varied with
clock ratio is exactly what a broken handshake would look like. It is the
harness's own reset tree: each domain holds for 256 of *its own* cycles, so clock
B releases 256/f_b - 256/f_a earlier than clock A and counts idle until clock A
joins it. That is 0.79, 0.45 and 1.42 us -- 85, 43 and 154 clock-B cycles, each
landing in the bracket observed. The 0.79 us figure is the one
`docs/clock-domains.md` already records for pair 0. No lockup, no drift, nothing
that accumulates over the soak.

#### The button, pressed seven times

Every soak above starts from a power-on reset, which is the easy case: nothing
has moved yet, so the two halves of a crossing cannot be out of step. The hard
case is a reset asserted *mid-stream*, asynchronously, while items are in flight
-- and that one is only reachable from the board, because it is a physical
button bouncing against two PLL outputs.

Pressed seven times at unplanned intervals during a 401-second capture:

```
335 report lines, 0 malformed
7 reset releases
err1 / err2 ever seen: 00000000 / 00000000
323 full periods, every one at exactly +06000000 and +04000000 per +08000000 cycles
```

The reporter fires once immediately on release -- `rpt_fire <= (rpt_tick == 0)`
with `rpt_tick` reset to zero -- so each press leaves its own signature in the
log: one all-zero line, then the counters climbing from nothing back to the
exact rate.

```
Gfbfffa01 00000000 a500007f a7fffc00 00000000 a500000f 4ffff801   <- running
G00000000 00000000 a5000000 00000000 00000000 a5000000 00000000   <- press
G05fffa00 00000000 a500007f 03fffc01 00000000 a500000f 07fff801   <- first period back
G0bfffa01 00000000 a500007f 07fffc00 00000000 a500000f 0ffff801   <- +06000000, exact
```

Three things this rules out, and it is worth being explicit because each has a
different signature:

* **No reset desync.** That failure is a constant pointer offset, so it costs
  exactly ONE error and then a clean stream -- which is why the checkers resync
  on mismatch rather than latching. Seven presses, seven chances for the two
  halves to leave zero at different times, and neither counter ever left
  `00000000`. `ddl_rst_cross` held both halves until the far side acknowledged,
  every time.
* **No occupancy damage.** A FIFO that came back with its pointers offset would
  still move data, just with permanently wrong occupancy -- fewer or more usable
  slots, and a rate below the ceiling forever after. All 323 steady periods are
  bit-exact, so both pointer sets really did restart at zero.
* **The reporter recovered too.** Every line well formed, and the `a5` alignment
  marker intact through all seven. That marker exists *because* of the garbling
  this instrument produced on the board earlier in this work, when a character
  tick landing while the UART was busy dropped a character but still shifted the
  register. Seven bouncing presses and no field slid.

What the board cannot reach is the *short* pulse: the harness holds reset for 256
cycles of each domain's own clock, so a press is always a long assertion. The
single-cycle `rst_n` case -- the one a plain two-flop synchronizer cannot survive,
where the far side misses the pulse entirely and comes up desynchronised forever
-- is covered in `sim/tb_rst_cross.sv` at ratios 1:1, 1:8, 1:32, 1:128 and 8:1.

The build is clean on the checks that would invalidate it: the netlist grep finds
all sixteen expected registers -- the FIFO's pointers and both synchronizer pairs,
the handshake's `resetting`/`f_meta`/`f_sync`/`f_applied`/`ack_meta`/`ack_sync`,
and the two process cores' salts -- no `EX0205` substitution, and **no
inter-domain paths analysed**, so the tool is not quietly timing the crossing.
Timing closes at 109.6 MHz against 108 on clock B, which is a 1.5% margin and the
tightest in this directory; a faster clock B would need the report instrument
trimmed again before it needed anything from the crossing.

### Conclusion

All four decisions Phase 1 rests on are now measurements rather than arguments:

1. **Synchronizers are necessary.** B against C: 1.34 billion corrupted items
   becomes zero, changing only the salts.
2. **Synchronizers are sufficient.** C and E: 1.03 and 1.14 billion items, not one
   error between them.
3. **Depth 2 is not enough.** C sits at 44% of the achievable rate.
4. **Depth 8 is enough.** E recovers all of it -- and `cdc_fifo` is the module
   `src/ir_cdc.rs` was already specified to reproduce.

And the Phase 2 claim is a measurement too:

5. **What the compiler writes is what arm E measured.** Arm G reaches the exact
   ceiling at three clock ratios with zero errors in 156 billion items, taking
   one reset per module and deriving the far side itself, and it survives the
   reset button being pressed repeatedly at unplanned intervals.

Arm B was additionally run at pair 1, which settled the coherent-clock caveat: the
crossing fails at both ratios, and the alignment governs only the severity. Pair 2
(67.5 -> 108) is built and unflashed if a third point is ever wanted.

```bash
bash cdc-demo/build.sh b --pair 2 --flash
```

---

## What building this turned up

Findings that cost a build each and are worth not rediscovering.

**`ODIV_SEL` is not free, and the tool substitutes silently.** Asking for
ODIV_SEL 10 produces exactly one line in an otherwise green build:

```
WARN (EX0205) : Instance "u_rpll" 's parameter "ODIV_SEL" value invalid,
replaced by default value "8"
```

Exit status, bitstream, utilization and timing all stay green while the part runs
at a VCO nothing in the source names — which here would silently change the clock
ratio being measured. rPLL takes `ODIV_SEL` only from a fixed set; 8 is in it, 6
and 10 are not. This **contradicts `KAMASUTRA2G/rtl/board/k2g_pll.sv`**, which
records 10 and 6 as working and 8 as failing; its failures were `gw_sh exit 1`
with an empty log, a different mode, and its ODIV values do not survive here.
`build.sh` now treats EX0205 as a failed build rather than a warning.

**The instrument needed more care than the experiment.** The first build of the
control arm closed at 70 MHz against 81, with 42 violated endpoints — every one of
them in the reporting logic, not in a DUT. An instrument that misses timing
produces corrupted counters, and corrupted counters read exactly like the failure
being looked for. Five shapes had to change, each recorded at its site: a 224-bit
shift register instead of seven snapshots and a mux; the report tick, the warm-up
`run` gate, the UART's divisor compare and the `pos` decode all registered rather
than left as wide compares driving wide clock enables; a bitwise OR instead of a
running-max compare; and the checkers pipelined one cycle so the DUT's item mux no
longer feeds a 16-bit add in the same cycle. All four arms now meet timing on both
clocks with the netlist structure verified.

**The reference FIFO has an Fmax, and it argues for Phase 1's formulation.**
`cdc_fifo`'s empty path — `rbin` → increment → gray → compare → `rempty` — closes
at about 130 MHz on this part at the slow corner, so arm E missed timing at 135
MHz while arms B and C met it. That path is long *because* it is the lookahead
formulation, computing the flag from `*_next`. DDL's salt compares the current
pointers instead and has no such path, which is the formulation Phase 1 keeps —
and it is one cycle shorter in each direction of the credit loop as well. Every
clock pair here is now at or under 108 MHz so that all four arms are comparable.

**A DDL parser bug, found writing the DUT.** `@send(dst, @rcv(src))` is rejected
with `error: this operator takes two operands` pointing at line 1 column 1 — the
declaration, not the expression. Splitting it into `let v = @rcv(src)` then
`@send(dst, v)` compiles, which is what `gen_chk.ddl` does.

## Files

| | |
|---|---|
| `pipe_cdc.v` | slot 1, from `KAMASUTRA2G/docs/attic/pipe.sv` |
| `gen_chk.ddl` | slot 2, compiled to `gen_chk.v` by `build.sh` |
| `cdc_demo_top.v` | the board top; arms behind `` `ifdef `` |
| `cdc_fifo.v` | arm E, from `KAMASUTRA2G/rtl/k2g_cdc_fifo.sv` |
| *(none)* | arm G has no source file here -- `build.sh g` compiles `gen_chk.ddl` with `--async-export` and puts the result on the board unedited |
| `armg-reset-presses.log` | arm G's serial output, verbatim, across the seven button presses |
| `cdc_pll.v.in` | rPLL template — a template, not a source file; see the header |
| `cdc_uart.v` | from `KAMASUTRA2G/rtl/board/k2g_uart.sv` |
| `build.sh` | build, check, flash; from `KAMASUTRA2G/rtl/board/build.sh` |
| `sim/` | the throughput measurement |

Copied rather than referenced, each with a provenance header, so this runs
without `E:\Code\KAMASUTRA2G` on disk.

`build.sh` keeps four behaviours that file learned the hard way — capture `gw_sh`
through a pipe rather than a redirect, treat the fresh bitstream as the verdict
rather than the exit code, print warnings on success, and read timing from
`project_tr_content.html` — and adds two of its own: fail on a substituted PLL
divider, and **grep the post-synthesis netlist for every register the experiment
depends on**. Attributes are a request, not a guarantee, and a DUT the tool
optimised away would look exactly like a passing arm B.
