// What "constant" means, and that it means one thing.
//
// Two mechanisms fold compile-time expressions: `const_eval` reads the syntax,
// and `const_operand` lowers first and then asks whether the result is a
// literal. They disagreed in both directions.
//
// On ACCEPTANCE: lowering does not fold `2 * 2`, so an array length and a
// `@slice` width took it and a `for` bound did not.
//
// On MEANING: `const_eval` computed in `u128` and ignored the widths its
// literals carried, so `a[8'd255 + 8'd1]` was index 256 and rejected, while
// `let i = 8'd255 + 8'd1` then `a[i]` was an eight-bit sum that wrapped to 0
// and read element 0. Naming a subexpression changed the program.

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

#[test]
fn the_same_constant_expression_is_constant_everywhere() {
    // An array length, a `@slice` width and a `for` bound are all compile-time
    // positions. `2 * 2` belongs in all three or none.
    compile("fun f (a: [u8; 2 * 2], o: out u8)\n  o = a[0]\n");
    compile("fun f (a: u32, b: u5, o: out u4)\n  o = @slice(a, b, 2 * 2)\n");
    compile(concat!(
        "fun f (a: [u8; 4], o: out u8)\n",
        "  var s: u8 = 8'd0\n",
        "  for i in 0..(2 * 2)\n",
        "    s = s + a[i]\n",
        "  o = s\n",
    ));
}

#[test]
fn sized_literal_arithmetic_wraps_the_same_way_folded_or_lowered() {
    // `8'd255 + 8'd1` is 0 in eight bits. Folding it must not say 256.
    let folded = compile("fun f (a: [u8; 4], o: out u8)\n  o = a[8'd255 + 8'd1]\n");
    assert!(folded.contains("a[7:0]"), "should select element 0\n{}", folded);

    // The same expression through a binding reaches element 0 too: the sum
    // wraps to 0 at runtime and the bound holds.
    let lowered = compile(concat!(
        "fun f (a: [u8; 4], i_unused: u1, o: out u8)\n",
        "  let i = 8'd255 + 8'd1\n",
        "  o = a[i]\n",
    ));
    assert!(lowered.contains("8'hFF + 8'd1"), "{}", lowered);
}

#[test]
fn unsized_literals_stay_in_the_mathematical_domain() {
    // An array length is written with unsized literals and must not wrap at
    // some incidental width: `4 * 64` is 256, not 0.
    let v = compile("fun f (a: [u8; 4 * 64], o: out u8)\n  o = a[4 * 64 - 1]\n");
    assert!(v.contains("a[2047:2040]"), "element 255 of 256\n{}", v);
}
