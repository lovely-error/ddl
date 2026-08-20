// `for i in 0..n`, which unrolls.
//
// There is no counter in the hardware unless something asks for one, and a
// `for` with a static trip count is not asking. It is a way to write the same
// wiring n times without writing it n times, so what comes out has to be
// exactly what the hand-written repetition would have been -- constant bit
// selects, constant addresses, no residue of the loop at all.

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

#[test]
fn a_range_unrolls_into_one_copy_per_iteration() {
    let v = compile(concat!(
        "fun popcount8 (v: i8, n: out i4)\n",
        "  var acc: i4 = @zeroed()\n",
        "  for i in 0..8\n",
        "    acc = acc + @zext(v[i], 4)\n",
        "  n = acc\n",
    ));
    for bit in 0..8 {
        assert!(v.contains(&format!("v[{}]", bit)), "bit {} missing:\n{}", bit, v);
    }
    // Eight adds, and no trace of a loop variable.
    assert_eq!(v.matches(" + ").count(), 8, "{}", v);
    assert!(!v.contains("wire i_"), "{}", v);
}

#[test]
fn the_index_is_a_constant_bit_select_not_a_part_select() {
    // `v[i]` where `i` is the induction variable folds to `v[3]`. A
    // part-select with a constant base would be correct and is exactly the
    // construct this backend exists to keep away from GowinSynthesis.
    let v = compile(concat!(
        "fun pick (v: i8, o: out i1)\n",
        "  var acc: i1 = @zeroed()\n",
        "  for i in 3..4\n",
        "    acc = v[i]\n",
        "  o = acc\n",
    ));
    assert!(v.contains("v[3]"), "{}", v);
    assert!(!v.contains("+: 1"), "{}", v);
}

#[test]
fn the_loop_variable_takes_the_width_of_what_it_meets() {
    // Typed as narrowly as its value allows, like an unsized literal, so
    // `acc + i` adopts `acc`'s width rather than demanding a cast per use.
    let v = compile(concat!(
        "fun total (o: out i8)\n",
        "  var acc: i8 = @zeroed()\n",
        "  for i in 0..4\n",
        "    acc = acc + i\n",
        "  o = acc\n",
    ));
    assert!(v.contains("8'd1"), "{}", v);
    assert!(v.contains("8'd3"), "{}", v);
}

#[test]
fn a_constant_parameter_can_be_the_bound() {
    // A process's plain parameter is folded at elaboration, so it is already a
    // constant by the time a body is lowered. Refusing it would make every
    // parameterised design write its sizes twice.
    let v = compile(concat!(
        "process p (n: i8 = 4, src: buffer in i8, dst: buffer out i8)\n",
        "  loop\n",
        "    let (v, ok) = @try_rcv(src)\n",
        "    var acc: i8 = @zeroed()\n",
        "    for i in 0..n\n",
        "      acc = acc + @zext(v[i], 8)\n",
        "    @try_send(dst, acc)\n",
    ));
    assert!(v.contains("src_data[3]"), "{}", v);
    assert!(!v.contains("src_data[4]"), "{}", v);
}

#[test]
fn naming_an_array_walks_its_indices() {
    // desc.md:68. The binding is the INDEX rather than a copy of the element:
    // a memory element is reached by subscript, and handing back a copy would
    // hide that every read is a port.
    let v = compile(concat!(
        "process p (src: buffer in i8, dst: buffer out i8)\n",
        "  var mem: #[impl(lutram)] [i8; 4] = @zeroed()\n",
        "  loop\n",
        "    let (x, ok) = @try_rcv(src)\n",
        "    var acc: i8 = @zeroed()\n",
        "    for i in mem\n",
        "      acc = acc + mem[i]\n",
        "    @try_send(dst, acc)\n",
    ));
    for addr in 0..4 {
        assert!(v.contains(&format!("mem[2'd{}]", addr)), "addr {}:\n{}", addr, v);
    }
}

#[test]
fn an_address_that_folded_to_a_constant_is_written_as_one() {
    // `mem[{{1{1'b0}}, 1'b1}]` is what a zero-extension of a one-bit constant
    // produces, and it is correct. It is also unreadable, and the reader of a
    // generated file has no source to check it against.
    let v = compile(concat!(
        "process p (src: buffer in i8, dst: buffer out i8)\n",
        "  var mem: #[impl(lutram)] [i8; 4] = @zeroed()\n",
        "  loop\n",
        "    let (x, ok) = @try_rcv(src)\n",
        "    @try_send(dst, mem[1])\n",
    ));
    assert!(v.contains("mem[2'd1]"), "{}", v);
    assert!(!v.contains("1'b1}"), "{}", v);
}

#[test]
fn an_empty_range_emits_nothing() {
    let v = compile(concat!(
        "fun f (o: out i8)\n",
        "  var acc: i8 = @zeroed()\n",
        "  for i in 4..4\n",
        "    acc = acc + 8'd1\n",
        "  o = acc\n",
    ));
    assert!(v.contains("assign o = 8'd0;"), "{}", v);
}

#[test]
fn loops_nest() {
    let v = compile(concat!(
        "fun f (o: out i8)\n",
        "  var acc: i8 = @zeroed()\n",
        "  for i in 0..2\n",
        "    for j in 0..3\n",
        "      acc = acc + 8'd1\n",
        "  o = acc\n",
    ));
    assert_eq!(v.matches("8'd1").count(), 6, "{}", v);
}

#[test]
fn the_binding_does_not_leak_past_the_loop() {
    let text = compile_err(concat!(
        "fun f (o: out i8)\n",
        "  var acc: i8 = @zeroed()\n",
        "  for i in 0..2\n",
        "    acc = acc + 8'd1\n",
        "  o = i\n",
    ));
    assert!(text.contains("`i`"), "{}", text);
}

#[test]
fn an_outer_name_is_shadowed_and_then_restored() {
    let v = compile(concat!(
        "fun f (o: out i8)\n",
        "  let i: i8 = 8'd100\n",
        "  var acc: i8 = @zeroed()\n",
        "  for i in 0..2\n",
        "    acc = acc + i\n",
        "  o = acc + i\n",
    ));
    // The loop saw 0 and 1; the tail sees 100 again, rendered in hex as the
    // backend renders anything wider than a small decimal.
    assert!(v.contains("8'h64"), "{}", v);
}

#[test]
fn a_bound_that_is_not_constant_says_why_it_has_to_be() {
    let text = compile_err(concat!(
        "fun f (v: i8, o: out i8)\n",
        "  var acc: i8 = @zeroed()\n",
        "  for i in 0..v\n",
        "    acc = acc + 8'd1\n",
        "  o = acc\n",
    ));
    assert!(text.contains("not known at compile time"), "{}", text);
    assert!(text.contains("unrolled"), "{}", text);
}

#[test]
fn a_backwards_range_is_an_error_rather_than_an_empty_loop() {
    let text = compile_err(concat!(
        "fun f (o: out i8)\n",
        "  var acc: i8 = @zeroed()\n",
        "  for i in 8..0\n",
        "    acc = acc + 8'd1\n",
        "  o = acc\n",
    ));
    assert!(text.contains("runs backwards"), "{}", text);
}

#[test]
fn an_absurd_trip_count_is_refused_before_it_is_built() {
    // Otherwise the compiler builds the logic first and the bug report is "it
    // stopped responding".
    let text = compile_err(concat!(
        "fun f (o: out i8)\n",
        "  var acc: i8 = @zeroed()\n",
        "  for i in 0..99999\n",
        "    acc = acc + 8'd1\n",
        "  o = acc\n",
    ));
    assert!(text.contains("would unroll 99999 times"), "{}", text);
}

#[test]
fn iterating_something_that_is_neither_says_both_forms() {
    let text = compile_err(concat!(
        "fun f (v: i8, o: out i8)\n",
        "  var acc: i8 = @zeroed()\n",
        "  for i in v\n",
        "    acc = acc + 8'd1\n",
        "  o = acc\n",
    ));
    assert!(text.contains("iterates a range or an array"), "{}", text);
    assert!(text.contains("for i in 0..n"), "{}", text);
}
