// `[T; n]` as a VALUE, rather than as storage.
//
// A memory is an unpacked array that only a subscript can reach and that lives
// inside one process (tests/memory.rs). This file is the other half: an
// `[T; n]` with no `#[impl(...)]`, which is a packed vector and can therefore
// be a struct field, a pipe payload or a parameter.
//
// Subscripting one used to fall through to the bit selects, so `line.words[1]`
// read bit 1 of the flattened struct and typed as `i1` -- a wrong answer that
// synthesizes, rather than a diagnostic.

use ddl::diag::SourceMap;
use ddl::driver::compile_to_verilog;
use ddl::verilog::EmitOptions;

fn squeeze(v: &str) -> String {
    // Ports are column-aligned, so an assertion about one must not also be an
    // assertion about how wide the widest port happened to be.
    let mut out = String::with_capacity(v.len());
    let mut last_was_space = false;
    for c in v.chars() {
        let is_space = c == ' ';
        if !(is_space && last_was_space) {
            out.push(c);
        }
        last_was_space = is_space;
    }
    out
}

fn compile(src: &str) -> String {
    let map = SourceMap::new("t.ddl", src);
    match compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(v) => squeeze(&v),
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

/// A cache line: a tag and the words behind it, which is the shape that found
/// this.
const LINE: &str = concat!(
    "struct line_t\n",
    "  tag: i17\n",
    "  words: [i32; 4]\n",
    "\n",
);

#[test]
fn a_constant_index_selects_an_element() {
    let v = compile(&format!("{}fun pick (l: line_t, o: out i32)\n  o = l.words[1]\n", LINE));
    // Element 1 of a 4 x i32 field, which sits in the low 128 bits of the
    // struct: bits 63:32, not bit 1.
    assert!(v.contains("[63:32]"), "{}", v);
    assert!(v.contains("output [31:0] o"), "{}", v);
}

#[test]
fn an_element_is_not_a_bit() {
    // The regression itself, stated as the thing that must not come back: a
    // 32-bit element cannot be assigned to an `i1`, and before this it could.
    let text = compile_err(&format!(
        "{}fun pick (l: line_t, o: out i1)\n  o = l.words[1]\n",
        LINE
    ));
    assert!(text.contains("cannot assign `i32`"), "{}", text);
}

#[test]
fn a_computed_index_is_a_part_select_scaled_by_the_element() {
    let v = compile(&format!(
        "{}fun pick (l: line_t, i: i2, o: out i32)\n  o = l.words[i]\n",
        LINE
    ));
    // Widened before scaling: `i` is 2 bits and the base it has to produce is
    // 7, so scaling in the index's own width would shift the top off.
    assert!(v.contains("<< 7'd5"), "{}", v);
    assert!(v.contains("+: 32]"), "{}", v);
}

#[test]
fn a_computed_index_shifts_rather_than_multiplies_where_it_can() {
    // A multiply is a DSP as far as GowinSynthesis is concerned, and an
    // address is the last place to spend one on a constant.
    let v = compile(&format!(
        "{}fun pick (l: line_t, i: i2, o: out i32)\n  o = l.words[i]\n",
        LINE
    ));
    assert!(!v.contains(" * "), "{}", v);
}

#[test]
fn an_element_width_that_is_not_a_power_of_two_multiplies() {
    // The anti-vacuous half of the test above: the shift is a special case,
    // and the general path has to be there and be right.
    let v = compile("fun pick (l: [i3; 5], i: i3, o: out i3)\n  o = l[i]\n");
    assert!(v.contains("* 4'd3"), "{}", v);
    assert!(v.contains("+: 3]"), "{}", v);
}

#[test]
fn a_range_of_elements_is_a_shorter_array() {
    let v = compile(&format!(
        "{}fun pick (l: line_t, o: out [i32; 2])\n  o = l.words[2..1]\n",
        LINE
    ));
    // Elements 2 and 1: bits 95:32, and 64 bits wide.
    assert!(v.contains("[95:32]"), "{}", v);
    assert!(v.contains("output [63:0] o"), "{}", v);
}

#[test]
fn an_index_past_the_end_is_refused() {
    let text = compile_err("fun oob (l: [i8; 4], o: out i8)\n  o = l[7]\n");
    assert!(text.contains("element 7 is out of bounds for `[i8; 4]`"), "{}", text);
    assert!(text.contains("4 element(s)"), "{}", text);
}

#[test]
fn a_range_past_the_end_is_refused() {
    let text = compile_err("fun oob (l: [i8; 4], o: out [i8; 2])\n  o = l[4..3]\n");
    assert!(text.contains("out of bounds"), "{}", text);
}

#[test]
fn an_array_travels_through_a_pipe() {
    // What the struct field was standing in for: a cache line as a payload.
    let v = compile(concat!(
        "process p (a: buffer in [i32; 4], b: buffer out i32)\n",
        "  loop\n",
        "    let c = @rcv(a)\n",
        "    @send(b, c[0])\n",
    ));
    // Two entries on the wire, so the port is twice the payload: 4 x i32 is
    // 128 bits, and a pipe of them is 256.
    assert!(v.contains("input [255:0] a_data"), "{}", v);
    assert!(v.contains("output [63:0] b_data"), "{}", v);
}

#[test]
fn an_unrolled_loop_indexes_by_element() {
    // `i` is not a literal, so `const_eval` misses it; it arrives as a value
    // that folded to a constant and must still select an element rather than
    // becoming a part-select with a constant base.
    let v = compile(concat!(
        "fun total (l: [i8; 4], o: out i8)\n",
        "  var acc: i8 = 8'd0\n",
        "  for i in 0..4\n",
        "    acc += l[i]\n",
        "  o = acc\n",
    ));
    assert!(v.contains("[7:0]"), "{}", v);
    assert!(v.contains("[15:8]"), "{}", v);
    assert!(v.contains("[23:16]"), "{}", v);
    assert!(v.contains("[31:24]"), "{}", v);
    assert!(!v.contains("+:"), "{}", v);
}

// ---- assigning an element -------------------------------------------------
//
// Reading `l.words[1]` was fixed before writing it was, which left the two
// halves of the same subscript disagreeing: `only a name or a field of one can
// be assigned`. A cache line arriving over a burst wants to be filled a beat
// at a time, and that is the whole of this.

#[test]
fn a_constant_element_is_spliced_in_place() {
    let v = compile(&format!(
        "{}{}",
        LINE,
        concat!(
            "fun fill (l: line_t, w: i32, o: out line_t)\n",
            "  var t: line_t = l\n",
            "  t.words[1] = w\n",
            "  o = t\n",
        )
    ));
    // `words` is the low 128 bits of the struct and element 1 is bits 63:32,
    // so the write keeps everything above 63 and everything below 32.
    assert!(v.contains("l[144:64]"), "{}", v);
    assert!(v.contains("l[31:0]"), "{}", v);
    // A constant index costs no comparison.
    assert!(!v.contains("=="), "{}", v);
}

#[test]
fn a_computed_element_muxes_every_slot() {
    let v = compile(concat!(
        "fun put (a: [i8; 4], k: i2, x: i8, o: out [i8; 4])\n",
        "  var t: [i8; 4] = a\n",
        "  t[k] = x\n",
        "  o = t\n",
    ));
    // One comparison per element, and the old value wherever the index misses.
    for slot in ["2'd0", "2'd1", "2'd2", "2'd3"] {
        assert!(v.contains(&format!("k == {}", slot)), "{} missing in {}", slot, v);
    }
    // A write is not a `+:`. Part-select on the left of an assignment is a
    // procedural construct, and this backend emits a value graph.
    assert!(!v.contains("+:"), "{}", v);
}

#[test]
fn an_element_of_a_field_is_reached_through_both_steps() {
    let v = compile(&format!(
        "{}{}",
        LINE,
        concat!(
            "fun fill (l: line_t, k: i2, w: i32, o: out line_t)\n",
            "  var t: line_t = l\n",
            "  t.words[k] = w\n",
            "  o = t\n",
        )
    ));
    // The tag survives untouched above the array it did not name.
    assert!(v.contains("l[144:128]"), "{}", v);
    assert!(v.contains("k == 2'd3"), "{}", v);
}

#[test]
fn an_element_index_past_the_end_is_refused() {
    let text = compile_err(concat!(
        "fun put (a: [i8; 4], x: i8, o: out [i8; 4])\n",
        "  var t: [i8; 4] = a\n",
        "  t[4] = x\n",
        "  o = t\n",
    ));
    assert!(text.contains("out of bounds"), "{}", text);
}

#[test]
fn a_computed_index_must_be_the_last_step() {
    // `a[i].f = x` would need a read-modify-write at an offset only the cycle
    // knows, which is a `+:` on the left of an assignment.
    let text = compile_err(concat!(
        "struct pair_t\n",
        "  lo: i8\n",
        "  hi: i8\n",
        "\n",
        "fun put (a: [pair_t; 4], k: i2, x: i8, o: out [pair_t; 4])\n",
        "  var t: [pair_t; 4] = a\n",
        "  t[k].lo = x\n",
        "  o = t\n",
    ));
    assert!(text.contains("must be the last step"), "{}", text);
    assert!(text.contains("let e = a[i]"), "{}", text);
}

#[test]
fn a_bit_of_an_integer_is_not_an_lvalue() {
    let text = compile_err(concat!(
        "fun put (v: i8, k: i3, o: out i8)\n",
        "  var t: i8 = v\n",
        "  t[k] = 1'b1\n",
        "  o = t\n",
    ));
    assert!(text.contains("is not an array"), "{}", text);
}

// ---- @slice ---------------------------------------------------------------
//
// A multi-bit part-select at a computed base. The MECHANISM was always here --
// an array element at a computed index lowers to a `+:` of the element width
// -- but there was no way to write one on a plain `iN`, so the workaround was
// to declare the thing `[i8; 4]`. That is usually what it was, and sometimes
// it is a 32-bit word that a field offset points into.

#[test]
fn a_computed_base_is_a_part_select() {
    let v = compile("fun ex (x: i32, b: i5, o: out i8)\n  o = @slice(x, b, 8)\n");
    assert!(v.contains("x[b +: 8]"), "{}", v);
}

#[test]
fn a_constant_base_is_an_ordinary_range() {
    // `+:` with a literal base is correct and is also the construct this
    // backend exists to keep away from GowinSynthesis.
    let v = compile("fun ex (x: i32, o: out i8)\n  o = @slice(x, 8, 8)\n");
    assert!(v.contains("x[15:8]"), "{}", v);
    assert!(!v.contains("+:"), "{}", v);
}

#[test]
fn a_slice_wider_than_its_operand_is_refused() {
    let text = compile_err("fun ex (x: i8, b: i3, o: out i16)\n  o = @slice(x, b, 16)\n");
    assert!(text.contains("does not fit"), "{}", text);
}

#[test]
fn a_constant_slice_past_the_end_is_refused() {
    let text = compile_err("fun ex (x: i32, o: out i8)\n  o = @slice(x, 28, 8)\n");
    assert!(text.contains("runs past the end"), "{}", text);
}

#[test]
fn a_slice_width_must_be_constant() {
    let text = compile_err("fun ex (x: i32, b: i5, o: out i8)\n  o = @slice(x, 0, b)\n");
    assert!(text.contains("must be a constant"), "{}", text);
}

#[test]
fn a_signed_base_is_refused() {
    let text = compile_err("fun ex (x: i32, b: s5, o: out i8)\n  o = @slice(x, b, 8)\n");
    assert!(text.contains("must be unsigned"), "{}", text);
}

// ---- what an unusable length says ----------------------------------------
//
// `const_eval` answers with WHY it could not fold, and these are the four
// answers. They used to collapse into one -- "array length must be a constant
// known at compile time" -- which is true of only the first: `8 / 0` is a
// constant expression, and telling its author to make it constant sends them
// looking for a runtime variable that is not there.

#[test]
fn a_length_that_divides_by_zero_says_so() {
    let text = compile_err(concat!(
        "process p (rd: buffer out i32)\n",
        "  var vals: [i32; 8 / 0] = @zeroed()\n",
        "  let _s = @try_send(rd, vals[3'd0])\n",
    ));
    assert!(text.contains("array length divides by zero"), "{}", text);
    // The `%` beside it folds through the same guard.
    let text = compile_err(concat!(
        "process p (rd: buffer out i32)\n",
        "  var vals: [i32; 8 % 0] = @zeroed()\n",
        "  let _s = @try_send(rd, vals[3'd0])\n",
    ));
    assert!(text.contains("array length divides by zero"), "{}", text);
}

#[test]
fn a_length_that_overflows_says_so() {
    let text = compile_err(concat!(
        "process p (rd: buffer out i32)\n",
        "  var vals: [i32; 2 ** 200] = @zeroed()\n",
        "  let _s = @try_send(rd, vals[3'd0])\n",
    ));
    assert!(text.contains("overflows"), "{}", text);
}

#[test]
fn the_two_ends_of_an_unusable_length_read_differently() {
    // Nothing to hold, and more than an index could reach. One message for
    // both would be wrong at whichever end the reader was standing at.
    let empty = compile_err(concat!(
        "process p (rd: buffer out i32)\n",
        "  var vals: [i32; 0] = @zeroed()\n",
        "  let _s = @try_send(rd, vals[3'd0])\n",
    ));
    assert!(empty.contains("holds nothing"), "{}", empty);

    let huge = compile_err(concat!(
        "process p (rd: buffer out i32)\n",
        "  var vals: [i32; 8000000000] = @zeroed()\n",
        "  let _s = @try_send(rd, vals[3'd0])\n",
    ));
    assert!(huge.contains("past what an index can address"), "{}", huge);
}
