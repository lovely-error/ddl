// `@try_rcv`, `@peek` and `@drop` in any stage of a `sequence`.
//
// An operation in stage k acts for the item stage k holds. It LOOKS on every
// cycle and TAKES only on a cycle that stage is live and moving, so:
//
//   * a bubble in stage k takes nothing,
//   * a held stage takes nothing twice while it waits,
//   * and an input is consumed exactly once for each `ok`, in order.
//
// These run the lowered module in the interpreter against producers and
// consumers that follow the salt protocol, under irregular back-pressure, and
// check those three things end to end. The diagnostics that go with the
// feature -- one stage per pipe, a pipe only peeked at, a head with nothing
// entering it -- are at the bottom.
mod common;

use common::Circuit;
use common::salt::{Consumer, Lcg, Producer, Steps};
use ddl::diag::SourceMap;
use ddl::driver::compile_to_verilog;
use ddl::verilog::EmitOptions;

fn compile(src: &str) -> String {
    let map = SourceMap::new("t.ddl", src);
    match compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(v) => v,
        Err(diags) => panic!("compile failed:\n{}", map.render_all(&diags)),
    }
}

fn compile_err(src: &str) -> String {
    let map = SourceMap::new("t.ddl", src);
    match compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(v) => panic!("expected failure, got:\n{}", v),
        Err(diags) => map.render_all(&diags),
    }
}

/// `x` through three stages, sampling the sideband `b` in the middle one.
const SIDEBAND: &str = concat!(
    "sequence s (src: buffer in u8, b: buffer in u8, o: buffer out u24)\n",
    "  let x = @rcv(src)\n",
    "  |||\n",
    "  let (y, ok) = @try_rcv(b)\n",
    "  var t: u8 = 8'd0\n",
    "  var f: u8 = 8'd0\n",
    "  if ok then\n",
    "    t = y\n",
    "    f = 8'd1\n",
    "  |||\n",
    "  @send(o, {f, t, x})\n",
);

#[test]
fn a_sideband_is_sampled_once_per_item_in_order_under_back_pressure() {
    // Everything at once: a steady stream on `src`, a sparser one on `b`, and a
    // consumer that stalls at random. Every `x` must arrive once and in order;
    // the items marked `ok` must be exactly `b`'s items, in order, none twice
    // and none skipped; and `b` must have been stepped exactly once for each.
    for seed in 1..=8u64 {
        let mut c = Circuit::new(SIDEBAND, "s");
        // `u8` items: 1..=250 and 101..=160, so none of them wraps.
        let n = 250u128;
        let m = 60u128;
        let mut src = Producer::new("src", 8, 1..=n);
        let mut b = Producer::new("b", 8, 101..=100 + m);
        let mut o = Consumer::new("o", 24);
        let mut rng = Lcg(seed);
        let mut b_steps = Steps::new("b");
        let mut got = Vec::new();
        for _ in 0..3000 {
            let src_willing = rng.chance(70);
            src.offer(&mut c, src_willing);
            let b_willing = rng.chance(35);
            b.offer(&mut c, b_willing);
            let o_willing = rng.chance(60);
            if let Some(item) = o.take(&mut c, o_willing) {
                got.push(item);
            }
            assert!(c.assertions_ok());
            c.tick();
            b_steps.watch(&c);
        }
        let xs: Vec<u128> = got.iter().map(|v| v & 0xff).collect();
        assert_eq!(xs, (1..=n).collect::<Vec<_>>(), "seed {seed}: every item, once, in order");
        let sampled: Vec<u128> = got.iter().filter(|v| (*v >> 16) == 1).map(|v| (v >> 8) & 0xff).collect();
        assert_eq!(sampled, (101..=100 + m).collect::<Vec<_>>(), "seed {seed}: every sideband item, once, in order");
        let unmarked_are_zero = got.iter().filter(|v| (*v >> 16) == 0).all(|v| (v >> 8) & 0xff == 0);
        assert!(unmarked_are_zero, "seed {seed}: `t` is only written under `ok`");
        assert_eq!(b_steps.count, m as usize, "seed {seed}: one step per sampled item");
    }
}

#[test]
fn a_bubble_in_the_sampling_stage_takes_nothing() {
    // `b` offers and nothing enters the pipeline. Every stage shifts -- the
    // sink has room -- and every stage is empty, so `b` must be left alone.
    let mut c = Circuit::new(SIDEBAND, "s");
    let mut b = Producer::new("b", 8, [7]);
    for _ in 0..40 {
        b.offer(&mut c, true);
        c.tick();
        assert_eq!(c.out("b_rsalt"), 0, "a bubble took from `b`");
        assert_eq!(c.out("o_wsalt"), 0, "a bubble produced");
    }
    // One item through, and exactly that one takes it.
    let mut src = Producer::new("src", 8, [9]);
    let mut b_steps = Steps::new("b");
    for _ in 0..10 {
        src.offer(&mut c, true);
        c.tick();
        b_steps.watch(&c);
    }
    assert_eq!(b_steps.count, 1);
    assert_eq!(c.out("o_data") & 0xff_ffff, (1 << 16) | (7 << 8) | 9);
}

#[test]
fn a_held_sampling_stage_takes_nothing_until_it_moves() {
    // The sink is never drained, so the pipeline fills and stage 1 ends up
    // live and held. `b` offers only once it is: the stage must not take while
    // it waits, and must take exactly once on the cycle it moves.
    let mut c = Circuit::new(SIDEBAND, "s");
    let mut src = Producer::new("src", 8, 1..=6);
    for _ in 0..30 {
        src.offer(&mut c, true);
        c.tick();
    }
    assert_eq!(c.out("o_wsalt"), 3, "the sink is full");
    assert_eq!(c.out("b_rsalt"), 0);

    let mut b = Producer::new("b", 8, [55, 66]);
    let mut b_steps = Steps::new("b");
    for _ in 0..40 {
        b.offer(&mut c, true);
        c.tick();
        b_steps.watch(&c);
    }
    assert_eq!(b_steps.count, 0, "a held stage took from `b`");

    // Room for one item: stage 2's item leaves, stage 1's moves down and takes
    // `55` as it goes, and stage 0's moves into stage 1 -- which is held again
    // and must not take `66`.
    let mut o = Consumer::new("o", 24);
    assert_eq!(o.take(&mut c, true), Some(1));
    for _ in 0..40 {
        b.offer(&mut c, true);
        c.tick();
        b_steps.watch(&c);
    }
    assert_eq!(b_steps.count, 1, "exactly one item moved past the sampling stage");
    let mut rest = Vec::new();
    for _ in 0..40 {
        if let Some(v) = o.take(&mut c, true) {
            rest.push(v);
        }
        c.tick();
    }
    // Items 2 and 3 were already past stage 1 when `b` started offering; item
    // 4 was the one held there; 5 and 6 take `66` and nothing.
    assert_eq!(
        rest,
        vec![2, 3, (1 << 16) | (55 << 8) | 4, (1 << 16) | (66 << 8) | 5, 6],
    );
}

#[test]
fn a_peek_decides_whether_a_later_stage_drops() {
    // Stage 2 looks at `b` and drops its head only when it matches the item
    // passing through. `x` climbs, so `3` and the first `7` are dropped, and
    // the second `7` sits at the head of `b` for good -- blocking `20`.
    let src = concat!(
        "sequence s (src: buffer in u8, b: buffer in u8, o: buffer out u16)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let d: u8 = x\n",
        "  |||\n",
        "  let (y, here) = @peek(b)\n",
        "  let hit: u1 = here & (y == d)\n",
        "  var h: u8 = 8'd0\n",
        "  if hit then\n",
        "    @drop(b)\n",
        "    h = 8'd1\n",
        "  @send(o, {h, d})\n",
    );
    let mut c = Circuit::new(src, "s");
    let mut feed = Producer::new("src", 8, 1..=40);
    let mut b = Producer::new("b", 8, [3, 7, 7, 20]);
    let mut o = Consumer::new("o", 16);
    let mut rng = Lcg(3);
    let mut b_steps = Steps::new("b");
    let mut got = Vec::new();
    for _ in 0..600 {
        b.offer(&mut c, true);
        let feed_willing = rng.chance(50);
        feed.offer(&mut c, feed_willing);
        let o_willing = rng.chance(50);
        if let Some(v) = o.take(&mut c, o_willing) {
            got.push(v);
        }
        c.tick();
        b_steps.watch(&c);
    }
    assert_eq!(got.iter().map(|v| v & 0xff).collect::<Vec<_>>(), (1..=40).collect::<Vec<_>>());
    let hits: Vec<u128> = got.iter().filter(|v| (*v >> 8) == 1).map(|v| v & 0xff).collect();
    assert_eq!(hits, vec![3, 7]);
    assert_eq!(b_steps.count, 2, "a peek alone never takes");
}

/// Drives a sequence whose only input is `a` and whose output is `o`, both
/// `u8`, with an irregular producer and consumer, and answers what came out.
fn run_single(src: &str, items: &[u128], cycles: usize) -> Vec<u128> {
    let mut c = Circuit::new(src, "s");
    let mut a = Producer::new("a", 8, items.iter().copied());
    let mut o = Consumer::new("o", 8);
    let mut rng = Lcg(11);
    let mut got = Vec::new();
    for _ in 0..cycles {
        let a_willing = rng.chance(40);
        a.offer(&mut c, a_willing);
        let o_willing = rng.chance(55);
        if let Some(v) = o.take(&mut c, o_willing) {
            got.push(v);
        }
        c.tick();
    }
    got
}

#[test]
fn a_head_of_peek_and_drop_behaves_as_a_head_of_try_rcv() {
    // Liveness widened to every input stage 0 touches: a head that looks and
    // then takes is the same pipeline as one that takes, item for item.
    let peeking = concat!(
        "sequence s (a: buffer in u8, o: buffer out u8)\n",
        "  let (x, here) = @peek(a)\n",
        "  @drop(a)\n",
        "  |||\n",
        "  @send(o, x + 8'd1)\n",
    );
    let taking = concat!(
        "sequence s (a: buffer in u8, o: buffer out u8)\n",
        "  let (x, ok) = @try_rcv(a)\n",
        "  |||\n",
        "  @send(o, x + 8'd1)\n",
    );
    let items: Vec<u128> = (10..60).collect();
    let want: Vec<u128> = items.iter().map(|v| v + 1).collect();
    assert_eq!(run_single(taking, &items, 800), want);
    assert_eq!(run_single(peeking, &items, 800), want);
}

#[test]
fn a_head_that_declines_to_drop_emits_the_same_item_again() {
    // Pinned, because it is deliberate: a head built on `@peek` is live while
    // its pipe offers, and an item it does not take is still offered on the
    // next cycle. A process written the same way would do the same.
    let src = concat!(
        "sequence s (a: buffer in u8, o: buffer out u8)\n",
        "  let (x, here) = @peek(a)\n",
        "  if x != 8'd0 then\n",
        "    @drop(a)\n",
        "  |||\n",
        "  @send(o, x)\n",
    );
    let mut c = Circuit::new(src, "s");
    let mut a = Producer::new("a", 8, [5, 0, 9]);
    let mut o = Consumer::new("o", 8);
    let mut a_steps = Steps::new("a");
    let mut got = Vec::new();
    for _ in 0..40 {
        a.offer(&mut c, true);
        if let Some(v) = o.take(&mut c, true) {
            got.push(v);
        }
        c.tick();
        a_steps.watch(&c);
    }
    assert_eq!(got[0], 5);
    assert!(got.len() > 10 && got[1..].iter().all(|v| *v == 0), "{got:?}");
    assert_eq!(a_steps.count, 1, "only the `5` was ever taken");
}

#[test]
fn a_receive_and_a_drop_in_disjoint_arms_take_once_per_item() {
    // Odd items receive from `b`, even items throw its head away. Either way
    // one entry goes per item while `b` has one, so the entries each item
    // accounts for are `b`'s, consecutively.
    let src = concat!(
        "sequence s (src: buffer in u8, b: buffer in u8, o: buffer out u24)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  var t: u8 = 8'd0\n",
        "  var f: u8 = 8'd0\n",
        "  if x[0] == 1'd1 then\n",
        "    let (y, ok) = @try_rcv(b)\n",
        "    if ok then\n",
        "      t = y\n",
        "      f = 8'd1\n",
        "  else\n",
        "    let took = @drop(b)\n",
        "    if took then\n",
        "      f = 8'd2\n",
        "  |||\n",
        "  @send(o, {f, t, x})\n",
    );
    for seed in 1..=4u64 {
        let mut c = Circuit::new(src, "s");
        let mut feed = Producer::new("src", 8, 1..=200);
        let mut b = Producer::new("b", 8, 1..=80);
        let mut o = Consumer::new("o", 24);
        let mut rng = Lcg(seed);
        let mut b_steps = Steps::new("b");
        let mut got = Vec::new();
        for _ in 0..2500 {
            let feed_willing = rng.chance(60);
            feed.offer(&mut c, feed_willing);
            let b_willing = rng.chance(30);
            b.offer(&mut c, b_willing);
            let o_willing = rng.chance(60);
            if let Some(v) = o.take(&mut c, o_willing) {
                got.push(v);
            }
            c.tick();
            b_steps.watch(&c);
        }
        assert_eq!(got.iter().map(|v| v & 0xff).collect::<Vec<_>>(), (1..=200).collect::<Vec<_>>());
        let accounted: Vec<&u128> = got.iter().filter(|v| (*v >> 16) != 0).collect();
        assert_eq!(accounted.len(), 80, "seed {seed}: every entry of `b` went somewhere");
        assert_eq!(b_steps.count, 80, "seed {seed}");
        for (position, v) in accounted.iter().enumerate() {
            let x = *v & 0xff;
            let kind = *v >> 16;
            let odd = x & 1 == 1;
            assert_eq!(kind, if odd { 1 } else { 2 }, "seed {seed}: the arm follows `x`");
            if odd {
                assert_eq!((*v >> 8) & 0xff, position as u128 + 1, "seed {seed}: received out of order");
            }
        }
    }
}

// ---- the netlist -----------------------------------------------------------

#[test]
fn a_later_stage_takes_on_its_own_validity_and_shift() {
    let v = compile(SIDEBAND);
    assert!(v.contains("wire b_take = (v0 & shift1) & b_present;"), "{}", v);
    // The head is not paced by an input a later stage owns.
    assert!(v.contains("wire take = src_present & shift0;"), "{}", v);
}

#[test]
fn a_conditional_take_is_narrowed_by_its_condition() {
    let v = compile(concat!(
        "sequence s (src: buffer in u8, b: buffer in u8, o: buffer out u8)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  var t: u8 = x\n",
        "  if x == 8'd4 then\n",
        "    let (y, ok) = @try_rcv(b)\n",
        "    t = y\n",
        "  @send(o, t)\n",
    ));
    let take = v.lines().find(|l| l.contains("wire b_take = ")).unwrap_or_else(|| panic!("{}", v));
    assert!(take.contains("v0 & shift1"), "{}", take);
    assert!(take.contains("b_present"), "{}", take);
    // Three terms: the stage moving, the pipe offering, and the arm.
    assert!(take.contains("(b_present & "), "the arm's condition is in the take: {}", take);
}

#[test]
fn a_peek_reuses_the_present_bit_rather_than_comparing_again() {
    let v = compile(concat!(
        "sequence s (src: buffer in u8, b: buffer in u8, o: buffer out u8)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let (y, here) = @peek(b)\n",
        "  if here then\n",
        "    @drop(b)\n",
        "  @send(o, x)\n",
    ));
    assert_eq!(v.matches("wire b_empty").count(), 1, "{}", v);
}

// ---- the graph -------------------------------------------------------------

#[test]
fn a_loop_closed_through_a_later_stage_sample_is_live() {
    // The feedback is sampled in stage 1 without waiting, so the loop has no
    // first item to be missing -- the same reason a head `@try_rcv` breaks it.
    let v = compile(concat!(
        "sequence acc (x: buffer in u16, fb: buffer in u16, o: buffer out u16, fbo: buffer out u16)\n",
        "  let a = @rcv(x)\n",
        "  |||\n",
        "  let (b, ok) = @try_rcv(fb)\n",
        "  var s: u16 = a\n",
        "  if ok then\n",
        "    s = a + b\n",
        "  @send(o, s)\n",
        "  @send(fbo, s)\n",
        "sequence hold (i: buffer in u16, o: buffer out u16)\n",
        "  let a = @rcv(i)\n",
        "  |||\n",
        "  @send(o, a)\n",
        "graph accum (src: buffer in u16, dst: buffer out u16)\n",
        "  let fwd: buffer u16\n",
        "  let back: buffer u16\n",
        "  acc(src, back, dst, fwd)\n",
        "  hold(fwd, back)\n",
    ));
    assert!(v.contains("module accum"), "{}", v);
}

// ---- diagnostics -----------------------------------------------------------

fn seq(params: &str, body: &str) -> String {
    format!("sequence s ({params})\n{body}")
}

const TWO_IN: &str = "src: buffer in u8, b: buffer in u8, o: buffer out u8";

#[test]
fn an_input_touched_in_two_stages_is_refused_and_both_are_named() {
    let text = compile_err(&seq(TWO_IN, concat!(
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let (y, here) = @peek(b)\n",
        "  |||\n",
        "  @drop(b)\n",
        "  @send(o, x)\n",
    )));
    assert!(text.contains("`b` is received from in stage 1 and stage 2"), "{}", text);
    assert!(text.contains("put every operation on this pipe in one stage"), "{}", text);
}

#[test]
fn a_blocking_input_sampled_again_below_the_head_is_refused() {
    let text = compile_err(&seq(TWO_IN, concat!(
        "  let x = @rcv(src)\n",
        "  let z = @rcv(b)\n",
        "  |||\n",
        "  let (y, ok) = @try_rcv(src)\n",
        "  @send(o, x)\n",
    )));
    assert!(text.contains("`src` is received from in stage 0 and stage 1"), "{}", text);
}

#[test]
fn a_blocking_receive_below_the_head_is_still_refused() {
    let text = compile_err(&seq(TWO_IN, concat!(
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let y = @rcv(b)\n",
        "  @send(o, x)\n",
    )));
    assert!(text.contains("only the first stage of a sequence may block on a read"), "{}", text);
    assert!(text.contains("use `@try_rcv` there"), "{}", text);
}

#[test]
fn an_input_only_peeked_at_never_drains_and_is_refused() {
    let text = compile_err(&seq(TWO_IN, concat!(
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let (y, here) = @peek(b)\n",
        "  @send(o, x)\n",
    )));
    assert!(text.contains("`b` is never received from"), "{}", text);
    assert!(text.contains("a `@peek` looks and takes nothing"), "{}", text);
}

#[test]
fn an_input_never_named_is_refused() {
    let text = compile_err(&seq(TWO_IN, "  let x = @rcv(src)\n  |||\n  @send(o, x)\n"));
    assert!(text.contains("`b` is never received from"), "{}", text);
}

#[test]
fn a_head_that_takes_from_nothing_is_refused() {
    let text = compile_err(&seq("b: buffer in u8, o: buffer out u8", concat!(
        "  let k: u8 = 8'd1\n",
        "  |||\n",
        "  let (y, ok) = @try_rcv(b)\n",
        "  @send(o, y + k)\n",
    )));
    assert!(text.contains("nothing enters this sequence"), "{}", text);
}

#[test]
fn a_blocking_receive_and_a_take_of_the_same_pipe_in_the_head_are_refused_either_way() {
    for body in [
        "  let x = @rcv(src)\n  @drop(src)\n  let z = @rcv(b)\n  |||\n  @send(o, x)\n",
        "  @drop(src)\n  let x = @rcv(src)\n  let z = @rcv(b)\n  |||\n  @send(o, x)\n",
        "  let x = @rcv(src)\n  let (y, ok) = @try_rcv(src)\n  let z = @rcv(b)\n  |||\n  @send(o, x)\n",
    ] {
        let text = compile_err(&seq(TWO_IN, body));
        assert!(text.contains("`src` is received from more than once in one cycle"), "{}\n{}", body, text);
    }
}

#[test]
fn a_peek_beside_a_blocking_receive_is_allowed() {
    // It spends nothing, so there is no second take to refuse.
    compile(&seq(TWO_IN, concat!(
        "  let x = @rcv(src)\n",
        "  let (y, here) = @peek(src)\n",
        "  let z = @rcv(b)\n",
        "  |||\n",
        "  @send(o, x + y + z)\n",
    )));
}

#[test]
fn two_unconditional_takes_in_one_stage_are_refused() {
    let text = compile_err(&seq(TWO_IN, concat!(
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  @drop(b)\n",
        "  let (y, ok) = @try_rcv(b)\n",
        "  @send(o, x)\n",
    )));
    assert!(text.contains("`b` is received from more than once in one cycle"), "{}", text);
}

#[test]
fn a_try_send_beside_a_send_to_the_same_output_is_refused() {
    let text = compile_err(&seq(TWO_IN, concat!(
        "  let x = @rcv(src)\n",
        "  let z = @rcv(b)\n",
        "  |||\n",
        "  let sent = @try_send(o, x)\n",
        "  @send(o, x)\n",
    )));
    assert!(text.contains("`o` is sent to more than once for one item"), "{}", text);
}

#[test]
fn nonblocking_operations_on_an_out_pipe_or_a_stranger_are_refused() {
    for (body, message) in [
        ("  let x = @rcv(src)\n  let z = @rcv(b)\n  |||\n  let (y, h) = @peek(o)\n  @send(o, x)\n", "`o` is an `out` pipe; it cannot be peeked at"),
        ("  let x = @rcv(src)\n  let z = @rcv(b)\n  |||\n  @drop(o)\n  @send(o, x)\n", "`o` is an `out` pipe; there is nothing on it to drop"),
        ("  let x = @rcv(src)\n  let z = @rcv(b)\n  |||\n  let (y, ok) = @try_rcv(q)\n  @send(o, x)\n", "`q` is not a pipe of this sequence"),
    ] {
        let text = compile_err(&seq(TWO_IN, body));
        assert!(text.contains(message), "{}\n{}", body, text);
    }
}
