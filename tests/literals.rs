// What a numeric literal is worth, and when it is refused.
//
// The lexer accumulates digits into a `u128`. It used to do so with
// `wrapping_mul`/`wrapping_add`, which made the storage limit invisible:
// `2^128` became `0`, and zero fits every width, so the width check that was
// supposed to catch an oversized literal was handed a number that had already
// lost the evidence. Both profiles compiled it and emitted `8'd0`.
//
// The boundary is therefore tested from both sides in every radix, and for
// sized literals as well as unsized ones.

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

/// `u128::MAX` and one more than it, per radix.
const AT_LIMIT: [&str; 4] = [
    "340282366920938463463374607431768211455",
    "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
    "0b11111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "0o3777777777777777777777777777777777777777777",
];
const OVER_LIMIT: [&str; 4] = [
    "340282366920938463463374607431768211456",
    "0x100000000000000000000000000000000",
    "0b100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    "0o4000000000000000000000000000000000000000000",
];

#[test]
fn the_largest_literal_that_fits_is_accepted_in_every_radix() {
    for lit in AT_LIMIT {
        let v = compile(&format!("fun f (o: out u128)\n  o = {}\n", lit));
        assert!(
            v.contains("128'hFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            "{} should be u128::MAX\n{}",
            lit,
            v
        );
    }
}

#[test]
fn a_literal_past_the_storage_limit_is_refused_in_every_radix() {
    for lit in OVER_LIMIT {
        let e = compile_err(&format!("fun f (o: out u128)\n  o = {}\n", lit));
        assert!(e.contains("does not fit in 128 bits"), "{} gave:\n{}", lit, e);
    }
}

#[test]
fn an_oversized_literal_is_refused_rather_than_wrapped_to_a_fitting_one() {
    // The exact shape of the bug: the value wrapped to 0, 0 fits `u8`, and the
    // file compiled to `assign o = 8'd0` with exit code 0.
    let e = compile_err(
        "fun f (o: out u8)\n  o = 340282366920938463463374607431768211456\n",
    );
    assert!(e.contains("does not fit in 128 bits"), "{}", e);
}

#[test]
fn a_sized_literal_overflows_on_its_value_and_on_its_width() {
    let value = compile_err(&format!(
        "fun f (o: out u8)\n  o = 8'd{}\n",
        OVER_LIMIT[0]
    ));
    assert!(value.contains("does not fit in 128 bits"), "{}", value);

    // The WIDTH is scanned by the same routine. An overflowed width used to
    // wrap before the `> u32::MAX` bound could judge it.
    let width = compile_err(&format!("fun f (o: out u8)\n  o = {}'d1\n", OVER_LIMIT[0]));
    assert!(!width.is_empty(), "an unusable width must be refused");
}

#[test]
fn a_zero_width_sized_literal_is_refused() {
    // `u0` is not a type, but the literal path built `Ty::UInt(0)` straight
    // from the digits before the tick and skipped that rule. The compiler
    // exited 0 and emitted `(0'd0 == 0'd0)`, which `vlog` rejects with
    // "Incorrect size constant for integer literal" -- a successful build that
    // produced invalid Verilog.
    let e = compile_err("fun f (o: out u1)\n  let z = 0'd0\n  o = z == z\n");
    assert!(e.contains("cannot be zero bits wide"), "{}", e);
}

#[test]
fn a_one_bit_sized_literal_is_still_fine() {
    // The narrow end of the range that IS valid, so the check above cannot
    // have been written one off.
    let v = compile("fun f (o: out u1)\n  o = 1'd1\n");
    assert!(v.contains("1'b1"), "{}", v);
}

#[test]
fn a_sized_literal_of_the_wrong_width_is_a_width_error() {
    // The strict-width rule's one exception is the UNSIZED literal, which
    // never claimed a width and so may take one. `16'd1` claimed sixteen bits.
    // After lowering both are just `Op::Const`, the distinction was gone, and
    // `a + 16'd1` on a `u8` compiled to `a + 8'd1` -- the silent truncation
    // the rule exists to forbid.
    let e = compile_err("fun f (a: u8, o: out u8)\n  o = a + 16'd1\n");
    assert!(e.contains("width mismatch"), "{}", e);
}

#[test]
fn an_unsized_literal_still_takes_its_width_from_context() {
    let v = compile("fun f (a: u8, o: out u8)\n  o = a + 1\n");
    assert!(v.contains("a + 8'd1"), "{}", v);
}

#[test]
fn a_sized_literal_of_the_right_width_is_unaffected() {
    let v = compile("fun f (a: u8, o: out u8)\n  o = a + 8'd1\n");
    assert!(v.contains("a + 8'd1"), "{}", v);
}
