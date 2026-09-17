// `@send` and `@try_send` under conditions, in any stage of a `sequence`.
//
// Each output is sent to from one stage, at most once per item, under whatever
// condition the source writes. A stage waits for a sink only on an item that
// actually `@send`s to it, and never for a `@try_send`, which pushes only when
// there is room and answers whether it did. And whatever a stage waits for, the
// stages after it keep moving.
//
// These run the lowered module in the interpreter against producers and
// consumers that follow the salt protocol. Diagnostics are at the bottom.
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

// ---- a blocked send holds its own stage and nothing after it ---------------

/// `x` leaves in stage 0, `y` two stages later.
const EARLY_AND_LATE: &str = concat!(
    "sequence s (src: buffer in u8, x: buffer out u8, y: buffer out u8)\n",
    "  let a = @rcv(src)\n",
    "  @send(x, a)\n",
    "  |||\n",
    "  let b: u8 = a\n",
    "  |||\n",
    "  @send(y, b)\n",
);

#[test]
fn the_stages_after_a_blocked_send_keep_moving() {
    let mut c = Circuit::new(EARLY_AND_LATE, "s");
    let mut src = Producer::new("src", 8, 1..=60);
    let mut x = Consumer::new("x", 8);
    let mut y = Consumer::new("y", 8);
    let mut x_pushes = Steps::port("x_wsalt".to_string());
    let mut x_got = Vec::new();
    let mut y_got = Vec::new();

    // `y` is not drained, so the stages below stage 0 fill up behind it while
    // `x` keeps taking what stage 0 sends.
    for _ in 0..30 {
        src.offer(&mut c, true);
        if let Some(v) = x.take(&mut c, true) {
            x_got.push(v);
        }
        c.tick();
        x_pushes.watch(&c);
    }
    // Now `x` stops draining. Stage 0 blocks on it within two items; the items
    // that already left stage 0 are below it, and must all reach `y`.
    for _ in 0..80 {
        src.offer(&mut c, true);
        if let Some(v) = y.take(&mut c, true) {
            y_got.push(v);
        }
        c.tick();
        x_pushes.watch(&c);
    }
    let left_stage_0 = x_pushes.count as u128;
    assert!(left_stage_0 >= 6, "items were below stage 0 when it blocked: {left_stage_0}");
    assert_eq!(y_got, (1..=left_stage_0).collect::<Vec<_>>(), "every item past the blocked stage arrived");
    assert_eq!(c.out("x_wsalt"), step2(x.rsalt), "`x` is full");

    // And releasing `x` releases everything, in order, once each.
    for _ in 0..400 {
        src.offer(&mut c, true);
        if let Some(v) = x.take(&mut c, true) {
            x_got.push(v);
        }
        if let Some(v) = y.take(&mut c, true) {
            y_got.push(v);
        }
        c.tick();
    }
    assert_eq!(x_got, (1..=60).collect::<Vec<_>>());
    assert_eq!(y_got, (1..=60).collect::<Vec<_>>());
}

/// A salt two transfers ahead of `r`: a buffer of two, full.
fn step2(r: u128) -> u128 {
    common::salt::step(common::salt::step(r))
}

#[test]
fn a_conditional_send_waits_only_on_the_items_that_send() {
    // Odd items also go to `odd`, which is never drained. It fills with 1 and 3;
    // 2 and 4 do not send to it and must not be held by it; 5 does, and holds
    // stage 1 -- while everything already below stage 1 still reaches `all`.
    let src = concat!(
        "sequence s (src: buffer in u8, odd: buffer out u8, all: buffer out u8)\n",
        "  let a = @rcv(src)\n",
        "  |||\n",
        "  if a[0] == 1'd1 then\n",
        "    @send(odd, a)\n",
        "  |||\n",
        "  @send(all, a)\n",
    );
    let mut c = Circuit::new(src, "s");
    let mut feed = Producer::new("src", 8, 1..=20);
    let mut odd = Consumer::new("odd", 8);
    let mut all = Consumer::new("all", 8);
    let mut all_got = Vec::new();
    for _ in 0..200 {
        feed.offer(&mut c, true);
        if let Some(v) = all.take(&mut c, true) {
            all_got.push(v);
        }
        c.tick();
    }
    assert_eq!(all_got, vec![1, 2, 3, 4]);
    assert_eq!(c.out("odd_wsalt"), 3, "`odd` holds 1 and 3");

    let mut odd_got = Vec::new();
    for _ in 0..400 {
        feed.offer(&mut c, true);
        if let Some(v) = odd.take(&mut c, true) {
            odd_got.push(v);
        }
        if let Some(v) = all.take(&mut c, true) {
            all_got.push(v);
        }
        c.tick();
    }
    assert_eq!(all_got, (1..=20).collect::<Vec<_>>());
    assert_eq!(odd_got, (1..=20).filter(|v| v % 2 == 1).collect::<Vec<_>>());
}

#[test]
fn a_match_routes_each_item_to_at_most_one_output_under_back_pressure() {
    let src = concat!(
        "enum route_e\n",
        "  ToA(u8)\n",
        "  ToB(u8)\n",
        "  Nowhere(u8)\n",
        "sequence s (src: buffer in route_e, a: buffer out u8, b: buffer out u8)\n",
        "  let r = @rcv(src)\n",
        "  |||\n",
        "  match r\n",
        "    .ToA v =>\n",
        "      @send(a, v)\n",
        "    .ToB v =>\n",
        "      @send(b, v)\n",
        "    .Nowhere v =>\n",
        "      let ignored: u8 = v\n",
        "    _ =>\n",
        "      let unused: u8 = 8'd0\n",
    );
    // `{tag, payload}`, the tag in the high bits, in declaration order.
    let route = |i: u128| i % 3;
    let items: Vec<u128> = (1..=200).map(|i| (route(i) << 8) | i).collect();
    for seed in 1..=6u64 {
        let mut c = Circuit::new(src, "s");
        let mut feed = Producer::new("src", 10, items.iter().copied());
        let mut a = Consumer::new("a", 8);
        let mut b = Consumer::new("b", 8);
        let mut rng = Lcg(seed);
        let mut taken = Steps::new("src");
        let (mut a_got, mut b_got) = (Vec::new(), Vec::new());
        for _ in 0..3000 {
            let feed_willing = rng.chance(70);
            feed.offer(&mut c, feed_willing);
            let a_willing = rng.chance(40);
            if let Some(v) = a.take(&mut c, a_willing) {
                a_got.push(v);
            }
            let b_willing = rng.chance(40);
            if let Some(v) = b.take(&mut c, b_willing) {
                b_got.push(v);
            }
            assert!(c.assertions_ok());
            c.tick();
            taken.watch(&c);
        }
        assert_eq!(a_got, (1..=200).filter(|i| route(*i) == 0).collect::<Vec<_>>(), "seed {seed}");
        assert_eq!(b_got, (1..=200).filter(|i| route(*i) == 1).collect::<Vec<_>>(), "seed {seed}");
        assert_eq!(taken.count, 200, "seed {seed}: the items routed nowhere left too");
    }
}

#[test]
fn sends_to_one_output_in_disjoint_arms_send_one_value_per_item() {
    let src = concat!(
        "sequence s (src: buffer in u8, o: buffer out u8)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  if x[0] == 1'd1 then\n",
        "    @send(o, x)\n",
        "  else\n",
        "    @send(o, x + 8'd100)\n",
    );
    for seed in 1..=4u64 {
        let mut c = Circuit::new(src, "s");
        let mut feed = Producer::new("src", 8, 1..=100);
        let mut o = Consumer::new("o", 8);
        let mut rng = Lcg(seed);
        let mut got = Vec::new();
        for _ in 0..1500 {
            let feed_willing = rng.chance(60);
            feed.offer(&mut c, feed_willing);
            let o_willing = rng.chance(50);
            if let Some(v) = o.take(&mut c, o_willing) {
                got.push(v);
            }
            c.tick();
        }
        let want: Vec<u128> = (1..=100).map(|i| if i % 2 == 1 { i } else { i + 100 }).collect();
        assert_eq!(got, want, "seed {seed}");
    }
}

#[test]
fn a_conditional_early_send_still_joins_downstream() {
    // The join from `probe_regressions.rs`, with the early send under a
    // condition every item meets: a guard on the room must not cost liveness.
    let src = concat!(
        "sequence s (src: buffer in u8, x: buffer out u8, y: buffer out u8)\n",
        "  let a = @rcv(src)\n",
        "  if a != 8'd0 then\n",
        "    @send(x, a)\n",
        "  |||\n",
        "  let b: u8 = a + 8'd1\n",
        "  |||\n",
        "  @send(y, b + 8'd1)\n",
    );
    let mut c = Circuit::new(src, "s");
    let mut feed = Producer::new("src", 8, 1..=40);
    let mut x = Consumer::new("x", 8);
    let mut y = Consumer::new("y", 8);
    let mut got = Vec::new();
    for cycle in 0..800 {
        feed.offer(&mut c, true);
        let stalling = cycle % 13 >= 9;
        let both = c.out("x_wsalt") != x.rsalt && c.out("y_wsalt") != y.rsalt;
        if !stalling && both {
            let a = x.take(&mut c, true).expect("offered");
            let b = y.take(&mut c, true).expect("offered");
            got.push((a, b));
        }
        c.tick();
    }
    assert_eq!(got, (1..=40).map(|i| (i, i + 2)).collect::<Vec<_>>());
}

// ---- `@try_send` -----------------------------------------------------------

/// `side` is offered every item from stage 1; `log` records the answer.
const OFFER: &str = concat!(
    "sequence s (src: buffer in u8, side: buffer out u8, log: buffer out u16)\n",
    "  let x = @rcv(src)\n",
    "  |||\n",
    "  let ok = @try_send(side, x)\n",
    "  let f: u8 = if ok then 8'd1 else 8'd0\n",
    "  |||\n",
    "  @send(log, {f, x})\n",
);

#[test]
fn a_try_send_never_holds_its_stage() {
    let mut c = Circuit::new(OFFER, "s");
    let mut feed = Producer::new("src", 8, 1..=30);
    let mut log = Consumer::new("log", 16);
    let mut got = Vec::new();
    for _ in 0..300 {
        feed.offer(&mut c, true);
        if let Some(v) = log.take(&mut c, true) {
            got.push(v);
        }
        c.tick();
    }
    assert_eq!(got.iter().map(|v| v & 0xff).collect::<Vec<_>>(), (1..=30).collect::<Vec<_>>());
    let delivered: Vec<u128> = got.iter().filter(|v| **v >> 8 == 1).map(|v| v & 0xff).collect();
    assert_eq!(delivered, vec![1, 2], "`side` had room for two, and said so for exactly those");
    assert_eq!(c.out("side_wsalt"), 3);
}

#[test]
fn a_try_send_answers_exactly_whether_it_delivered() {
    for seed in 1..=8u64 {
        let mut c = Circuit::new(OFFER, "s");
        let mut feed = Producer::new("src", 8, 1..=200);
        let mut side = Consumer::new("side", 8);
        let mut log = Consumer::new("log", 16);
        let mut rng = Lcg(seed);
        let (mut side_got, mut got) = (Vec::new(), Vec::new());
        for _ in 0..2000 {
            let feed_willing = rng.chance(70);
            feed.offer(&mut c, feed_willing);
            let side_willing = rng.chance(30);
            if let Some(v) = side.take(&mut c, side_willing) {
                side_got.push(v);
            }
            let log_willing = rng.chance(60);
            if let Some(v) = log.take(&mut c, log_willing) {
                got.push(v);
            }
            c.tick();
        }
        // What `side` still holds counts as delivered too.
        for _ in 0..4 {
            if let Some(v) = side.take(&mut c, true) {
                side_got.push(v);
            }
            c.tick();
        }
        assert_eq!(got.iter().map(|v| v & 0xff).collect::<Vec<_>>(), (1..=200).collect::<Vec<_>>(), "seed {seed}");
        let delivered: Vec<u128> = got.iter().filter(|v| **v >> 8 == 1).map(|v| v & 0xff).collect();
        assert_eq!(side_got, delivered, "seed {seed}: `ok` is exactly the items `side` received");
        assert!(delivered.len() > 20 && delivered.len() < 200, "seed {seed}: the test exercised both answers");
    }
}

#[test]
fn an_offer_and_a_blocking_send_to_one_output_share_it_in_disjoint_arms() {
    // Odd items must reach `o` and wait for it; even items are offered and
    // skip it when it is full. `log` says which happened to each.
    let src = concat!(
        "sequence s (src: buffer in u8, o: buffer out u8, log: buffer out u16)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  var f: u8 = 8'd0\n",
        "  if x[0] == 1'd1 then\n",
        "    @send(o, x)\n",
        "    f = 8'd1\n",
        "  else\n",
        "    let ok = @try_send(o, x)\n",
        "    if ok then\n",
        "      f = 8'd2\n",
        "  |||\n",
        "  @send(log, {f, x})\n",
    );
    for seed in 1..=6u64 {
        let mut c = Circuit::new(src, "s");
        let mut feed = Producer::new("src", 8, 1..=150);
        let mut o = Consumer::new("o", 8);
        let mut log = Consumer::new("log", 16);
        let mut rng = Lcg(seed);
        let (mut o_got, mut got) = (Vec::new(), Vec::new());
        for _ in 0..3000 {
            let feed_willing = rng.chance(70);
            feed.offer(&mut c, feed_willing);
            let o_willing = rng.chance(35);
            if let Some(v) = o.take(&mut c, o_willing) {
                o_got.push(v);
            }
            let log_willing = rng.chance(70);
            if let Some(v) = log.take(&mut c, log_willing) {
                got.push(v);
            }
            c.tick();
        }
        assert_eq!(got.iter().map(|v| v & 0xff).collect::<Vec<_>>(), (1..=150).collect::<Vec<_>>(), "seed {seed}");
        for v in &got {
            let (f, x) = (v >> 8, v & 0xff);
            if x % 2 == 1 {
                assert_eq!(f, 1, "seed {seed}: an odd item always sends");
            } else {
                assert!(f == 0 || f == 2, "seed {seed}: an even item only offers");
            }
        }
        let delivered: Vec<u128> = got.iter().filter(|v| **v >> 8 != 0).map(|v| v & 0xff).collect();
        assert_eq!(o_got, delivered, "seed {seed}: `o` received exactly the sends and the accepted offers, in order");
        assert!(got.iter().any(|v| v >> 8 == 0), "seed {seed}: some offer was declined");
    }
}

#[test]
fn a_sequence_of_only_offers_never_stalls() {
    let src = concat!(
        "sequence s (src: buffer in u8, o: buffer out u8)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let ok = @try_send(o, x)\n",
    );
    let v = compile(src);
    assert!(!v.contains("wire shift"), "nothing waits, so nothing shifts: {}", v);

    let mut c = Circuit::new(src, "s");
    let mut feed = Producer::new("src", 8, 1..=50);
    let mut taken = Steps::new("src");
    for _ in 0..200 {
        feed.offer(&mut c, true);
        c.tick();
        taken.watch(&c);
    }
    assert_eq!(taken.count, 50, "a full sink never held the head");
    let mut o = Consumer::new("o", 8);
    let mut got = Vec::new();
    for _ in 0..4 {
        if let Some(v) = o.take(&mut c, true) {
            got.push(v);
        }
        c.tick();
    }
    assert_eq!(got, vec![1, 2], "the two that fit, and nothing overwritten");
}

// ---- the netlist -----------------------------------------------------------

#[test]
fn an_unconditional_send_adds_no_gate() {
    let v = compile(EARLY_AND_LATE);
    assert!(!v.contains("_clear"), "{}", v);
    assert!(!v.contains("x_push"), "{}", v);
    assert!(v.contains("wire shift2 = !y_full;"), "{}", v);
}

#[test]
fn a_conditional_send_guards_its_room_and_its_push() {
    let v = compile(concat!(
        "sequence s (src: buffer in u8, odd: buffer out u8, all: buffer out u8)\n",
        "  let a = @rcv(src)\n",
        "  |||\n",
        "  if a[0] == 1'd1 then\n",
        "    @send(odd, a)\n",
        "  |||\n",
        "  @send(all, a)\n",
    ));
    let clear = v.lines().find(|l| l.contains("wire odd_clear = ")).unwrap_or_else(|| panic!("{}", v));
    assert!(clear.contains("odd_room |"), "{}", clear);
    let push = v.lines().find(|l| l.contains("wire odd_push = ")).unwrap_or_else(|| panic!("{}", v));
    assert!(push.contains("push1"), "{}", push);
    let shift1 = v.lines().find(|l| l.contains("wire shift1 = ")).unwrap_or_else(|| panic!("{}", v));
    assert!(shift1.contains("odd_clear"), "{}", shift1);
}

#[test]
fn an_offer_is_not_waited_for_and_pushes_only_into_room() {
    let v = compile(OFFER);
    for line in v.lines().filter(|l| l.contains("wire shift")) {
        assert!(!line.contains("side"), "a shift waits on an offer: {}", line);
    }
    let push = v.lines().find(|l| l.contains("wire side_push = ")).unwrap_or_else(|| panic!("{}", v));
    assert!(push.contains("side_room"), "{}", push);
}

// ---- the graph -------------------------------------------------------------

// ---- diagnostics -----------------------------------------------------------

fn seq(body: &str) -> String {
    format!("sequence s (src: buffer in u8, o: buffer out u8)\n{body}")
}

#[test]
fn a_send_on_every_item_beside_a_conditional_one_is_refused() {
    let text = compile_err(&seq(concat!(
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  if x == 8'd1 then\n",
        "    @send(o, x)\n",
        "  @send(o, x)\n",
    )));
    assert!(text.contains("`o` is sent to more than once for one item"), "{}", text);
}

#[test]
fn two_offers_to_one_output_on_overlapping_paths_are_refused() {
    let text = compile_err(&seq(concat!(
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let a = @try_send(o, x)\n",
        "  if x == 8'd1 then\n",
        "    let b = @try_send(o, x)\n",
    )));
    assert!(text.contains("`o` is sent to more than once for one item"), "{}", text);
}

#[test]
fn an_offer_from_two_stages_is_refused_and_both_are_named() {
    let text = compile_err(&seq(concat!(
        "  let x = @rcv(src)\n",
        "  let a = @try_send(o, x)\n",
        "  |||\n",
        "  if x == 8'd1 then\n",
        "    @send(o, x)\n",
    )));
    assert!(text.contains("`o` is sent to in stage 0 and stage 1"), "{}", text);
}

#[test]
fn a_send_or_offer_into_an_in_pipe_is_refused() {
    for body in [
        "  let x = @rcv(src)\n  if x == 8'd1 then\n    @send(src, x)\n  |||\n  @send(o, x)\n",
        "  let x = @rcv(src)\n  let ok = @try_send(src, x)\n  |||\n  @send(o, x)\n",
    ] {
        let text = compile_err(&seq(body));
        assert!(text.contains("`src` is an `in` pipe; it cannot be sent to"), "{}\n{}", body, text);
    }
}

#[test]
fn a_send_of_the_wrong_type_in_an_arm_is_refused() {
    let text = compile_err(&seq(concat!(
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  if x == 8'd1 then\n",
        "    @send(o, @zext(x, 16))\n",
    )));
    assert!(text.contains("`o` carries `u8` but `u16` was sent"), "{}", text);
}

#[test]
fn a_blocking_send_inside_an_expression_is_still_refused() {
    let text = compile_err(&seq(concat!(
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let y = @send(o, x)\n",
    )));
    assert!(text.contains("a blocking `@send` has to be a statement of its own"), "{}", text);
}

#[test]
fn an_output_only_ever_offered_to_counts_as_sent() {
    compile(&seq("  let x = @rcv(src)\n  |||\n  if x == 8'd1 then\n    let ok = @try_send(o, x)\n"));
}
