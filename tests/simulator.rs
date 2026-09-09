// The behavioural simulator, checked against what the backend actually emits.
//
// `tests/common/mod.rs` is the oracle several suites judge lowered IR with, so
// a wrong answer there does not fail -- it passes the wrong thing. Division
// and remainder ignored signedness and folded in `u128`, which made `i8`
// `-4 / 2` come out as 126 where the hardware answers 254 (`-2`). The backend
// emits a typed Verilog operator, so the oracle and the thing it was checking
// disagreed, quietly, in the oracle's favour.

mod common;

use common::{Circuit, mask, signed};

fn eval_bin(op: &str, ty: &str, a: u128, b: u128) -> u128 {
    let src = format!(
        "fun p (x: {t}, y: {t}, o: out {t})\n  o = x {op} y\n",
        t = ty,
        op = op
    );
    let mut c = Circuit::new(&src, "p");
    c.set("x", a);
    c.set("y", b);
    c.out("o")
}

#[test]
fn signed_division_matches_two_s_complement() {
    // -4 / 2 == -2. In eight bits: 0xFC / 0x02 == 0xFE.
    assert_eq!(eval_bin("/", "i8", 0xFC, 0x02), 0xFE);
    // -8 / -2 == 4.
    assert_eq!(eval_bin("/", "i8", 0xF8, 0xFE), 4);
    // 7 / -2 == -3, truncating toward zero as Verilog does.
    assert_eq!(eval_bin("/", "i8", 7, 0xFE), 0xFD);
}

#[test]
fn signed_remainder_takes_the_sign_of_the_dividend() {
    // -7 % 2 == -1.
    assert_eq!(eval_bin("%", "i8", 0xF9, 2), 0xFF);
    // 7 % -2 == 1.
    assert_eq!(eval_bin("%", "i8", 7, 0xFE), 1);
}

#[test]
fn unsigned_division_is_unchanged() {
    assert_eq!(eval_bin("/", "u8", 252, 2), 126);
    assert_eq!(eval_bin("%", "u8", 253, 4), 1);
}

#[test]
fn the_most_negative_value_divided_by_minus_one_does_not_panic() {
    // Overflows a two's-complement division in every profile.
    assert_eq!(eval_bin("/", "i8", 0x80, 0xFF), 0x80);
}

#[test]
fn dividing_by_zero_answers_zero_rather_than_trapping() {
    // Documented, not correct: hardware says `x` and this is two-state.
    assert_eq!(eval_bin("/", "i8", 0xFC, 0), 0);
    assert_eq!(eval_bin("/", "u8", 8, 0), 0);
}

#[test]
fn the_width_helpers_hold_at_both_ends() {
    // `128 - w` is a shift by 128 at zero and a negative one past 128, and
    // these are reached with whatever widths the compiler accepted.
    assert_eq!(mask(0), 0);
    assert_eq!(mask(1), 1);
    assert_eq!(mask(8), 0xFF);
    assert_eq!(mask(127), u128::MAX >> 1);
    assert_eq!(mask(128), u128::MAX);
    assert_eq!(mask(200), u128::MAX);

    assert_eq!(signed(0, 0), 0);
    assert_eq!(signed(1, 1), -1);
    assert_eq!(signed(0xFF, 8), -1);
    assert_eq!(signed(0x7F, 8), 127);
    assert_eq!(signed(u128::MAX, 128), -1);
    assert_eq!(signed(u128::MAX, 200), -1);
}
