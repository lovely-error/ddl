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
