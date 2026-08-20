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
    "process gate (src: buffer in i32, dst: buffer out i32)\n",
    "  loop\n",
    "    let (v, present) = @peek(src)\n",
    "    let wanted: i1 = present & (!v[0])\n",
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
    assert!(v.contains("wire wanted = src_valid & (!src_data[0]);"), "{}", v);
    assert!(v.contains("wire src_xfer = src_valid & dst_room;"), "{}", v);
}

#[test]
fn a_peek_does_not_spend_the_cycles_one_operation() {
    // GATE peeks and then receives from the same pipe in the same cycle. If
    // the peek counted, this would be "received from more than once".
    let v = compile(GATE);
    assert!(v.contains("assign src_ready = dst_room;"), "{}", v);
}

#[test]
fn two_peeks_are_fine() {
    let v = compile(concat!(
        "process twice (src: buffer in i32, dst: buffer out i1)\n",
        "  loop\n",
        "    let (a, p1) = @peek(src)\n",
        "    let (b, p2) = @peek(src)\n",
        "    let _s = @try_send(dst, p1 & p2 & (a == b))\n",
    ));
    assert!(v.contains("assign src_ready"), "{}", v);
}

#[test]
fn a_drop_takes_the_item() {
    let v = compile(concat!(
        "process drain (src: buffer in i32, dst: buffer out i1)\n",
        "  loop\n",
        "    let took = @drop(src)\n",
        "    let _s = @try_send(dst, took)\n",
    ));
    // `took` is the transfer, the same answer `@try_rcv` gives.
    assert!(v.contains("wire src_xfer = src_valid & dst_room;"), "{}", v);
}

#[test]
fn a_bare_drop_discards_the_answer_too() {
    let v = compile(concat!(
        "process drain (src: buffer in i32, dst: buffer out i32)\n",
        "  var n: i32 = @zeroed()\n",
        "  loop\n",
        "    @drop(src)\n",
        "    n = n + 32'd1\n",
        "    let _s = @try_send(dst, n)\n",
    ));
    assert!(v.contains("assign src_ready"), "{}", v);
}

#[test]
fn a_drop_spends_the_cycles_one_operation() {
    // Unlike a peek. Two consuming operations on one pipe in one cycle is one
    // transfer being counted twice, whichever pair they are.
    let text = compile_err(concat!(
        "process bad (src: buffer in i32, dst: buffer out i32)\n",
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
        "process bad (src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let (v, p) = @peek(dst)\n",
        "    let _s = @try_send(dst, v)\n",
    ));
    assert!(text.contains("cannot be peeked at"), "{}", text);

    let text = compile_err(concat!(
        "process bad (src: buffer in i32, dst: buffer out i32)\n",
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
        "process skipper (ctl: buffer in i1, src: buffer in i32, dst: buffer out i32)
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
    let ready = v.lines().find(|l| l.contains("assign src_ready")).expect("a ready");
    // Only in the cycle the `ctl` receive fires, and only on the branch that
    // asked for it.
    assert!(ready.contains("fire_s0"), "{}", v);
    assert!(ready.contains("ctl_data") || ready.contains("branch_s0"), "{}", v);
}

#[test]
fn a_drop_beside_a_blocking_receive_on_the_same_pipe_is_refused() {
    // A barrier already spends the state's one transfer on that pipe. This
    // used to lower silently and perform ONE consume while the program asked
    // for two.
    let text = compile_err(concat!(
        "process bad (src: buffer in i32, dst: buffer out i32)
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
        "process bad (src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let (a, b) = src\n",
        "    let _s = @try_send(dst, a)\n",
    ));
    assert!(text.contains("`@try_rcv(p)` and `@peek(p)` produce a pair"), "{}", text);
}
