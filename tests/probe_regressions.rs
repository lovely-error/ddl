//! Behavioral checks of the lowered circuit, independent of emitted names.
//! The companion Questa runner also checks the emitted Verilog itself.
mod common;

use common::{Circuit, mask};
use ddl::diag::SourceMap;

#[test]
fn empty_nonblocking_input_never_advances_even_beside_a_barrier() {
    let mut c = Circuit::new(include_str!("probes/p1.ddl"), "p1");
    c.set("a_data", 42);
    for _ in 0..4 {
        c.tick();
        assert_eq!(c.out("b_rsalt"), 0);
    }
    c.set("a_wsalt", 1);
    assert_eq!(c.collect(60, 8), [42]);
    assert_eq!(c.out("b_rsalt"), 0);
}

#[test]
fn a_nonblocking_success_waits_for_the_enclosing_barrier_to_commit() {
    let mut c = Circuit::new(include_str!("probes/p1.ddl"), "p1");
    c.set("a_data", 42);
    c.set("b_data", 7);
    c.set("b_wsalt", 1);
    for _ in 0..12 {
        c.tick();
        assert_eq!(c.out("b_rsalt"), 0);
    }
    c.set("a_wsalt", 1);
    assert_eq!(c.collect(80, 8), [49]);
    assert_eq!(c.out("b_rsalt"), 1);
}

#[test]
fn a_conditional_drop_in_a_blocking_state_only_transfers_on_its_path() {
    let src = "process p (a: buffer in u1, b: buffer in u8, o: buffer out u1)\n  loop\n    let take = @rcv(a)\n    var got: u1 = 1'b0\n    if take then\n      got = @drop(b)\n    @send(o, got)\n";
    for take in [0, 1] {
        let mut c = Circuit::new(src, "p");
        c.set("a_wsalt", 1);
        c.set("a_data", take);
        c.set("b_wsalt", 1);
        assert_eq!(c.collect(80, 1), [take]);
        assert_eq!(c.out("b_rsalt"), take);
    }
}

#[test]
fn polling_empty_and_full_buffers_preserves_their_salts_and_entries() {
    let src = include_str!("probes/p5.ddl");
    let mut a = Circuit::new(src, "p5a");
    a.set("src_wsalt", 1);
    a.set("src_data", 42);
    let mut b = Circuit::new(src, "p5b");
    b.set("src_wsalt", 1);
    b.set("src_data", 42);
    b.set("dst_rsalt", 3);
    for _ in 0..20 {
        a.tick();
        b.tick();
        assert_eq!(a.out("other_rsalt"), 0);
        assert_eq!(b.out("dst_wsalt"), 0);
        assert_eq!(b.out("dst_data"), 0);
    }
    a.set("other_wsalt", 1);
    b.set("dst_rsalt", 0);
    for _ in 0..5 {
        a.tick();
        b.tick();
    }
    assert_eq!(a.out("other_rsalt"), 1);
    assert_eq!(b.out("dst_wsalt"), 1);
}

#[test]
fn a_taken_break_skips_the_later_send() {
    let mut c = Circuit::new(include_str!("probes/p5.ddl"), "p5c");
    c.set("src_wsalt", 1);
    c.set("src_data", 42);
    c.set("other_wsalt", 1);
    for _ in 0..20 {
        c.tick();
        assert_eq!(c.out("dst_wsalt"), 0);
    }
    assert_eq!(c.out("other_rsalt"), 1);
}

#[test]
fn narrow_payload_roundtrips_every_byte() {
    let src = include_str!("probes/p6.ddl");
    let mut pack = Circuit::new(src, "pack_small");
    let mut unpack = Circuit::new(src, "unpack");
    for x in 0..256 {
        pack.set("x", x);
        unpack.set("e", pack.out("o"));
        assert_eq!(unpack.out("o"), x);
    }
}

#[test]
fn an_assertion_checks_only_the_executing_match_arm() {
    let src = include_str!("probes/p8.ddl");
    let mut c = Circuit::new(src, "p8");
    c.set("q_wsalt", 1);
    c.set("q_data", 511);
    for _ in 0..20 {
        assert!(c.assertions_ok());
        c.tick();
    }
    assert_eq!(c.out("p_data") & 255, 255);
    let mut bad = Circuit::new(src, "p8");
    bad.set("q_wsalt", 1);
    bad.set("q_data", 255);
    bad.tick();
    assert!(!bad.assertions_ok(), "Rd8(255) must still fail");
}

#[test]
fn assertions_in_inlined_branch_conditions_are_execution_guarded_too() {
    let src = "fun permitted (x: u8, o: out u1)\n  @fatal(x != 8'hff, \"poison\")\n  o = x[0]\nprocess p (src: buffer in u8, o: buffer out u8)\n  loop\n    let x = @rcv(src)\n    if permitted(x) then\n      @send(o, x)\n";
    let mut c = Circuit::new(src, "p");
    c.set("src_data", 255);
    for _ in 0..8 {
        assert!(c.assertions_ok());
        c.tick();
    }
    c.set("src_wsalt", 1);
    assert!(
        !c.assertions_ok(),
        "the check must execute when the input arrives"
    );
    c.set("src_data", 1);
    assert_eq!(c.collect(80, 8), [1]);
}

#[test]
fn sends_and_branches_observe_source_order_under_backpressure() {
    let mut c = Circuit::new(include_str!("probes/p9.ddl"), "p9");
    assert_eq!(c.collect(200, 4), (0..8).collect::<Vec<_>>());
}

#[test]
fn locals_survive_waits_reinitialize_on_reentry_and_shadow_independently() {
    let mut c = Circuit::new(include_str!("probes/lifetimes.ddl"), "lifetimes");
    c.set("src_wsalt", 3);
    c.set("src_data", (4 << 8) | 1);
    assert_eq!(c.collect(300, 8), [1, 2, 3, 100, 101, 4, 9, 4, 5, 6, 7, 9]);
}

#[test]
fn an_inner_initializer_reads_the_outer_binding_and_reset_restarts_scopes() {
    let src = "process p (o: buffer out u8)\n  var n: u8 = 8'd10\n  loop\n    loop\n      var n: u8 = n + 8'd1\n      @send(o, n)\n      break\n    @send(o, n)\n    break\n";
    let mut c = Circuit::new(src, "p");
    assert_eq!(c.collect(100, 8), [11, 10]);
    c.regs = c.m.regs.iter().map(|r| r.reset).collect();
    c.inputs.clear();
    assert_eq!(c.collect(100, 8), [11, 10]);
}

#[test]
fn a_typed_mutable_receive_checks_its_initializer_type() {
    let map = SourceMap::new(
        "type.ddl",
        "process p (i: buffer in u8, o: buffer out u4)\n  loop\n    var n: u4 = @rcv(i)\n    @send(o, n)\n",
    );
    let errs = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
    assert!(
        map.render_all(&errs)
            .contains("declared `u4` but the pipe carries `u8`")
    );
}

#[test]
fn a_mutable_receive_binding_survives_a_nested_loop() {
    let mut c = Circuit::new(include_str!("probes/lifetimes.ddl"), "received_local");
    c.set("src_wsalt", 3);
    c.set("src_data", (2 << 8) | 3);
    assert_eq!(c.collect(200, 8), [3, 2, 1, 2, 1]);
}

#[test]
fn a_binding_cannot_escape_its_lexical_scope() {
    let map = SourceMap::new(
        "scope.ddl",
        "process p (o: buffer out u8)\n  loop\n    loop\n      var n: u8 = 8'd1\n      @send(o, n)\n      break\n    @send(o, n)\n",
    );
    let errs = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
    assert!(map.render_all(&errs).contains("`n` is not in scope here"));
}

#[test]
fn match_payloads_and_mutable_locals_are_separate_in_sibling_arms() {
    let mut c = Circuit::new(include_str!("probes/arm_scopes.ddl"), "arm_scopes");
    c.set("src_wsalt", 3);
    c.set("src_data", (0x11234 << 17) | 0xaa00);
    assert_eq!(c.collect(120, 16), [0xaa, 0xab, 0x1234, 0x1236]);
}

#[test]
fn a_shadowed_reference_keeps_its_own_source_location() {
    let map = SourceMap::new(
        "shadow.ddl",
        "process p (o: buffer out u16)\n  var n: u8 = 8'd1\n  loop\n    loop\n      var n: u8 = 8'd2\n      @send(o, n)\n      break\n",
    );
    let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
    assert!(map.render_all(&errors).contains("--> shadow.ddl:6:"));
}

#[test]
fn selection_bases_remain_signals_and_register_names_are_unique() {
    // p7.ddl holds two independent funs, so which one the file is for is a
    // question the compiler will not answer on its own; naming both says
    // "all of them", which is what a probe fixture wants.
    let emit = |src, targets: &[&str]| {
        let opts = ddl::verilog::EmitOptions {
            export: ddl::ir_export::ExportFlags {
                export: targets.iter().map(|s| s.to_string()).collect(),
                bare: Vec::new(),
            crossings: Vec::new(),
            },
            ..Default::default()
        };
        ddl::driver::compile_to_verilog(&SourceMap::new("test.ddl", src), &opts).unwrap()
    };
    let selects = emit(include_str!("probes/p7.ddl"), &["p7a", "p7b"]);
    assert!(!selects.contains("[7:0][7]"));
    assert!(!selects.contains("(v >> k)["));
    let names = emit(include_str!("probes/p11.ddl"), &[]);
    assert!(names.contains("reg [7:0] ans_data_1;"));
    assert!(names.contains("assign ans_data = {ans_e1, ans_e0};"));
}

#[test]
fn receive_and_drop_ignore_unrelated_output_backpressure() {
    for operation in ["let took = @drop(src)", "let (x, took) = @try_rcv(src)"] {
        for available in [0, 1] {
            let source = format!(
                "process p (src: buffer in u8, blocked: buffer out u8, observed: buffer out u1)\n  loop\n    {operation}\n    @try_send(observed, took)\n"
            );
            let mut c = Circuit::new(&source, "p");
            c.set("src_wsalt", available);
            c.set("src_data", 42);
            c.set("blocked_rsalt", 3); // wsalt=0 means full.
            c.tick();
            assert_eq!(c.out("src_rsalt"), available);
            assert_eq!(c.out("blocked_wsalt"), 0);
            // The tap is a pipe, so it shows what was committed last cycle
            // rather than what is being decided this one.
            assert_eq!(c.out("observed_wsalt"), 1);
            assert_eq!(c.out("observed_data") & 1, available);
        }
    }
}

#[test]
fn peek_only_and_unrequested_inputs_never_consume() {
    for available in [0, 1] {
        let source = "process p (src: buffer in u8, unused: buffer in u8, observed: buffer out u1)\n  loop\n    let (x, present) = @peek(src)\n    @try_send(observed, present)\n";
        let mut c = Circuit::new(source, "p");
        c.set("src_wsalt", available);
        c.set("unused_wsalt", 1);
        for _ in 0..12 {
            // Drain the tap every cycle, so it never fills and the process
            // keeps being asked the question. This is what the old `port`
            // spelling gave for free: a sink that is always ready.
            let rsalt = c.out("observed_wsalt");
            c.set("observed_rsalt", rsalt);
            c.tick();
            assert_eq!(c.out("src_rsalt"), 0);
            assert_eq!(c.out("unused_rsalt"), 0);
            assert_eq!(c.out("observed_data") & 1, available);
        }
    }
}

#[test]
fn polling_peek_send_drop_forwards_each_item_once_under_backpressure() {
    let mut c = Circuit::new(include_str!("probes/communication.ddl"), "polling_forward");
    c.set("unrelated_rsalt", 3);
    let (mut w, mut r, mut produced, mut pair) = (0u128, 0u128, 0u128, 0u128);
    let mut received = Vec::new();
    for cycle in 0..400 {
        if produced < 64 && w != ((!c.out("src_rsalt")) & 3) && cycle % 7 != 0 {
            let idx = (w ^ (w >> 1)) & 1;
            pair = (pair & !(255 << (idx * 8))) | (produced << (idx * 8));
            produced += 1;
            w ^= if idx == 0 { 1 } else { 2 };
        }
        c.set("src_wsalt", w);
        c.set("src_data", pair);
        if cycle % 11 >= 6 && c.out("o_wsalt") != r {
            let idx = (r ^ (r >> 1)) & 1;
            received.push((c.out("o_data") >> (idx * 8)) & 255);
            r ^= if idx == 0 { 1 } else { 2 };
        }
        c.set("o_rsalt", r);
        assert!(c.assertions_ok());
        c.tick();
        assert_eq!(c.out("unrelated_wsalt"), 0);
    }
    assert_eq!(received, (0..64).collect::<Vec<_>>());
    assert_eq!(c.out("src_rsalt"), w);
}

#[test]
fn a_finished_polling_process_stops_sends_and_execution_assertions() {
    let mut c = Circuit::new(include_str!("probes/communication.ddl"), "polling_once");
    c.set("src_wsalt", 1);
    c.set("src_data", 42);
    assert!(c.assertions_ok());
    c.tick();
    for _ in 0..12 {
        assert!(c.assertions_ok());
        assert_eq!(c.out("src_rsalt"), 1);
        assert_eq!(c.out("o_wsalt"), 1);
        assert_eq!(c.out("o_data") & 255, 42);
        c.tick();
    }
}

#[test]
fn independent_is_an_identifier_not_a_process_modifier() {
    use ddl::driver::compile_to_verilog;
    use ddl::verilog::EmitOptions;
    let ordinary = SourceMap::new(
        "name.ddl",
        "process independent (src: buffer in u8)\n  loop\n    let took = @drop(src)\n",
    );
    assert!(compile_to_verilog(&ordinary, &EmitOptions::default()).is_ok());
    let blocking = SourceMap::new(
        "mode.ddl",
        "process independent p (src: buffer in u8)\n  loop\n    let x = @rcv(src)\n",
    );
    assert!(compile_to_verilog(&blocking, &EmitOptions::default()).is_err());
}

#[test]
fn shared_branch_continuations_survive_every_predecessor() {
    for first in [false, true] {
        for second in [false, true] {
            for use_match in [false, true] {
                let next = if use_match {
                    "    match b\n      .Yes =>\n        @send(o, 8'd1)\n      .No =>\n        @send(o, 8'd2)\n"
                } else {
                    "    if b then\n      @send(o, 8'd1)\n    else\n      @send(o, 8'd2)\n"
                };
                let d_ty = if use_match { "choice" } else { "u1" };
                let src = format!("enum choice: u1\n  No\n  Yes\nprocess p (c: buffer in u1, d: buffer in {d_ty}, i: buffer in u8, o: buffer out u8)\n  loop\n    let a = @rcv(c)\n    let b = @rcv(d)\n    if a then\n      let x = @rcv(i)\n{next}");
                let mut c = Circuit::new(&src, "p");
                c.set("c_wsalt", 1); c.set("c_data", first as u128);
                c.set("d_wsalt", 1); c.set("d_data", second as u128);
                c.set("i_wsalt", 1);
                assert_eq!(c.collect(60, 8), [if second { 1 } else { 2 }]);
            }
        }
    }
}

#[test]
fn for_only_reads_cross_waits() {
    let mut c = Circuit::new(include_str!("probes/adversarial.ddl"), "for_read2");
    c.set("src_wsalt", 3); c.set("src_data", 0x0703);
    for _ in 0..8 { c.tick(); }
    c.set("other_wsalt", 1);
    for _ in 0..20 { c.tick(); }
    assert_eq!(c.out("dst_wsalt"), 1);
    assert_eq!(c.out("dst_data") & 255, 12);
}

#[test]
fn exclusive_channel_requests_mux_data_and_transfer_results() {
    for flag in [0, 1] {
        for full in [false, true] {
            let src = "process p (c: buffer in u1, o: buffer out u8, success: buffer out u1)
  loop
    let (flag, present) = @peek(c)
    var sent: u1 = 1'b0
    if flag then
      sent = @try_send(o, 8'd1)
    else
      sent = @try_send(o, 8'd2)
    @try_send(success, sent)
".to_string();
            let mut c = Circuit::new(&src, "p");
            c.set("c_wsalt", 1); c.set("c_data", flag);
            c.set("o_rsalt", if full {3} else {0});
            c.tick();
            // The transfer result rides out on `success`, one cycle behind the
            // decision that produced it: a buffer's entry is a register, so
            // what the consumer reads is what the sender committed last cycle.
            assert_eq!(c.out("success_wsalt"), 1);
            assert_eq!(c.out("success_data") & 1, (!full) as u128);
            assert_eq!(c.out("o_wsalt"), (!full) as u128);
            if !full { assert_eq!(c.out("o_data") & 255, if flag == 1 {1} else {2}); }
        }
    }
}

#[test]
fn overlapping_requests_and_barrier_port_duplicates_are_rejected() {
    for body in [
        "    @try_send(o, 8'd1)\n    @try_send(o, 8'd2)\n",
        "    @try_send(o, 8'd1)\n    @send(o, 8'd2)\n",
        "    @send(o, 8'd1)\n    @try_send(o, 8'd2)\n",
    ] {
        let map = SourceMap::new("duplicate.ddl", format!("process p (o: buffer out u8)\n  loop\n{body}"));
        let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
        assert!(map.render_all(&errors).contains("sent to more than once in one cycle"));
    }
}

#[test]
fn for_waits_and_breaks_have_an_explicit_diagnostic() {
    for op in ["@send(o, 8'd1)", "let x = @rcv(i)", "break", "let x = mem[0]"] {
        let memory = if op.contains("mem") { "  var mem: #[impl(bram)] [u8; 16]\n" } else { "" };
        let map = SourceMap::new("for_wait.ddl", format!("process p (i: buffer in u8, o: buffer out u8)\n{memory}  loop\n    for k in 0..2\n      {op}\n    @send(o, 8'd0)\n"));
        let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
        let text = map.render_all(&errors);
        assert!(text.contains("a `for` body must be combinational"), "{text}");
    }
}

#[test]
fn sequence_assertions_follow_validity_and_shift_at_every_stage() {
    for stage in 0..3 {
        for builtin in ["assert", "fatal"] {
            let mut body = "  let x = @rcv(src)\n".to_string();
            for k in 0..3 {
                if k == stage {
                    body += &format!("  @{builtin}(x != 8'd0, \"zero item\")\n");
                }
                if k != 2 { body += "  |||\n"; }
            }
            body += "  @send(o, x)\n";
            let src = format!("sequence s (src: buffer in u8, o: buffer out u8)\n{body}");
            let mut c = Circuit::new(&src, "s");
            for _ in 0..6 { assert!(c.assertions_ok()); c.tick(); }
            // Full, and it stays full: write salt is still zero.
            c.set("o_rsalt", 3);
            // Each stage shifts on its own, so a stall holds an item only where
            // the cut below it is occupied. Good items fill every stage below
            // `stage`; the zero-valued one behind them stops AT `stage`.
            let mut items = vec![1; 2 - stage];
            items.push(0);
            feed(&mut c, &items);
            for _ in 0..6 { assert!(c.assertions_ok(), "stage {stage}, {builtin}"); c.tick(); }
            // A zero-valued transaction fails only when its stage advances.
            c.set("o_rsalt", 0);
            assert!(!c.assertions_ok(), "stage {stage}, {builtin}");
            c.tick();
            assert!(c.assertions_ok());
        }
    }
}

/// Offers `items` on `src` in order, waiting for room before each, then lets
/// the pipeline settle. Assertions must hold throughout.
///
/// It does not insist the sequence took them all: an item meant to stop in
/// stage 0 is one the head has not taken.
fn feed(c: &mut Circuit, items: &[u128]) {
    let mut w = 0u128;
    let mut data = 0u128;
    for item in items {
        for _ in 0..20 {
            let full = w == (!c.out("src_rsalt") & 3);
            if !full { break; }
            assert!(c.assertions_ok());
            c.tick();
        }
        let ix = (w ^ (w >> 1)) & 1;
        data = (data & !(0xff << (ix * 8))) | (item << (ix * 8));
        c.set("src_data", data);
        w ^= if ix == 0 { 1 } else { 2 };
        c.set("src_wsalt", w);
        assert!(c.assertions_ok());
        c.tick();
    }
    for _ in 0..8 {
        assert!(c.assertions_ok());
        c.tick();
    }
}

/// One side of a pipe as a consumer sees it: the next entry if one is on
/// offer, and taking it.
fn offered(c: &Circuit, pipe: &str, r: u128, width: u32) -> Option<u128> {
    let has_item = c.out(&format!("{pipe}_wsalt")) != r;
    if !has_item {
        return None;
    }
    let ix = (r ^ (r >> 1)) & 1;
    Some((c.out(&format!("{pipe}_data")) >> (ix as u32 * width)) & mask(width))
}

fn taken(r: u128) -> u128 {
    let ix = (r ^ (r >> 1)) & 1;
    r ^ if ix == 0 { 1 } else { 2 }
}

/// Runs sequence `s`, with outputs `x` and `y`, against a consumer that JOINS
/// its two outputs -- takes from `x` and `y` together or from neither -- and
/// stalls on an irregular pattern. Answers the pairs it received.
fn run_join(src: &str, n: u128, cycles: usize) -> Vec<(u128, u128)> {
    let mut c = Circuit::new(src, "s");
    let (mut w, mut data, mut next) = (0u128, 0u128, 0u128);
    let (mut rx, mut ry) = (0u128, 0u128);
    let mut got = Vec::new();
    for cycle in 0..cycles {
        // The producer offers the next item whenever the input has room.
        let input_has_room = w != (!c.out("src_rsalt") & 3);
        if input_has_room && next < n {
            let ix = (w ^ (w >> 1)) & 1;
            data = (data & !(0xff << (ix * 8))) | ((next + 1) << (ix * 8));
            c.set("src_data", data);
            w = taken(w);
            c.set("src_wsalt", w);
            next += 1;
        }
        let consumer_is_stalling = cycle % 13 >= 9;
        let pair = (offered(&c, "x", rx, 8), offered(&c, "y", ry, 8));
        if let (false, (Some(a), Some(b))) = (consumer_is_stalling, pair) {
            got.push((a, b));
            rx = taken(rx);
            ry = taken(ry);
            c.set("x_rsalt", rx);
            c.set("y_rsalt", ry);
        }
        assert!(c.assertions_ok());
        c.tick();
    }
    got
}

#[test]
fn outputs_sent_stages_apart_can_be_joined_downstream() {
    // `x` leaves in stage 0 and `y` two stages later. The joining consumer
    // cannot drain `x` until `y` arrives, so `x` fills while the item's late
    // half is still in the pipeline. With one shift for the whole pipeline
    // that is a deadlock after two items: `x` full holds every stage, and the
    // late half never gets to its send. Per stage, the stages below keep
    // moving and everything arrives, paired and in order.
    let src = "sequence s (src: buffer in u8, x: buffer out u8, y: buffer out u8)\n  let a = @rcv(src)\n  @send(x, a)\n  |||\n  let b: u8 = a + 8'd1\n  |||\n  @send(y, b + 8'd1)\n";
    let n = 40;
    let got = run_join(src, n, 600);
    let want: Vec<(u128, u128)> = (1..=n).map(|i| (i, i + 2)).collect();
    assert_eq!(got, want);
}

#[test]
fn outputs_sent_from_one_stage_join_the_same_way() {
    // The shape that always worked, through the same harness, so the test
    // above is not passing on something the harness does by itself.
    let src = "sequence s (src: buffer in u8, x: buffer out u8, y: buffer out u8)\n  let a = @rcv(src)\n  |||\n  let b: u8 = a + 8'd1\n  |||\n  @send(x, a)\n  @send(y, b + 8'd1)\n";
    let n = 40;
    let got = run_join(src, n, 600);
    let want: Vec<(u128, u128)> = (1..=n).map(|i| (i, i + 2)).collect();
    assert_eq!(got, want);
}

#[test]
fn a_stalled_sink_holds_only_the_stages_whose_cut_below_is_occupied() {
    // The behavioural half of per-stage shifting. The sink is full from the
    // start, and a bad item in stage 0 still moves -- and fires -- because
    // nothing is below it to wait for. Under one global shift it would have
    // sat in stage 0 forever.
    let src = "sequence s (src: buffer in u8, o: buffer out u8)\n  let x = @rcv(src)\n  @assert(x != 8'd0, \"zero item\")\n  |||\n  |||\n  @send(o, x)\n";
    let mut c = Circuit::new(src, "s");
    c.set("o_rsalt", 3);
    c.set("src_data", 0);
    c.set("src_wsalt", 1);
    assert!(!c.assertions_ok(), "stage 0 moves into an empty stage 1 while the sink is full");
    c.tick();
    assert_eq!(c.out("src_rsalt"), 1, "and the head took the item");
    assert!(c.assertions_ok());
}

#[test]
fn sequence_tail_inlined_assertions_are_execution_guarded() {
    let src = "fun checked (x: u8, y: out u8)\n  @assert(x != 8'd0)\n  y = x\nsequence s (src: buffer in u8, o: buffer out u8)\n  let x = @rcv(src)\n  |||\n  @send(o, checked(x))\n";
    let mut c = Circuit::new(src, "s");
    assert!(c.assertions_ok());
    c.set("src_wsalt", 1);
    c.tick();
    assert!(!c.assertions_ok());
    c.set("o_rsalt", 3);
    assert!(c.assertions_ok());
}

#[test]
fn sequence_shadowing_does_not_replace_outer_values() {
    for inner_ty in ["u16", "u32"] {
        let src = format!("sequence s (src: buffer in u16, o: buffer out u16)\n  let a = @rcv(src)\n  let x: u16 = 10\n  |||\n  if a > 16'd5 then\n    let x: {inner_ty} = 20\n  |||\n  @send(o, x)\n");
        let mut c = Circuit::new(&src, "s");
        c.set("src_wsalt", 3);
        c.set("src_data", (2 << 16) | 6);
        assert_eq!(c.collect(40, 16), [10, 10]);
    }
}

#[test]
fn sequence_tail_constants_use_the_output_type() {
    for (expr, expected) in [("0", 0), ("42", 42), ("@zeroed()", 0)] {
        let src = format!("sequence s (src: buffer in u16, o: buffer out u16)\n  let x = @rcv(src)\n  |||\n  @send(o, {expr})\n");
        let mut c = Circuit::new(&src, "s");
        c.set("src_wsalt", 3);
        assert_eq!(c.collect(40, 16), [expected, expected]);
    }
    let map = SourceMap::new("overflow.ddl", "sequence s (src: buffer in u8, o: buffer out u8)\n  let x = @rcv(src)\n  @send(o, 256)\n");
    assert!(ddl::driver::compile_to_verilog(&map, &Default::default()).is_err());
}

#[test]
fn sequence_duplicate_transfers_and_misplaced_nonblocking_buffers_are_diagnosed() {
    for (body, message) in [
        ("  let x = @rcv(src)\n  let y = @rcv(src)\n  |||\n  @send(o, x)\n", "`src` is received from twice"),
        ("  let x = @rcv(src)\n  |||\n  @send(o, 8'd1)\n  @send(o, 8'd2)\n", "`o` is sent to more than once for one item"),
        // `@peek`, `@try_rcv` and `@drop` are supported in every stage, but a
        // pipe belongs to one: below the head, these touch the pipe the head
        // already receives from, and would act for a different item.
        ("  let x = @rcv(src)\n  |||\n  let (v, ok) = @peek(src)\n  @send(o, x)\n", "`src` is received from in stage 0 and stage 1"),
        ("  let x = @rcv(src)\n  |||\n  let (v, ok) = @try_rcv(src)\n  @send(o, x)\n", "`src` is received from in stage 0 and stage 1"),
        ("  let x = @rcv(src)\n  |||\n  @drop(src)\n  @send(o, x)\n", "`src` is received from in stage 0 and stage 1"),
        ("  let x = @rcv(src)\n  |||\n  @try_send(o, x)\n  @send(o, x)\n", "`o` is sent to more than once for one item"),
    ] {
        let map = SourceMap::new("invalid_sequence.ddl", format!("sequence s (src: buffer in u8, o: buffer out u8)\n{body}"));
        // Calling the API directly ensures a panic cannot masquerade as a diagnostic.
        let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
        let text = map.render_all(&errors);
        assert!(text.contains(message), "{text}");
    }
}

#[test]
fn sequence_bram_rebinding_keeps_address_and_result_distinct() {
    let src = "sequence s (src: buffer in u4, o: buffer out u16)\n  var mem: #[impl(bram)] [u16; 16]\n  let addr = @rcv(src)\n  |||\n  let addr = mem[addr]\n  |||\n  @send(o, addr)\n";
    let map = SourceMap::new("rebind.ddl", src);
    ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap();
    // Distinct identities must not excuse using the new result before its edge.
    let bad = src.replace("  |||\n  @send", "  @send");
    let map = SourceMap::new("early.ddl", bad);
    let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
    assert!(map.render_all(&errors).contains("stage"));
}

#[test]
fn sequence_assertions_keep_the_enclosing_branch_guard() {
    let src = "sequence s (src: buffer in u8, o: buffer out u8)\n  let x = @rcv(src)\n  |||\n  if x != 8'd0 then\n    @assert(x == 8'd1)\n  @send(o, x)\n";
    for (item, expected) in [(0, true), (1, true), (2, false)] {
        let mut c = Circuit::new(src, "s");
        c.set("src_wsalt", 1);
        c.set("src_data", item);
        c.tick();
        assert_eq!(c.assertions_ok(), expected);
    }
}

#[test]
fn sequence_send_captures_the_payload_before_later_assignments() {
    for cuts in 0..3 {
        let src = format!(
            "sequence s (src: buffer in u8, o: buffer out u8)\n  let a = @rcv(src)\n{}  var x: u8 = a\n  @send(o, x)\n  x = 8'd99\n  @assert(x == 8'd99)\n",
            "  |||\n".repeat(cuts),
        );
        let mut c = Circuit::new(&src, "s");
        c.set("src_wsalt", 3);
        c.set("src_data", (42 << 8) | 7);
        assert_eq!(c.collect(40, 8), [7, 42], "{cuts} cuts");
    }
}

#[test]
fn sequence_receive_annotations_are_checked_and_mutability_survives_cuts() {
    for ty in ["DoesNotExist", "u16", "i8"] {
        let map = SourceMap::new("receive.ddl", format!("sequence s (src: buffer in u8, o: buffer out u8)\n  let a: {ty} = @rcv(src)\n  |||\n  @send(o, a)\n"));
        let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
        let text = map.render_all(&errors);
        assert!(text.contains("receive.ddl:2:"), "{text}");
        assert!(text.contains(ty), "{text}");
    }
    let src = "sequence s (src: buffer in u8, o: buffer out u8)\n  var a: u8 = @rcv(src)\n  a += 1\n  |||\n  a += 2\n  |||\n  @send(o, a)\n";
    let mut c = Circuit::new(src, "s");
    c.set("src_wsalt", 3);
    c.set("src_data", (42 << 8) | 7);
    assert_eq!(c.collect(40, 8), [10, 45]);
    let map = SourceMap::new("immutable.ddl", src.replace("var a", "let a"));
    let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
    assert!(map.render_all(&errors).contains("cannot be assigned"));
}

#[test]
fn sequence_bram_read_annotations_are_checked_before_the_cut() {
    for ty in ["DoesNotExist", "u16", "i8", "u8"] {
        // Rebinding also checks that diagnostics retain the source anchor
        // after lexical resolution gives the read result a synthetic name.
        let map = SourceMap::new("read.ddl", format!("sequence s (src: buffer in u4, o: buffer out u8)\n  var mem: #[impl(bram)] [u8; 16]\n  let a = @rcv(src)\n  let a: {ty} = mem[a]\n  |||\n  @send(o, a)\n"));
        let result = ddl::driver::compile_to_verilog(&map, &Default::default());
        if ty == "u8" {
            result.unwrap();
        } else {
            let text = map.render_all(&result.unwrap_err());
            assert!(text.contains("read.ddl:4:"), "{text}");
            assert!(text.contains(ty), "{text}");
        }
    }
}

#[test]
fn process_nonblocking_shadowing_preserves_outer_bindings() {
    for inner_ty in ["u8", "u16"] {
        for repeating in [false, true] {
            let indent = if repeating { "    " } else { "  " };
            let body = format!("{indent}let (a, got) = @try_rcv(src)\n{indent}let x: u8 = 10\n{indent}if a > 8'd5 then\n{indent}  let x: {inner_ty} = 20\n{indent}@try_send(o, x)\n");
            let src = format!("process p (src: buffer in u8, o: buffer out u8)\n{}{body}", if repeating { "  loop\n" } else { "" });
            let mut c = Circuit::new(&src, "p");
            c.set("src_data", 6);
            c.set("src_wsalt", 1);
            let outputs = c.collect(20, 8);
            assert!(!outputs.is_empty());
            assert!(outputs.iter().all(|v| *v == 10), "{outputs:?}");
        }
    }
}

#[test]
fn process_direct_bram_read_checks_its_declared_type() {
    for ty in ["DoesNotExist", "u16", "i8", "u8"] {
        let map = SourceMap::new("process_read.ddl", format!("process p (src: buffer in u4, o: buffer out u8)\n  var mem: #[impl(bram)] [u8; 16]\n  loop\n    let a = @rcv(src)\n    let a: {ty} = mem[a]\n    @send(o, a)\n"));
        let result = ddl::driver::compile_to_verilog(&map, &Default::default());
        if ty == "u8" {
            result.unwrap();
        } else {
            let text = map.render_all(&result.unwrap_err());
            assert!(text.contains("process_read.ddl:5:"), "{text}");
            assert!(text.contains(ty), "{text}");
        }
    }
}

#[test]
fn process_conditional_bram_reads_execute_only_the_selected_arm() {
    for expression in [
        "if a then mem[@try_send(tap, 8'd7)] else 8'd0",
        "if a then 8'd0 else mem[@try_send(tap, 8'd7)]",
        "if a then mem[@try_send(tap, 8'd7)] else mem[1'd0]",
    ] {
        for item in [0, 1] {
            let src = format!("process p (src: buffer in u1, tap: buffer out u8, o: buffer out u8)\n  var mem: #[impl(bram)] [u8; 2]\n  loop\n    let a = @rcv(src)\n    let r = {expression}\n    @send(o, r)\n");
            let mut c = Circuit::new(&src, "p");
            c.memories[0] = vec![11, 42];
            c.set("src_wsalt", 1);
            c.set("src_data", item);
            let mut taps = vec![];
            for _ in 0..20 {
                assert!(c.assertions_ok());
                // Drain the tap so it is always ready, which is what the old
                // `port` spelling gave for free and what makes the
                // `@try_send` under test succeed whenever it is reached.
                let ws = c.out("tap_wsalt");
                c.set("tap_rsalt", ws);
                c.tick();
                if c.out("tap_wsalt") != ws {
                    let ix = (ws ^ (ws >> 1)) & 1;
                    taps.push((c.out("tap_data") >> (ix * 8)) & 255);
                }
            }
            let selected = if expression.starts_with("if a then 8'd0") { item == 0 } else { item != 0 };
            let other_read = expression.ends_with("else mem[1'd0]") && !selected;
            assert_eq!(taps, if selected { vec![7] } else { vec![] }, "{expression}, {item}");
            assert_eq!(c.read_events, if selected { vec![(0, 1)] } else if other_read { vec![(0, 0)] } else { vec![] });
            assert_eq!(c.out("o_wsalt"), 1);
            assert_eq!(c.out("o_data") & 255, if selected { 42 } else if other_read { 11 } else { 0 });
        }
    }
}

#[test]
fn process_nested_conditional_reads_survive_reentry_and_output_stalls() {
    for expression in [
        "if a[1] then (if a[0] then mem[1'd1] else 8'd5) else mem[1'd0]",
        "\n      if a[1] then (if a[0] then mem[1'd1] else 8'd5) else mem[1'd0]",
    ] {
        let src = format!("process p (src: buffer in u2, o: buffer out u8)\n  var mem: #[impl(bram)] [u8; 2]\n  loop\n    let a = @rcv(src)\n    let r = {expression}\n    @send(o, r)\n");
        let mut c = Circuit::new(&src, "p");
        c.memories[0] = vec![11, 42];
        let items = [0, 2, 3, 1];
        let (mut next, mut ws, mut rs, mut data) = (0, 0, 0, 0);
        let mut outputs = vec![];
        for cycle in 0..100 {
            if next < items.len() && ws != (c.out("src_rsalt") ^ 3) {
                let ix = (ws ^ (ws >> 1)) & 1;
                data = (data & !(3 << (ix * 2))) | (items[next] << (ix * 2));
                ws ^= if ix == 0 { 1 } else { 2 };
                next += 1;
            }
            c.set("src_wsalt", ws);
            c.set("src_data", data);
            c.set("o_rsalt", rs);
            let next_rs = if cycle > 40 && cycle % 3 != 0 && c.out("o_wsalt") != rs {
                let ix = (rs ^ (rs >> 1)) & 1;
                outputs.push((c.out("o_data") >> (ix * 8)) & 255);
                rs ^ if ix == 0 { 1 } else { 2 }
            } else { rs };
            c.tick();
            rs = next_rs;
        }
        assert_eq!(outputs, [11, 5, 42, 11], "{expression}");
        assert_eq!(c.read_events, [(0, 0), (0, 1), (0, 0)]);
    }
}

#[test]
fn process_conditional_read_evaluates_its_condition_once() {
    let src = "process p (src: buffer in u8, flag: buffer out u8, o: buffer out u8)\n  var mem: #[impl(bram)] [u8; 2]\n  loop\n    let a = @rcv(src)\n    let r = if @try_send(flag, a) then mem[1'd1] else mem[1'd0]\n    @send(o, r)\n";
    let mut c = Circuit::new(src, "p");
    c.memories[0] = vec![11, 42];
    c.set("src_wsalt", 1);
    c.set("src_data", 7);
    let mut flags = vec![];
    for _ in 0..20 {
        let ws = c.out("flag_wsalt");
        c.set("flag_rsalt", ws);
        c.tick();
        if c.out("flag_wsalt") != ws {
            let ix = (ws ^ (ws >> 1)) & 1;
            flags.push((c.out("flag_data") >> (ix * 8)) & 255);
        }
    }
    assert_eq!(flags, [7]);
    assert_eq!(c.read_events, [(0, 1)]);
    assert_eq!(c.out("o_data") & 255, 42);
}

// ---- sequences with several pipes -----------------------------------------

/// A producer that always has both entries filled: the salt one lap ahead of
/// whatever the consumer has acknowledged.
fn keep_full(c: &mut Circuit, pipe: &str, data: u128) {
    let acked = c.out(&format!("{}_rsalt", pipe));
    c.set(&format!("{}_wsalt", pipe), (!acked) & 3);
    c.set(&format!("{}_data", pipe), data);
}

const JOIN: &str = concat!(
    "sequence s (a: buffer in u16, b: buffer in u16, o: buffer out u16)\n",
    "  let x = @rcv(a)\n",
    "  let y = @rcv(b)\n",
    "  |||\n",
    "  @send(o, x + y)\n",
);

#[test]
fn a_join_waits_for_its_slowest_input() {
    let mut c = Circuit::new(JOIN, "s");
    c.set("a_wsalt", 1);
    c.set("a_data", 5);
    // `b` has nothing. Neither input may be consumed and nothing may leave:
    // the item `a` is offering has no partner yet, and taking it would strand
    // it a cycle deep in a pipeline that cannot finish it.
    for _ in 0..20 {
        c.tick();
        assert_eq!(c.out("a_rsalt"), 0, "a was taken without b");
        assert_eq!(c.out("o_wsalt"), 0, "an item left without b");
    }
    c.set("b_wsalt", 1);
    c.set("b_data", 7);
    c.tick();
    assert_eq!(c.out("a_rsalt"), 1, "both step together");
    assert_eq!(c.out("b_rsalt"), 1, "both step together");
    c.tick();
    assert_eq!(c.out("o_wsalt"), 1);
    assert_eq!(c.out("o_data") & 0xffff, 12);
}

#[test]
fn a_scatter_stalls_on_its_fullest_sink() {
    let src = concat!(
        "sequence s (a: buffer in u16, x: buffer out u16, y: buffer out u16)\n",
        "  let p = @rcv(a)\n",
        "  |||\n",
        "  @send(x, p)\n",
        "  @send(y, p)\n",
    );
    let mut c = Circuit::new(src, "s");
    // `x` is drained every cycle and `y` never is. Every sink or none, so `x`
    // stops accepting when `y` fills, however eagerly it is being read.
    for _ in 0..30 {
        keep_full(&mut c, "a", 4);
        let drained = c.out("x_wsalt");
        c.set("x_rsalt", drained);
        c.tick();
        assert_eq!(c.out("x_wsalt"), c.out("y_wsalt"), "the sinks move together");
    }
    assert_eq!(c.out("y_wsalt"), 3, "y took its two entries and no more");

    // Releasing the slow sink releases the fast one, and the input with it.
    // Counted rather than compared: a salt is two gray-coded bits and comes
    // back round to where it started every four transfers.
    let mut pushes = 0;
    let mut last = c.out("y_wsalt");
    for _ in 0..20 {
        keep_full(&mut c, "a", 4);
        let x_drained = c.out("x_wsalt");
        let y_drained = c.out("y_wsalt");
        c.set("x_rsalt", x_drained);
        c.set("y_rsalt", y_drained);
        c.tick();
        assert_eq!(c.out("x_wsalt"), c.out("y_wsalt"), "the sinks move together");
        let now = c.out("y_wsalt");
        if now != last {
            pushes += 1;
            last = now;
        }
    }
    assert!(pushes > 4, "the pipeline resumed; saw {} pushes", pushes);
}

const OPTIONAL: &str = concat!(
    "sequence s (a: buffer in u16, b: buffer in u16, o: buffer out u16)\n",
    "  let x = @rcv(a)\n",
    "  let (y, ok) = @try_rcv(b)\n",
    "  var t: u16 = x\n",
    "  if ok then\n",
    "    t = x + y\n",
    "  |||\n",
    "  @send(o, t)\n",
);

#[test]
fn an_optional_input_does_not_stall_the_pipeline() {
    let mut c = Circuit::new(OPTIONAL, "s");
    c.set("a_wsalt", 1);
    c.set("a_data", 5);
    c.tick();
    c.tick();
    assert_eq!(c.out("o_wsalt"), 1, "the head fired without b");
    assert_eq!(c.out("o_data") & 0xffff, 5, "ok was low, so t is x alone");
    assert_eq!(c.out("b_rsalt"), 0, "an empty optional input is not consumed");

    // Offer one item on the optional input; exactly one output carries it, and
    // its salt steps exactly once.
    c.set("a_wsalt", 3);
    c.set("a_data", (6 << 16) | 5);
    c.set("b_wsalt", 1);
    c.set("b_data", 7);
    c.tick();
    assert_eq!(c.out("b_rsalt"), 1, "the optional input gave up its item");
    c.tick();
    assert_eq!(c.out("o_wsalt"), 3);
    assert_eq!((c.out("o_data") >> 16) & 0xffff, 13, "x + y this time");
    for _ in 0..10 {
        c.tick();
        assert_eq!(c.out("b_rsalt"), 1, "and only that one item");
    }
}

#[test]
fn an_idle_all_optional_head_commits_nothing() {
    // The bubble, pinned. The pipeline shifts on every one of these cycles --
    // both sinks have room -- and shifting is not committing: `offered` is low,
    // so the validity bit it shifts in is low and nothing reaches the output.
    let src = concat!(
        "sequence s (a: buffer in u16, b: buffer in u16, o: buffer out u16)\n",
        "  let (x, xok) = @try_rcv(a)\n",
        "  let (y, yok) = @try_rcv(b)\n",
        "  var t: u16 = 16'd0\n",
        "  if xok then\n",
        "    t = t + x\n",
        "  if yok then\n",
        "    t = t + y\n",
        "  |||\n",
        "  @send(o, t)\n",
    );
    let mut c = Circuit::new(src, "s");
    for _ in 0..20 {
        c.tick();
        assert_eq!(c.out("o_wsalt"), 0, "a head with nothing must not produce");
    }
    // One item on either input is enough to fire it once.
    c.set("b_wsalt", 1);
    c.set("b_data", 7);
    c.tick();
    assert_eq!(c.out("b_rsalt"), 1);
    c.tick();
    assert_eq!(c.out("o_wsalt"), 1);
    assert_eq!(c.out("o_data") & 0xffff, 7);
    assert_eq!(c.out("a_rsalt"), 0, "the input that had nothing was not stepped");
    for _ in 0..10 {
        c.tick();
        assert_eq!(c.out("o_wsalt"), 1, "and then it went quiet again");
    }
}
