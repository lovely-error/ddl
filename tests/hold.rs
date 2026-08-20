// `@hold(c)` -- refuse this cycle's input while `c`.
//
// The gap it closes. A process with no blocking operation is one state that
// fires every cycle: its `ready` is "every output slot has room", and the body
// had no say in it. A process that DOES block is a state machine, which can
// decline its input -- every state but the one waiting on it does -- but a
// state waits on one handshake, so a stage that consumes and produces every
// cycle cannot be written that way either.
//
// K2G's execute stage needs both at once: one micro-op per cycle, except when
// a widening multiply owes a second writeback, and then nothing new until it
// has gone (k2g_core.sv:787, where every term of `stall` is a register).
//
// The condition is read BEFORE the body, which is not a restriction dodged but
// the point: `ready` has to be register-derived, and the body's own `fired` is
// downstream of `ready`.

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

/// The widening-multiply shape: one item per cycle, except when a second
/// result is owed.
const WIDEN: &str = concat!(
    "process widen (uops: buffer in i32, wb: buffer out i32)\n",
    "  var pending: i1 = @zeroed()\n",
    "  var lo: i32 = @zeroed()\n",
    "  loop\n",
    "    @hold(pending)\n",
    "    let (u, got) = @try_rcv(uops)\n",
    "    let out_val: i32 = if pending then lo else u + u\n",
    "    let _s = @try_send(wb, out_val)\n",
    "    if pending then\n",
    "      pending = 1'b0\n",
    "    else\n",
    "      if got & u[0] then\n",
    "        pending = 1'b1\n",
    "        lo = u\n",
);

#[test]
fn a_hold_takes_the_input_ready_down() {
    let v = compile(WIDEN);
    assert!(v.contains("wire not_held = !pending;"), "{}", v);
    assert!(v.contains("wire n14 = wb_room & not_held;"), "{}", v);
    assert!(v.contains("assign uops_ready = n14;"), "{}", v);
}

#[test]
fn the_hold_is_register_derived_so_rule_three_survives() {
    // `ready` may look at the process's own state and must not look at the
    // other side's handshake. `pending` is a register, and `valid` is still
    // the slot's own.
    let v = compile(WIDEN);
    let ready = v.lines().find(|l| l.contains("assign uops_ready")).expect("a ready");
    assert!(!ready.contains("wb_ready"), "{}", v);
    assert!(v.contains("assign wb_valid = wb_busy;"), "{}", v);
    // Anti-vacuous: `wb_ready` is in the module, just not on that line.
    assert!(v.contains("wb_ready"), "{}", v);
}

#[test]
fn a_held_cycle_still_produces() {
    // The half that makes it worth having. Without it the slot is written only
    // when an item arrives, so a stage could decline its input to finish a
    // second result and then have nowhere to put it.
    let v = compile(WIDEN);
    assert!(v.contains("wire n27 = uops_xfer | pending;"), "{}", v);
}

#[test]
fn the_output_slot_is_still_there_so_nothing_is_dropped() {
    // Unlike a state machine, where `valid` comes straight from the state and
    // a refused offer is a lost item.
    let v = compile(WIDEN);
    assert!(v.contains("reg wb_skid_busy;"), "{}", v);
    assert!(v.contains("reg [31:0] wb_skid;"), "{}", v);
}

#[test]
fn a_process_without_a_hold_is_unchanged() {
    // Anti-vacuous, and the compatibility claim: `ready` is the slot's room
    // and nothing else.
    let v = compile(concat!(
        "process plain (uops: buffer in i32, wb: buffer out i32)\n",
        "  loop\n",
        "    let (u, got) = @try_rcv(uops)\n",
        "    let _s = @try_send(wb, u)\n",
    ));
    assert!(v.contains("assign uops_ready = wb_room;"), "{}", v);
    assert!(!v.contains("not_held"), "{}", v);
}

#[test]
fn a_hold_on_a_branch_is_refused() {
    // It decides whether this cycle's input is accepted, so it is read before
    // the body runs and cannot sit on a path.
    let text = compile_err(concat!(
        "process bad (uops: buffer in i32, wb: buffer out i32)\n",
        "  var pending: i1 = @zeroed()\n",
        "  loop\n",
        "    let (u, got) = @try_rcv(uops)\n",
        "    if got then\n",
        "      @hold(pending)\n",
        "    let _s = @try_send(wb, u)\n",
    ));
    assert!(text.contains("`@hold` belongs at the top of a `loop`"), "{}", text);
}

#[test]
fn a_hold_needs_an_i1() {
    let text = compile_err(concat!(
        "process bad (uops: buffer in i32, wb: buffer out i32)\n",
        "  var n: i32 = @zeroed()\n",
        "  loop\n",
        "    @hold(n)\n",
        "    let (u, got) = @try_rcv(uops)\n",
        "    let _s = @try_send(wb, u)\n",
    ));
    assert!(text.contains("`@hold` takes an `i1`"), "{}", text);
}

#[test]
fn a_hold_cannot_read_what_the_body_computes() {
    // The circularity, refused by scoping rather than by a special case: the
    // condition is lowered before the body, so a name the body binds is not
    // in scope yet. A hold that could read `got` would be a combinational
    // loop through `ready`.
    let text = compile_err(concat!(
        "process bad (uops: buffer in i32, wb: buffer out i32)\n",
        "  loop\n",
        "    @hold(got)\n",
        "    let (u, got) = @try_rcv(uops)\n",
        "    let _s = @try_send(wb, u)\n",
    ));
    assert!(text.contains("`got` is not defined"), "{}", text);
}
