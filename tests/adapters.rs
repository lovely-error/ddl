//! What the boundary adapters promise, checked by simulating them.
//!
//! These are the load-bearing claim of the FIFO boundary. Everything a person
//! now wires up by hand talks to one of these, so if an adapter drops an item,
//! duplicates one, reorders two, or accepts one it had no room for, the
//! failure lands in someone else's Verilog and looks like a compiler that
//! cannot be trusted.
//!
//! The contract, in both directions:
//!
//!   - nothing moves unless the receiving side said it had room;
//!   - what is offered stays offered, and stays the same, until it is taken;
//!   - items come out in the order they went in, once each.
//!
//! Driven under an irregular stall pattern on both sides at once, because the
//! interesting failures are all at the edges -- full, empty, and the cycle a
//! transfer happens on.

mod common;

use common::Circuit;
use ddl::diag::{DiagSink, SourceMap};
use ddl::ir_adapt::{self, Adapt};
use ddl::symbols::Symbols;
use ddl::ty::Ty;

fn adapter(kind: Adapt, width: u32) -> Circuit {
    let map = SourceMap::new("adapt.ddl", "");
    let syms = Symbols::default();
    let mut sink = DiagSink::new(&map);
    let m = ir_adapt::build(&map, &syms, kind, &Ty::UInt(width), &mut sink)
        .expect("an adapter is built from its shape alone");
    Circuit::from_module(m)
}

/// One end-to-end pass of `n` items through a write adapter into a read
/// adapter, with `stall_in` deciding when the producer offers and `stall_out`
/// when the consumer takes.
///
/// The two are wired the way a wrapper wires them: the write side's salt
/// output drives the read side's salt input, and the read side's `rsalt`
/// comes back. Nothing here is combinational across the pair -- both sides
/// present registers -- so one propagation pass per cycle is exact.
fn round_trip(n: u128, cycles: usize, offer: impl Fn(usize) -> bool, take: impl Fn(usize) -> bool) -> Vec<u128> {
    let mut w = adapter(Adapt::WportToSalt, 32);
    let mut r = adapter(Adapt::SaltToRport, 32);

    let mut next = 0u128;
    let mut got: Vec<u128> = Vec::new();

    for cycle in 0..cycles {
        // The wire between them, evaluated from registers on both sides.
        r.set("i_wsalt", w.out("o_wsalt"));
        r.set("i_data", w.out("o_data"));
        w.set("o_rsalt", r.out("i_rsalt"));

        // Producer: offer the next item when it is this cycle's turn and the
        // adapter says it has room. Offering without room is the caller's
        // error, and the next test makes sure it is refused anyway.
        let writing = offer(cycle) && next < n && w.out("can_receive") != 0;
        w.set("receive_en", writing as u128);
        w.set("data_write_in", if writing { next } else { 0xdead_beef });

        // Consumer: take when it is this cycle's turn and something is there.
        let reading = take(cycle) && r.out("has_data") != 0;
        r.set("drop_item", reading as u128);
        if reading {
            got.push(r.out("data_read_out"));
        }

        if writing {
            next += 1;
        }
        w.tick();
        r.tick();
    }
    got
}

#[test]
fn every_item_comes_out_once_and_in_order() {
    // Irregular on both sides and mutually out of phase, so the pair spends
    // time full, time empty, and time transferring on both edges at once.
    let got = round_trip(24, 400, |c| c % 3 != 2, |c| c % 7 >= 3);
    assert_eq!(got, (0..24).collect::<Vec<u128>>());
}

#[test]
fn a_consumer_that_never_takes_receives_nothing_and_the_producer_stops() {
    // The back-pressure path end to end: with nothing draining, the read
    // adapter fills, its `rsalt` stops moving, the write adapter fills behind
    // it, and `can_receive` goes low and stays low.
    let got = round_trip(24, 200, |_| true, |_| false);
    assert!(got.is_empty(), "{:?}", got);

    let mut w = adapter(Adapt::WportToSalt, 32);
    let mut r = adapter(Adapt::SaltToRport, 32);
    for _ in 0..50 {
        r.set("i_wsalt", w.out("o_wsalt"));
        r.set("i_data", w.out("o_data"));
        w.set("o_rsalt", r.out("i_rsalt"));
        w.set("receive_en", (w.out("can_receive") != 0) as u128);
        w.set("data_write_in", 7);
        r.set("drop_item", 0);
        w.tick();
        r.tick();
    }
    assert_eq!(w.out("can_receive"), 0, "a full pipe still says it has room");
}

#[test]
fn back_to_back_transfers_lose_nothing() {
    // No stalls at all: one item in and one out every cycle it can manage.
    let got = round_trip(32, 200, |_| true, |_| true);
    assert_eq!(got, (0..32).collect::<Vec<u128>>());
}

#[test]
fn what_is_offered_stays_stable_until_it_is_taken() {
    // A consumer is allowed to look at `data_read_out` for as long as it
    // likes before raising `drop_item`. If the value moved underneath it, a
    // reader that registered it a cycle later would get a different item.
    let mut w = adapter(Adapt::WportToSalt, 32);
    let mut r = adapter(Adapt::SaltToRport, 32);

    let mut pushed = 0u128;
    let mut seen: Option<u128> = None;
    for cycle in 0..60 {
        r.set("i_wsalt", w.out("o_wsalt"));
        r.set("i_data", w.out("o_data"));
        w.set("o_rsalt", r.out("i_rsalt"));

        let writing = pushed < 4 && w.out("can_receive") != 0;
        w.set("receive_en", writing as u128);
        w.set("data_write_in", 100 + pushed);

        // Take only every twelfth cycle, so each item is offered for a long
        // stretch first.
        let reading = cycle % 12 == 11 && r.out("has_data") != 0;
        r.set("drop_item", reading as u128);

        if r.out("has_data") != 0 {
            let now = r.out("data_read_out");
            if let Some(before) = seen {
                assert_eq!(before, now, "the offered item changed at cycle {}", cycle);
            }
            seen = Some(now);
        }
        if reading {
            seen = None;
        }
        if writing {
            pushed += 1;
        }
        w.tick();
        r.tick();
    }
}

#[test]
fn a_write_offered_without_room_is_refused() {
    // `receive_en` is gated by `room` inside the adapter as well as outside
    // it. A far side that raises it while full must not corrupt the pipe --
    // otherwise the write pointer walks past the read pointer and items are
    // silently lost.
    //
    // The assertion is CONSERVATION, not just ordering: the items that come
    // out must be exactly the ones the adapter said it had room for. Checking
    // only that the output rises would pass while items went missing, because
    // any subsequence of a rising sequence is still rising.
    let mut w = adapter(Adapt::WportToSalt, 32);
    let mut r = adapter(Adapt::SaltToRport, 32);

    let mut accepted = Vec::new();
    let mut got = Vec::new();
    for cycle in 0..120u128 {
        r.set("i_wsalt", w.out("o_wsalt"));
        r.set("i_data", w.out("o_data"));
        w.set("o_rsalt", r.out("i_rsalt"));

        // Held high the whole time, room or not.
        w.set("receive_en", 1);
        w.set("data_write_in", cycle);
        // What the adapter promised to take this cycle.
        let had_room = w.out("can_receive") != 0;

        let reading = cycle >= 40 && r.out("has_data") != 0;
        r.set("drop_item", reading as u128);
        if reading {
            got.push(r.out("data_read_out"));
        }
        if had_room {
            accepted.push(cycle);
        }
        w.tick();
        r.tick();
    }
    // Everything it took, in order, once each. The pipe may still be holding
    // the last item or two, so the output is a prefix of what was accepted.
    assert!(
        accepted.starts_with(&got),
        "items were lost or corrupted\n  accepted: {:?}\n  got:      {:?}",
        accepted,
        got
    );
    assert!(got.len() + 4 >= accepted.len(), "too much left behind: {:?} vs {:?}", got, accepted);
    assert!(!got.is_empty(), "nothing got through at all");
}

#[test]
fn a_drop_on_an_empty_pipe_does_not_move_the_read_pointer() {
    // The mirror of the previous test. `drop_item` held high against an empty
    // pipe must not advance `rsalt`, or the producer would see a pipe that is
    // permanently full.
    let mut r = adapter(Adapt::SaltToRport, 32);
    r.set("i_wsalt", 0);
    r.set("i_data", 0);
    r.set("drop_item", 1);
    for _ in 0..20 {
        assert_eq!(r.out("has_data"), 0);
        r.tick();
    }
    assert_eq!(r.out("i_rsalt"), 0, "the read pointer moved on an empty pipe");
}

// ---- the master-side adapters, which drive an `extern` --------------------

#[test]
fn the_extern_facing_adapters_present_the_mirror_of_the_exported_ones() {
    // An `extern`'s `buffer in` is written INTO by the graph, so the adapter
    // there is the master: it answers nothing and drives `receive_en` from
    // the flag the foreign module publishes.
    let w = adapter(Adapt::SaltToWport, 32);
    let dirs: Vec<(String, bool)> = w
        .m
        .ports
        .iter()
        .map(|p| (p.name.clone(), matches!(p.dir, ddl::ir::PortDir::In)))
        .collect();
    let is_input = |n: &str| dirs.iter().find(|(name, _)| name == n).map(|(_, i)| *i);
    assert_eq!(is_input("can_receive"), Some(true), "{:?}", dirs);
    assert_eq!(is_input("receive_en"), Some(false), "{:?}", dirs);
    assert_eq!(is_input("data_write_in"), Some(false), "{:?}", dirs);

    let r = adapter(Adapt::RportToSalt, 32);
    let dirs: Vec<(String, bool)> = r
        .m
        .ports
        .iter()
        .map(|p| (p.name.clone(), matches!(p.dir, ddl::ir::PortDir::In)))
        .collect();
    let is_input = |n: &str| dirs.iter().find(|(name, _)| name == n).map(|(_, i)| *i);
    assert_eq!(is_input("has_data"), Some(true), "{:?}", dirs);
    assert_eq!(is_input("drop_item"), Some(false), "{:?}", dirs);
    assert_eq!(is_input("data_read_out"), Some(true), "{:?}", dirs);
}

#[test]
fn a_master_write_adapter_only_writes_when_the_far_side_can_receive() {
    // It holds `receive_en` low while the foreign module says it cannot
    // receive, however much it has waiting.
    let mut a = adapter(Adapt::SaltToWport, 32);
    // A full input pipe: wsalt two gray steps ahead of a reset rsalt.
    a.set("i_wsalt", 3);
    a.set("i_data", 0);
    a.set("can_receive", 0);
    for _ in 0..10 {
        assert_eq!(a.out("receive_en"), 0, "wrote into a module that said it could not receive");
        a.tick();
    }
    a.set("can_receive", 1);
    assert_eq!(a.out("receive_en"), 1, "did not write when the far side was ready");
}

// ---- the wrapper, as a whole ----------------------------------------------
//
// An export target's wrapper is three instances: a write adapter, the lowered
// logic, and a read adapter. Testing the adapters alone leaves the join
// between them untested, and the join is the part nobody has ever exercised.
//
// These rebuild that netlist and drive it through its FIFO ports, which is
// what someone instantiating the module actually does.

use common::modules;

const MUL3: &str = "sequence mul3 (src: buffer in u16, dst: buffer out u32)\n  let x = @rcv(src)\n  |||\n  let doubled: u16 = x + x\n  let wide: u32 = {16'd0, doubled}\n  let scaled: u32 = wide + wide\n  @send(dst, scaled)\n";

/// The wrapper's netlist, driven for `cycles` with the given stall patterns.
fn through_wrapper(
    n: u128,
    cycles: usize,
    offer: impl Fn(usize) -> bool,
    take: impl Fn(usize) -> bool,
) -> Vec<u128> {
    let core_m = modules(MUL3).into_iter().find(|m| m.name == "mul3").expect("lowered");
    let mut core = Circuit::from_module(core_m);
    let mut w = adapter(Adapt::WportToSalt, 16);
    let mut r = adapter(Adapt::SaltToRport, 32);

    let mut next = 0u128;
    let mut got = Vec::new();
    for cycle in 0..cycles {
        // Every signal crossing an instance boundary is a register output on
        // the side that drives it, so one propagation pass per cycle is exact.
        core.set("src_wsalt", w.out("o_wsalt"));
        core.set("src_data", w.out("o_data"));
        w.set("o_rsalt", core.out("src_rsalt"));

        r.set("i_wsalt", core.out("dst_wsalt"));
        r.set("i_data", core.out("dst_data"));
        core.set("dst_rsalt", r.out("i_rsalt"));

        let writing = offer(cycle) && next < n && w.out("can_receive") != 0;
        w.set("receive_en", writing as u128);
        w.set("data_write_in", next);

        let reading = take(cycle) && r.out("has_data") != 0;
        r.set("drop_item", reading as u128);
        if reading {
            got.push(r.out("data_read_out"));
        }

        if writing {
            next += 1;
        }
        w.tick();
        core.tick();
        r.tick();
    }
    got
}

#[test]
fn the_wrapper_carries_every_item_through_the_logic_it_wraps() {
    // `mul3` multiplies by four. Each item has to come out once, in order,
    // and carrying its own answer rather than a neighbour's -- which is the
    // failure a wrapper that mis-registered a crossing would produce, and
    // which an ordering check alone would not see.
    let got = through_wrapper(20, 400, |c| c % 3 != 2, |c| c % 5 >= 2);
    let want: Vec<u128> = (0..20).map(|i| i * 4).collect();
    assert_eq!(got, want);
}

#[test]
fn the_wrapper_holds_everything_when_nothing_drains_it() {
    let got = through_wrapper(20, 300, |_| true, |_| false);
    assert!(got.is_empty(), "{:?}", got);
}

#[test]
fn the_wrapper_runs_back_to_back_with_no_stalls() {
    let got = through_wrapper(20, 300, |_| true, |_| true);
    let want: Vec<u128> = (0..20).map(|i| i * 4).collect();
    assert_eq!(got, want);
}
