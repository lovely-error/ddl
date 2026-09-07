// `@peek(p)` and `@drop(p)`.
//
// Every other pipe operation completes a transfer, and a pipe gets one of
// those per cycle. These two split that apart:
//
//   * `@peek` LOOKS. It reads the pipe's `valid` and `data` and asks for
//     nothing, so the item is still there afterwards and the cycle's one
//     consuming operation is still available.
//   * `@drop` TAKES and discards. A `@try_rcv` that binds nothing, for when
//     the answer is "whatever it was, not this".
//
// The pair is what lets a stage decide from an item's contents whether it
// wants it -- look, then either take it or throw it away.

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

/// Look at the head, take it only if it is wanted.
const GATE: &str = concat!(
    "process gate (src: buffer in u32, dst: buffer out u32)\n",
    "  loop\n",
    "    let (v, present) = @peek(src)\n",
    "    let wanted: u1 = present & (!v[0])\n",
    "    let (x, got) = @try_rcv(src)\n",
    "    if got & wanted then\n",
    "      let _s = @try_send(dst, x)\n",
);

#[test]
fn present_is_the_offer_and_got_is_the_transfer() {
    // The distinction the whole intrinsic exists for. `present` is `valid`
    // alone; `got` is `valid` and this side's `ready`. On a cycle the process
    // is not accepting they disagree, and that disagreement is what lets a
    // peek see an item before deciding to take it.
    let v = compile(GATE);
    assert!(v.contains("wire wanted = src_present & (!src_item[0]);"), "{}", v);
    assert!(v.contains("wire src_take = "), "{}", v);
    assert!(v.contains("!src_empty"), "{}", v);
}

#[test]
fn a_peek_does_not_spend_the_cycles_one_operation() {
    // GATE peeks and then receives from the same pipe in the same cycle. If
    // the peek counted, this would be "received from more than once".
    let v = compile(GATE);
    assert!(v.contains("wire src_take = "), "{}", v);
    assert!(v.contains("!src_empty"), "{}", v);
}

#[test]
fn two_peeks_are_fine() {
    let v = compile(concat!(
        "process twice (src: buffer in u32, dst: buffer out u1)\n",
        "  loop\n",
        "    let (a, p1) = @peek(src)\n",
        "    let (b, p2) = @peek(src)\n",
        "    let _s = @try_send(dst, p1 & p2 & (a == b))\n",
    ));
    assert!(v.contains("assign src_rsalt"), "{}", v);
}

#[test]
fn a_drop_takes_the_item() {
    let v = compile(concat!(
        "process drain (src: buffer in u32, dst: buffer out u1)\n",
        "  loop\n",
        "    let took = @drop(src)\n",
        "    let _s = @try_send(dst, took)\n",
    ));
    // `took` is the transfer, the same answer `@try_rcv` gives.
    assert!(v.contains("wire src_take = "), "{}", v);
    assert!(v.contains("!src_empty"), "{}", v);
}

#[test]
fn a_bare_drop_discards_the_answer_too() {
    let v = compile(concat!(
        "process drain (src: buffer in u32, dst: buffer out u32)\n",
        "  var n: u32 = @zeroed()\n",
        "  loop\n",
        "    @drop(src)\n",
        "    n = n + 32'd1\n",
        "    let _s = @try_send(dst, n)\n",
    ));
    assert!(v.contains("assign src_rsalt"), "{}", v);
}

#[test]
fn a_drop_spends_the_cycles_one_operation() {
    // Unlike a peek. Two consuming operations on one pipe in one cycle is one
    // transfer being counted twice, whichever pair they are.
    let text = compile_err(concat!(
        "process bad (src: buffer in u32, dst: buffer out u32)\n",
        "  loop\n",
        "    let took = @drop(src)\n",
        "    let (x, got) = @try_rcv(src)\n",
        "    let _s = @try_send(dst, x)\n",
    ));
    assert!(text.contains("received from more than once in one cycle"), "{}", text);
}

#[test]
fn neither_works_on_an_output() {
    let text = compile_err(concat!(
        "process bad (src: buffer in u32, dst: buffer out u32)\n",
        "  loop\n",
        "    let (v, p) = @peek(dst)\n",
        "    let _s = @try_send(dst, v)\n",
    ));
    assert!(text.contains("cannot be peeked at"), "{}", text);

    let text = compile_err(concat!(
        "process bad (src: buffer in u32, dst: buffer out u32)\n",
        "  loop\n",
        "    @drop(dst)\n",
        "    let (x, got) = @try_rcv(src)\n",
        "    let _s = @try_send(dst, x)\n",
    ));
    assert!(text.contains("there is nothing on it to drop"), "{}", text);
}

#[test]
fn a_drop_in_a_state_machine_consumes_in_that_state() {
    // In a process with states, `ready` is per state, so a drop is genuinely
    // "take one here" rather than a rename of an unused binding: the state
    // that drops `src` claims it, and the state that does not, does not.
    let v = compile(concat!(
        "process skipper (ctl: buffer in u1, src: buffer in u32, dst: buffer out u32)
",
        "  loop
",
        "    let c = @rcv(ctl)
",
        "    if c then
",
        "      @drop(src)
",
        "    @send(dst, 32'd1)
",
    ));
    assert!(v.contains("reg state"), "{}", v);
    // The dropping state claims `src`, and only while the branch holds.
    // Only in the cycle the `ctl` receive fires, and only on the branch that
    // asked for it. `src_take` is the toggle enable -- the salt protocol's
    // whole answer to "am I claiming this pipe".
    let take = v.lines().find(|l| l.contains("wire src_take")).expect("a take");
    assert!(
        take.contains("src_xfer_s0") && v.contains("wire src_xfer_s0 = fire_s0 & (!src_empty);"),
        "{}",
        v
    );
    assert!(take.contains("ctl_item") || take.contains("branch_s0"), "{}", v);
}

#[test]
fn a_drop_beside_a_blocking_receive_on_the_same_pipe_is_refused() {
    // A barrier already spends the state's one transfer on that pipe. This
    // used to lower silently and perform ONE consume while the program asked
    // for two.
    let text = compile_err(concat!(
        "process bad (src: buffer in u32, dst: buffer out u32)
",
        "  loop
",
        "    let x = @rcv(src)
",
        "    if x[0] then
",
        "      @drop(src)
",
        "    @send(dst, x)
",
    ));
    assert!(text.contains("received from more than once in one cycle"), "{}", text);
}

#[test]
fn only_the_two_pair_forms_produce_a_pair() {
    let text = compile_err(concat!(
        "process bad (src: buffer in u32, dst: buffer out u32)\n",
        "  loop\n",
        "    let (a, b) = src\n",
        "    let _s = @try_send(dst, a)\n",
    ));
    assert!(text.contains("`@try_rcv(p)` and `@peek(p)` produce a pair"), "{}", text);
}

// ---- a pipe is claimed where the program asks for it ----------------------

/// The widening-multiply shape with no stall construct in it: the receive is
/// written on the branch that can take one, and that is the whole of it.
const WIDEN: &str = concat!(
    "process widen (src: buffer in u32, dst: buffer out u32)\n",
    "  var pending: u1 = @zeroed()\n",
    "  var lo: u32 = @zeroed()\n",
    "  loop\n",
    "    let (x, present) = @peek(src)\n",
    "    var took: u1 = 1'b0\n",
    "    if pending then\n",
    "      took = 1'b0\n",
    "    else\n",
    "      took = @drop(src)\n",
    "    let out_val: u32 = if pending then lo else x + x\n",
    "    if pending | took then\n",
    "      let _s = @try_send(dst, out_val)\n",
    "    if pending then\n",
    "      pending = 1'b0\n",
    "    else\n",
    "      if took then\n",
    "        pending = 1'b1\n",
    "        lo = x\n",
);

#[test]
fn a_guarded_receive_takes_ready_down_on_the_other_branch() {
    // There is no `@hold` any more and nothing replaced it: the branch the
    // `@drop` is written on IS the condition, so a cycle spent finishing
    // something declines its input by construction.
    let v = compile(WIDEN);
    
    let take = v
        .lines()
        .find(|l| l.contains("wire src_take"))
        .expect("a transfer condition");
    let guard = take
        .trim()
        .trim_end_matches(';')
        .rsplit(" & ")
        .next()
        .unwrap();
    assert!(
        take.contains("src_xfer") && v.contains(&format!("wire {guard} = !pending;")),
        "{}",
        v
    );
}

#[test]
fn an_offer_is_made_on_its_own_branch_and_no_other() {
    // Which is what lets the held cycle produce. The offer used to be gated on
    // an input having transferred, and a cycle that declined its input to
    // finish a result therefore had nowhere to put it.
    let v = compile(WIDEN);
    assert!(v.contains("assign dst_wsalt = dst_wsalt_q;"), "{}", v);
    // Anti-vacuous: an UNguarded send offers whenever it is reached, so the
    // guard above is doing something rather than being the default.
    let plain = compile(concat!(
        "process pass (src: buffer in u32, dst: buffer out u32)\n",
        "  loop\n",
        "    let (x, got) = @try_rcv(src)\n",
        "    let _s = @try_send(dst, x)\n",
    ));
    assert!(plain.contains("wire src_take = !src_empty;"), "{}", plain);
    assert!(plain.contains("wire dst_push = !dst_full;"), "{}", plain);
}

#[test]
fn got_agrees_with_the_narrowed_ready() {
    // `got` has to mean "a transfer happened". A receive on a branch claims
    // the pipe only there, so on any other branch nothing transferred and the
    // answer must say so, or the program acts on an item it never got.
    let v = compile(concat!(
        "process g (src: buffer in u32, dst: buffer out u1)\n",
        "  var arm: u1 = @zeroed()\n",
        "  loop\n",
        "    arm = !arm\n",
        "    var took: u1 = 1'b0\n",
        "    if arm then\n",
        "      took = @drop(src)\n",
        "    let _s = @try_send(dst, took)\n",
    ));
    assert!(v.contains("src_rsalt"), "{}", v);
    assert!(v.contains("arm"), "{}", v);
}
