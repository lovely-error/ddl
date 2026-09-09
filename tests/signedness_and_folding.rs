// Regressions for the miscompilations found by RTL differential probing.
//
// Every case here was silently wrong in emitted Verilog and passed the whole
// suite while it was. They are grouped by the property that broke rather than
// by the file that was repaired, because each repair is in a different file and
// the property is what has to hold.
//
// The RTL benches that found them need Questa and are not part of this suite.
// These are the text-level guards that fail on a laptop, in a second, when a
// repair regresses.

use ddl::diag::SourceMap;
use ddl::driver::compile_to_verilog;
use ddl::ir_export::ExportFlags;
use ddl::verilog::EmitOptions;

fn compile(src: &str) -> String {
    compile_exporting(src, &[])
}

fn compile_exporting(src: &str, targets: &[&str]) -> String {
    let map = SourceMap::new("t.ddl", src);
    let opts = EmitOptions {
        export: ExportFlags {
            export: targets.iter().map(|s| s.to_string()).collect(),
            bare: Vec::new(),
        },
        ..EmitOptions::default()
    };
    match compile_to_verilog(&map, &opts) {
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

/// The line a `fun` drives its output on, which is where a constant that went
/// wrong shows up.
fn output_line(v: &str) -> String {
    v.lines()
        .find(|l| l.trim_start().starts_with("assign o = "))
        .unwrap_or_else(|| panic!("no output assignment in:\n{}", v))
        .trim()
        .to_string()
}

// ---- a signed constant keeps its signedness ------------------------------

#[test]
fn signed_division_by_a_constant_is_a_signed_division() {
    // `render_const` has no `'sd` form and constants were always folded, so
    // this emitted `x / 8'd2` -- and Verilog makes one unsigned operand turn
    // the whole expression unsigned. At x = -4 the hardware answered 126.
    let v = compile("fun f (x: i8, o: out i8)\n  o = x / 2\n");
    assert!(
        !v.contains("x / 8'd2"),
        "the divisor is inline and unsigned, so the divide is unsigned:\n{}",
        v
    );
    assert!(
        v.contains("wire signed [7:0] n") && v.contains("8'd2"),
        "the divisor should be a signed wire:\n{}",
        v
    );
}

#[test]
fn a_signed_remainder_by_a_constant_is_signed_too() {
    let v = compile("fun f (x: i8, o: out i8)\n  o = x % 8\n");
    assert!(!v.contains("x % 8'd8"), "{}", v);
}

#[test]
fn an_unsigned_constant_is_still_folded_inline() {
    // The guard is signedness, not constants: an unsigned one has no bus to
    // eliminate and reads better where it is used.
    let v = compile("fun f (x: u8, o: out u8)\n  o = x / 2\n");
    assert!(v.contains("x / 8'd2"), "{}", v);
}

// ---- a negative literal is a negative number -----------------------------

#[test]
fn a_negative_literal_compares_as_a_negative_number() {
    // `-4` lowered as a negation of `4` kept the operand's provisional type,
    // which is as narrow as possible and UNSIGNED: `u3` holding `0b100`. Every
    // widening after that was a zero extension, so `x <= -4` asked `x <= 4`.
    // A comparison never checks a width or a signedness, so nothing said so.
    let v = compile("fun f (x: i8, o: out u1)\n  o = x <= -4\n");
    assert!(v.contains("8'hFC"), "-4 should reach the wire as 8'hFC:\n{}", v);
    assert!(!v.contains("-3'd4"), "negated at three bits, which is +4:\n{}", v);
}

#[test]
fn a_negative_literal_takes_a_signed_binding() {
    // It could not before: `-4` was `u3`, so this was "cannot assign `u3` to
    // `k`". The compiler forced the bit pattern to be spelled out by hand
    // while accepting the bare negative in a comparison beside it.
    let v = compile("fun f (o: out i8)\n  let k: i8 = -4\n  o = k\n");
    assert!(v.contains("8'hFC"), "{}", v);
}

#[test]
fn a_negative_literal_has_no_unsigned_form() {
    let text = compile_err("fun f (o: out u8)\n  let k: u8 = -1\n  o = k\n");
    assert!(text.contains("u8"), "{}", text);
}

#[test]
fn negating_a_negative_literal_gives_a_positive_one() {
    // The folding above reads the operand as a magnitude, so without a case
    // for an already-negative one `- -4` folded to -4 a second time.
    let v = compile("fun f (x: i8, o: out u1)\n  o = x <= - -4\n");
    assert!(v.contains("9'd4"), "`- -4` is +4:\n{}", v);
    assert!(!v.contains("hFC"), "{}", v);
}

#[test]
fn a_negative_literal_widens_by_sign_extension() {
    let v = compile("fun f (x: i16, o: out u1)\n  o = x < -100\n");
    assert!(v.contains("16'hFF9C"), "-100 in sixteen bits is FF9C:\n{}", v);
}

// ---- a folded constant means what it says, everywhere ---------------------

/// `4'd4 * 4'd4` is sixteen. Folding kept four bits for the product and got
/// zero, because `joined_width` did not know a product is as wide as both
/// operands together.
const MUL16: &str = "4'd4 * 4'd4";

#[test]
fn a_folded_product_reaches_a_loop_bound_intact() {
    // A `for` bound has no `let`-bound sibling to disagree with, so this was
    // simply a sixteen-iteration loop compiling to zero iterations.
    let v = compile(&format!(
        "fun f (x: u8, o: out u8)\n  var acc: u8 = 8'd0\n  for i in 0..({})\n    acc = acc + x\n  o = acc\n",
        MUL16
    ));
    assert_eq!(v.matches("+ x").count(), 16, "sixteen iterations:\n{}", v);
}

#[test]
fn a_folded_product_reaches_a_bit_select_intact() {
    let v = compile(&format!("fun f (x: u32, o: out u1)\n  o = x[{}]\n", MUL16));
    assert!(v.contains("x[16]"), "{}", v);
}

#[test]
fn a_folded_product_reaches_a_bit_range_intact() {
    let v = compile(&format!(
        "fun f (x: u32, o: out u8)\n  o = x[({}) + 7 .. {}]\n",
        MUL16, MUL16
    ));
    assert!(v.contains("x[23:16]"), "{}", v);
}

#[test]
fn a_folded_product_reaches_an_enum_discriminant_intact() {
    // The worst of the four: a discriminant is part of the type, so every
    // match, comparison and constructor in the program agrees on the wrong
    // encoding -- and the duplicate-discriminant check compares folded values
    // too, so a collision it caused would not be caught either.
    let v = compile(&format!(
        "enum e: u8\n  A = {}\n  B = 8'd99\nfun f (c: e, o: out u1)\n  o = c == A\n",
        MUL16
    ));
    assert!(v.contains("8'h10"), "the discriminant is sixteen:\n{}", v);
}

#[test]
fn a_folded_product_reaches_an_array_length_intact() {
    let v = compile(&format!(
        "fun f (o: out u8)\n  var a: [u8; {}] = @zeroed()\n  o = a[0]\n",
        MUL16
    ));
    assert!(v.contains("[127:0]"), "sixteen bytes:\n{}", v);
}

#[test]
fn a_constant_expression_picks_what_the_same_number_picks() {
    // The property behind every case above, and the one the recheck asked to
    // be kept: an expression and the number it evaluates to are the same
    // constant. Comparing the two emissions needs no model of the semantics,
    // which is what makes it hold for the next operator to drift as well.
    for (expr, number) in [
        ("4'd4 * 4'd4", "16"),   // a product is as wide as both operands
        ("4'd8 << 8'd1", "0"),   // a shift keeps the LEFT operand's four bits
        ("8'd200 >> 4'd3", "25"),
        ("5'd20 / 5'd2", "10"),
        ("5'd29 % 5'd12", "5"),
        ("4'd2 - 4'd8", "10"),
        ("~5'd4", "27"),
        ("-4'd4", "12"),
        ("2 ** 4", "16"),
    ] {
        let from_expr = output_line(&compile(&format!(
            "fun f (a: [u8; 32], o: out u8)\n  o = a[{}]\n",
            expr
        )));
        let from_number = output_line(&compile(&format!(
            "fun f (a: [u8; 32], o: out u8)\n  o = a[{}]\n",
            number
        )));
        assert_eq!(
            from_expr, from_number,
            "`{}` is {}, but it does not select what {} selects",
            expr, number, number
        );
    }
}

// ---- storage keeps the out-of-range contract too --------------------------

const LUT12: &str = concat!(
    "process f (idx: buffer in u4, val: buffer out u16)\n",
    "  var t: #[impl(lutram)] [u16; 12] = @zeroed()\n",
    "  loop\n",
    "    let i = @rcv(idx)\n",
    "    let v = t[i]\n",
    "    @send(val, v)\n",
);

#[test]
fn an_asynchronous_read_past_the_end_answers_zero() {
    // docs/overview.md promises an out-of-range read is zero. A packed array
    // kept that; storage did not, because `fit_address` widens and diagnoses
    // but never range-checks. A depth-12 memory read at 12 was `t[12]` on a
    // `reg [15:0] t [0:11]` -- `x`, and unconstrained in synthesis.
    let v = compile(LUT12);
    assert!(v.contains("< 4'hC"), "no range guard:\n{}", v);
    assert!(v.contains("16'd0"), "no zero answer:\n{}", v);
}

#[test]
fn a_synchronous_read_past_the_end_answers_zero() {
    // The address is gone by the time `_q` is readable, so the decision has to
    // ride the read's own clock edge.
    let v = compile(concat!(
        "process f (idx: buffer in u4, val: buffer out u16)\n",
        "  var t: #[impl(bram)] [u16; 12]\n",
        "  loop\n",
        "    let i = @rcv(idx)\n",
        "    let v = t[i]\n",
        "    @send(val, v)\n",
    ));
    assert!(v.contains("inrange"), "no registered range bit:\n{}", v);
    assert!(v.contains("< 4'hC"), "{}", v);
}

#[test]
fn a_pipeline_read_past_the_end_answers_zero() {
    let v = compile(concat!(
        "sequence f (src: buffer in u4, dst: buffer out u16)\n",
        "  var m: #[impl(bram)] [u16; 12]\n",
        "  let a = @rcv(src)\n",
        "  let v = m[a]\n",
        "  |||\n",
        "  @send(dst, v)\n",
    ));
    assert!(v.contains("inrange"), "no crossing range bit:\n{}", v);
}

#[test]
fn a_power_of_two_depth_costs_no_guard() {
    // The guard is emitted only when the address can name a missing element,
    // which is the same rule a packed array's dynamic index follows.
    let v = compile(&LUT12.replace("[u16; 12]", "[u16; 16]"));
    assert!(
        !v.contains("inrange") && !v.contains(" < 4'"),
        "a full address space needs no guard:\n{}",
        v
    );
}

// ---- an enum compares with its own type -----------------------------------

#[test]
fn two_different_enums_do_not_compare() {
    // An enum is scalar, so the general comparison rule accepted it against
    // any other scalar and the same-enum check above it never rejected
    // anything. `small == big` compiled, zero-extending one to the other.
    let text = compile_err(concat!(
        "enum small: u2\n  S0\n  S1\n",
        "enum big: u8\n  B0 = 8'd3\n  B1 = 8'd200\n",
        "fun f (p: small, q: big, o: out u1)\n  o = p == q\n",
    ));
    assert!(text.contains("cannot be compared"), "{}", text);
}

#[test]
fn an_enum_still_compares_with_its_own_type_and_with_a_literal() {
    // Both must keep working: the first is what enums are for, and the second
    // is the unsized-literal rule, which says a literal that never claimed a
    // width takes its partner's.
    let v = compile(concat!(
        "enum k: u4\n  K0\n  K1\n  K2\n",
        "fun f (p: k, o: out u1, r: out u1)\n  o = p == K1\n  r = p == 2\n",
    ));
    assert!(v.contains("assign o"), "{}", v);
    assert!(v.contains("assign r"), "{}", v);
}

#[test]
fn an_enum_still_compares_with_a_plain_integer() {
    // Deliberately left alone: examples/k2g_decode.ddl matches a five-bit
    // instruction field against named opcodes sixty-five times, so a
    // same-width integer beside an enum is a shape this language means to
    // allow, whatever a strict reading of docs/overview.md would say.
    let v = compile("enum k: u4\n  K0\n  K1\nfun f (p: k, q: u4, o: out u1)\n  o = p == q\n");
    assert!(v.contains("assign o"), "{}", v);
}

// ---- the file is the targets and what they use ---------------------------

#[test]
fn an_unexported_externs_adapters_are_not_emitted() {
    // The adapter list collected a use for every pipe of every `extern` in the
    // compilation, and the rebuild after pruning put back the ones whose graph
    // had just been thrown away. Nothing downstream catches an unreferenced
    // module either: that is Verilator's MULTITOP, which examples/lint.vlt
    // waives for a reason of its own.
    let v = compile_exporting(
        concat!(
            "sequence wanted (a: buffer in u16, b: buffer out u16)\n",
            "  let x = @rcv(a)\n",
            "  |||\n",
            "  @send(b, x + 16'd1)\n",
            "extern stray (i: buffer in u7, o: buffer out u7)\n",
            "graph unused_graph (p: buffer in u7, q: buffer out u7)\n",
            "  stray(p, q)\n",
        ),
        &["wanted"],
    );
    for line in v.lines().filter(|l| l.starts_with("module ")) {
        let name = line
            .trim_start_matches("module ")
            .trim_end_matches(" (")
            .trim();
        let instantiated = v.contains(&format!("  {} u_", name));
        assert!(
            instantiated || name == "wanted",
            "`{}` is emitted but nothing instantiates it:\n{}",
            name,
            v
        );
    }
}
