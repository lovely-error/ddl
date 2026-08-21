// Blocking waits on a conditional path.
//
// The scheduler used to be a chain: one state per barrier, each falling
// through to the next. A wait inside an `if` was refused, because a chain has
// nowhere to put a fork. It is a graph now, and these tests pin down the two
// things that makes possible and the one thing it must not cost:
//
//   * each arm gets its own states, and both rejoin at what followed the `if`
//   * a branch decided from a just-received value costs no extra cycle
//   * a `var` assigned in a state changes only when that state fires

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

/// The shape this whole change exists for: one command pipe, and what the
/// command says decides which pipe is waited on next.
const RW: &str = concat!(
    "process rw (cmd: buffer in i8, din: buffer in i32, dout: buffer out i32)\n",
    "  var cell: i32 = @zeroed()\n",
    "  loop\n",
    "    let c = @rcv(cmd)\n",
    "    let is_write: i1 = c[0]\n",
    "    if is_write then\n",
    "      let d = @rcv(din)\n",
    "      cell = d\n",
    "    else\n",
    "      @send(dout, cell)\n",
);

#[test]
fn each_arm_gets_its_own_state_and_both_rejoin() {
    let v = compile(RW);

    // Three states: the command wait, the write path, the read path.
    assert!(v.contains("wire in_s0 = state == 2'd0;"), "{}", v);
    assert!(v.contains("wire in_s1 = state == 2'd1;"), "{}", v);
    assert!(v.contains("wire in_s2 = state == 2'd2;"), "{}", v);

    // State 0 forks; both arms come back to 0.
    assert!(v.contains("fire_s0 ? (branch_s0 ? 2'd1 : 2'd2)"), "{}", v);
    assert!(v.contains("fire_s1 ? 2'd0"), "{}", v);
    assert!(v.contains("fire_s2 ? 2'd0"), "{}", v);
}

#[test]
fn a_branch_on_a_just_received_value_costs_no_extra_cycle() {
    // The condition reads `cmd_data` -- the port, in the cycle the receive
    // completes -- and not a registered copy of it a cycle later. That is what
    // the post scope of a barrier state buys, and without it every command
    // decode would cost a cycle it does not need.
    let v = compile(RW);
    assert!(v.contains("wire branch_s0 = cmd_item[0];"), "{}", v);
    assert!(!v.contains("branch_s0 = c_r"), "{}", v);
}

#[test]
fn only_the_arm_that_ran_writes_the_register() {
    // `cell` is assigned on the write path only, so the write path firing is
    // the only thing that may change it.
    let v = compile(RW);
    assert!(v.contains("cell_ <= (fire_s1 ? din_item : cell_);"), "{}", v);
}

#[test]
fn each_pipe_is_only_ready_in_the_state_that_waits_on_it() {
    // The bug this prevents is a process that accepts an item it is not ready
    // to handle, which shows up much later as a dropped transfer.
    let v = compile(RW);
    assert!(v.contains("wire fire_s0 = in_s0 & (!cmd_empty);"), "{}", v);
    assert!(v.contains("wire fire_s1 = in_s1 & (!din_empty);"), "{}", v);
    assert!(v.contains("wire fire_s2 = in_s2 & (!dout_full);"), "{}", v);
}

#[test]
fn a_var_changes_only_when_its_state_fires() {
    // Before the state graph, a `var` assigned inside a blocking loop took its
    // new value EVERY cycle rather than once per iteration: `seen <= seen + 1`
    // with no gate at all. A counter that counted clock ticks instead of
    // items.
    let v = compile(concat!(
        "process counter (src: buffer in i32, dst: buffer out i32)\n",
        "  var seen: i32 = @zeroed()\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    seen = seen + 32'd1\n",
        "    @send(dst, a + seen)\n",
    ));
    assert!(v.contains("seen <= (fire_s0 ? (seen + 32'd1) : seen);"), "{}", v);
}

#[test]
fn an_arm_with_work_and_no_wait_gets_a_state_of_its_own() {
    // Its statements cannot run in the branching state: that state is on both
    // paths, so they would run on the other arm too. A state of its own is a
    // cycle, and the only way to guard them.
    let v = compile(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  var n: i32 = @zeroed()\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    if a[0] then\n",
        "      @send(dst, a)\n",
        "    else\n",
        "      n = n + 32'd1\n",
    ));
    assert!(v.contains("wire in_s2 = state == 2'd2;"), "{}", v);
    // It has no barrier, so it advances the cycle it is entered.
    assert!(v.contains("in_s2 ? 2'd0"), "{}", v);
    assert!(v.contains("n <= (in_s2 ? (n + 32'd1) : n);"), "{}", v);
}

#[test]
fn an_empty_else_falls_straight_through() {
    let v = compile(concat!(
        "process p (go: i1 = 1'b1, src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    if go then\n",
        "      let a = @rcv(src)\n",
        "    @send(dst, 32'd0)\n",
    ));
    assert!(v.contains("module p ("), "{}", v);
    assert!(v.contains("wire fire_s1 ="), "{}", v);
}

#[test]
fn conditionals_nest() {
    let v = compile(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    if a[0] then\n",
        "      if a[1] then\n",
        "        @send(dst, a)\n",
        "      else\n",
        "        @send(dst, 32'd0)\n",
        "    else\n",
        "      @send(dst, 32'd1)\n",
    ));
    assert!(v.contains("wire branch_s0 ="), "{}", v);
    assert!(v.contains("wire branch_s1 ="), "{}", v);
    // The inner branch is decided a cycle after the receive, so it reads the
    // registered copy rather than the port.
    assert!(v.contains("branch_s1 = a_r[1]"), "{}", v);
}

#[test]
fn a_value_received_on_one_arm_can_be_read_after_the_join() {
    let v = compile(concat!(
        "process p (sel: buffer in i1, a: buffer in i32, b: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let s = @rcv(sel)\n",
        "    if s then\n",
        "      let x = @rcv(a)\n",
        "      @send(dst, x)\n",
        "    else\n",
        "      let x = @rcv(b)\n",
        "      @send(dst, x)\n",
    ));
    // `x` is defined on both arms and read after each, so it is one register
    // written by whichever arm fired.
    assert!(v.contains("reg [31:0] x_r;"), "{}", v);
    assert!(v.contains("a_data"), "{}", v);
    assert!(v.contains("b_data"), "{}", v);
}

#[test]
fn a_body_that_runs_once_still_reaches_its_terminal_state_from_either_arm() {
    let v = compile(concat!(
        "process once (src: buffer in i32, dst: buffer out i32)\n",
        "  let a = @rcv(src)\n",
        "  if a[0] then\n",
        "    @send(dst, a)\n",
        "  else\n",
        "    @send(dst, 32'd0)\n",
    ));
    // State 3 is the terminal one: not a state of the program, so nothing
    // drives a handshake in it and the machine parks.
    assert!(v.contains("fire_s1 ? 2'd3"), "{}", v);
    assert!(v.contains("fire_s2 ? 2'd3"), "{}", v);
}

#[test]
fn a_linear_loop_is_scheduled_exactly_as_before() {
    // The graph must not cost anything where there is no fork.
    let v = compile(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    let b = @rcv(src)\n",
        "    @send(dst, a + b)\n",
    ));
    assert!(v.contains("fire_s0 ? 2'd1 : (fire_s1 ? 2'd2 : (fire_s2 ? 2'd0"), "{}", v);
    assert!(!v.contains("branch_s"), "{}", v);
}

#[test]
fn a_match_holding_a_wait_says_which_construct_does_work() {
    let text = compile_err(concat!(
        "enum e: i1\n",
        "  A\n",
        "  B\n",
        "process p (src: buffer in i32, dst: buffer out i32, k: e = A)\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    match k\n",
        "      A =>\n",
        "        @send(dst, a)\n",
        "      B =>\n",
        "        @send(dst, 32'd0)\n",
    ));
    assert!(text.contains("only an `if` can hold a blocking"), "{}", text);
}

#[test]
fn a_wait_in_the_condition_itself_is_refused() {
    let text = compile_err(concat!(
        "process p (src: buffer in i1, dst: buffer out i32)\n",
        "  loop\n",
        "    if @rcv(src) then\n",
        "      @send(dst, 32'd1)\n",
        "    else\n",
        "      @send(dst, 32'd0)\n",
    ));
    assert!(text.contains("cannot contain a blocking"), "{}", text);
}

// ---- non-blocking operations in a state machine ---------------------------

/// The widening-multiply shape: one item in, one cycle, except when a second
/// writeback is owed -- and then nothing new is accepted until it has gone.
const MULW: &str = concat!(
    "process mulw (uops: buffer in i32, wb: buffer out i32)
",
    "  var hi: i32 = @zeroed()
",
    "  loop
",
    "    let u = @rcv(uops)
",
    "    let wide: i1 = u[0]
",
    "    hi = u + u
",
    "    let _s = @try_send(wb, u)
",
    "    if wide then
",
    "      @send(wb, hi)
",
);

#[test]
fn a_try_send_does_not_make_its_state_wait() {
    // The difference between `@try_send` and `@send` in one line of Verilog:
    // state 0 fires on the INPUT's handshake alone, so a sink that is not
    // ready does not hold the state.
    let v = compile(MULW);
    assert!(v.contains("wire fire_s0 = in_s0 & (!uops_empty);"), "{}", v);
    assert!(v.contains("wire fire_s1 = in_s1 & (!wb_full);"), "{}", v);
}

#[test]
fn the_two_cycle_path_refuses_new_work_while_it_finishes() {
    // What the shape is for. `uops_ready` is the receiving state and nothing
    // else, so the second writeback cannot be overtaken by the next item.
    let v = compile(MULW);
    assert!(v.contains("wire fire_s0 = in_s0 & (!uops_empty);"), "{}", v);
}

#[test]
fn a_pipe_offered_in_two_states_is_valid_in_both_and_muxed_by_state() {
    let v = compile(MULW);
    // `fire_s0`, not `in_s0`: the offer is made in the cycle the item
    // arrives, so a packet computed from data that is not there is never
    // published.
    assert!(v.contains("wire wb_take = fire_s1 | fire_s0;"), "{}", v);
    // The value is PUSHED into an entry rather than muxed onto the wire.
    assert!(v.contains("wb_e0 <= "), "{}", v);
    assert!(v.contains("assign wb_data = {wb_e1, wb_e0};"), "{}", v);
}

#[test]
fn neither_side_reaches_the_other_with_a_non_blocking_offer() {
    // The property the whole language is arranged around, and a non-blocking
    // offer is where it would be easiest to lose. Both published values are
    // register reads, so the check is symmetric now: the producer's drivers
    // must not mention the consumer's salt, or the other way round.
    let v = compile(MULW);
    // Anti-vacuous: both salts are in the module, just not in each other.
    assert!(v.contains("wb_rsalt"), "{}", v);
    assert!(v.contains("uops_wsalt"), "{}", v);

    for line in v.lines().filter(|l| l.trim_start().starts_with("assign ")) {
        if line.contains("wb_wsalt") || line.contains("wb_data") {
            assert!(!line.contains("wb_rsalt"), "producer reads the consumer:
{}", line);
        }
        if line.contains("uops_rsalt") {
            assert!(!line.contains("uops_wsalt"), "consumer reads the producer:
{}", line);
        }
    }
}

#[test]
fn the_branch_still_costs_no_extra_cycle() {
    // The offer did not push the branch into a state of its own: it is decided
    // in the cycle the item arrives, as fsm_branch's other tests require.
    let v = compile(MULW);
    assert!(v.contains("wire branch_s0 = uops_item[0];"), "{}", v);
    assert!(v.contains("state <= ((fire_s0 | fire_s1) ? (fire_s0 ? branch_s0"), "{}", v);
}

#[test]
fn a_try_rcv_samples_without_waiting() {
    // The other direction: a state whose barrier is a send may still look at
    // an input, and `got` says whether anything was there.
    let v = compile(concat!(
        "process tap (src: buffer in i32, dst: buffer out i32)
",
        "  var last: i32 = @zeroed()
",
        "  loop
",
        "    let (x, got) = @try_rcv(src)
",
        "    if got then
",
        "      last = x
",
        "    @send(dst, last)
",
    ));
    // The send is what the state waits on; the sample is not.
    assert!(v.contains("!dst_full"), "{}", v);
    assert!(v.contains("wire fire_s0"), "{}", v);
}

#[test]
fn a_body_with_no_blocking_operation_still_takes_them() {
    // The stateless form, unchanged: no state register at all.
    let v = compile(concat!(
        "process a (i: buffer in i32, o: buffer out i32)
",
        "  loop
",
        "    let (u, got) = @try_rcv(i)
",
        "    let _s = @try_send(o, u)
",
    ));
    assert!(!v.contains("state"), "{}", v);
    assert!(v.contains("assign o_wsalt = o_wsalt_q;"), "{}", v);
}
