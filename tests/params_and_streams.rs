// `inout` parameters and `stream` pipes.
//
// Both are cases where the language already had the vocabulary and the
// compiler refused the word. `inout` is desc.md:62 -- by reference and
// readable, which is what lets a helper update its argument instead of
// returning a new copy of it. `stream` is desc.md:82 -- a pipe whose producer
// is never told to wait, because the oldest item is overwritten instead.

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

// ---- inout ---------------------------------------------------------------

#[test]
fn an_inout_parameter_is_two_ports_at_a_module_boundary() {
    // Not a Verilog `inout`, which is a tri-state and not what this means. The
    // value that came in and the value going back are separate wires.
    let v = compile(concat!(
        "fun bump (step: i8, acc: inout i8)\n",
        "  acc = acc + step\n",
    ));
    assert!(v.contains("input  [7:0] acc,"), "{}", v);
    assert!(v.contains("output [7:0] acc_out"), "{}", v);
    assert!(v.contains("assign acc_out = (acc + step);"), "{}", v);
}

#[test]
fn an_inout_argument_is_updated_in_place_at_the_call_site() {
    // The call is inlined, so "by reference" means the caller's binding is
    // replaced with whatever the callee left in it.
    let v = compile(concat!(
        "fun bump (step: i8, acc: inout i8)\n",
        "  acc = acc + step\n",
        "fun total (a: i8, b: i8, o: out i8)\n",
        "  var acc: i8 = @zeroed()\n",
        "  bump(a, acc)\n",
        "  bump(b, acc)\n",
        "  o = acc\n",
    ));
    assert!(v.contains("assign o = ((8'd0 + a) + b);"), "{}", v);
    // Inlined, so no instantiation.
    assert!(!v.contains("bump u_"), "{}", v);
}

#[test]
fn an_inout_can_be_read_before_it_is_written() {
    // That is the difference from `out`: the value arrived with the call.
    let v = compile(concat!(
        "fun double_it (x: inout i8)\n",
        "  x = x + x\n",
        "fun use_it (a: i8, o: out i8)\n",
        "  var v: i8 = a\n",
        "  double_it(v)\n",
        "  o = v\n",
    ));
    assert!(v.contains("assign o = (a + a);"), "{}", v);
}

#[test]
fn an_inout_mixes_with_a_bound_output() {
    let v = compile(concat!(
        "fun step (acc: inout i8, carry: out i1)\n",
        "  acc = acc + 8'd1\n",
        "  carry = acc == 8'd0\n",
        "fun run (a: i8, o: out i8, c: out i1)\n",
        "  var acc: i8 = a\n",
        "  let (got) = step(acc)\n",
        "  o = acc\n",
        "  c = got\n",
    ));
    // The incremented value reaches both `o` (through the `inout`) and the
    // carry test (through the `out`), from one shared expression.
    assert!(v.contains("= a + 8'd1;"), "{}", v);
    assert!(v.contains("assign c = got;"), "{}", v);
}

#[test]
fn an_inout_argument_has_to_be_a_variable() {
    // An expression has nowhere for the answer to go.
    let text = compile_err(concat!(
        "fun bump (step: i8, acc: inout i8)\n",
        "  acc = acc + step\n",
        "fun total (a: i8, o: out i8)\n",
        "  bump(a, a + 8'd1)\n",
        "  o = a\n",
    ));
    assert!(text.contains("is `inout`, so it has to be a variable"), "{}", text);
}

#[test]
fn an_inout_argument_cannot_be_a_let() {
    let text = compile_err(concat!(
        "fun bump (step: i8, acc: inout i8)\n",
        "  acc = acc + step\n",
        "fun total (a: i8, o: out i8)\n",
        "  let acc: i8 = a\n",
        "  bump(a, acc)\n",
        "  o = acc\n",
    ));
    assert!(text.contains("cannot be assigned"), "{}", text);
    assert!(text.contains("declare it `var`"), "{}", text);
}

#[test]
fn a_call_with_outputs_still_has_to_bind_them() {
    let text = compile_err(concat!(
        "fun f (a: i8, o: out i8)\n",
        "  o = a\n",
        "fun g (a: i8, o: out i8)\n",
        "  f(a)\n",
        "  o = a\n",
    ));
    assert!(text.contains("results need binding"), "{}", text);
}

#[test]
fn a_call_that_produces_nothing_says_so() {
    let text = compile_err(concat!(
        "fun nothing (a: i8)\n",
        "  let x: i8 = a\n",
        "fun g (a: i8, o: out i8)\n",
        "  nothing(a)\n",
        "  o = a\n",
    ));
    assert!(text.contains("produces nothing"), "{}", text);
}

// ---- stream --------------------------------------------------------------

#[test]
fn a_stream_output_on_a_sequence_has_no_ready() {
    let v = compile(concat!(
        "sequence widen (src: buffer in i16, dst: stream out i32)\n",
        "  let a = @rcv(src)\n",
        "  |||\n",
        "  let w: i32 = @zext(a, 32)\n",
        "  @send(dst, w)\n",
    ));
    assert!(v.contains("output        dst_valid,"), "{}", v);
    assert!(v.contains("output [31:0] dst_data"), "{}", v);
    assert!(!v.contains("dst_ready"), "{}", v);
}

#[test]
fn a_stream_sink_means_the_pipeline_never_stalls() {
    // Nothing downstream can refuse an item, so the shift enable is a constant
    // and the input is always accepted.
    let v = compile(concat!(
        "sequence widen (src: buffer in i16, dst: stream out i32)\n",
        "  let a = @rcv(src)\n",
        "  |||\n",
        "  let w: i32 = @zext(a, 32)\n",
        "  @send(dst, w)\n",
    ));
    assert!(v.contains("assign src_ready = 1'b1;"), "{}", v);
    assert!(!v.contains("(!v1) | 1'b1"), "{}", v);
}

#[test]
fn a_stream_input_is_sampled_rather_than_accepted() {
    let v = compile(concat!(
        "sequence tap (src: stream in i16, dst: buffer out i32)\n",
        "  let a = @rcv(src)\n",
        "  |||\n",
        "  let w: i32 = @zext(a, 32)\n",
        "  @send(dst, w)\n",
    ));
    assert!(v.contains("input         src_valid,"), "{}", v);
    assert!(!v.contains("src_ready"), "{}", v);
    // The validity bit rides through, so a cycle with nothing on the stream
    // produces an item marked invalid rather than a stall.
    assert!(v.contains("v0 <= (shift ? src_valid : v0);"), "{}", v);
}

#[test]
fn a_graph_wires_a_stream_with_two_legs_not_three() {
    let v = compile(concat!(
        "sequence widen (src: buffer in i16, dst: stream out i32)\n",
        "  let a = @rcv(src)\n",
        "  |||\n",
        "  let w: i32 = @zext(a, 32)\n",
        "  @send(dst, w)\n",
        "sequence sink_ (src: stream in i32, dst: buffer out i32)\n",
        "  let a = @rcv(src)\n",
        "  |||\n",
        "  @send(dst, a)\n",
        "graph g (src: buffer in i16, dst: buffer out i32)\n",
        "  let mid: stream i32\n",
        "  widen(src, mid)\n",
        "  sink_(mid, dst)\n",
    ));
    assert!(v.contains("wire mid_valid;"), "{}", v);
    assert!(v.contains("wire [31:0] mid_data;"), "{}", v);
    assert!(!v.contains("wire mid_ready;"), "{}", v);
}
