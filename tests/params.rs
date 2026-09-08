// `inout` parameters, and what a parameter is allowed to be.
//
// `inout` is desc.md:86 -- by reference and readable, which is what lets a
// helper update its argument instead of returning a new copy of it.
//
// The rest of it is what a parameter may be qualified with. There is one pipe
// kind and it always has back-pressure, so the list is short and everything
// outside it is refused -- there is no lossy pipe to ask for and no spelling
// that quietly gets you one.

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
        "fun bump (step: u8, acc: inout u8)\n",
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
        "fun bump (step: u8, acc: inout u8)\n",
        "  acc = acc + step\n",
        "fun total (a: u8, b: u8, o: out u8)\n",
        "  var acc: u8 = @zeroed()\n",
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
        "fun double_it (x: inout u8)\n",
        "  x = x + x\n",
        "fun use_it (a: u8, o: out u8)\n",
        "  var v: u8 = a\n",
        "  double_it(v)\n",
        "  o = v\n",
    ));
    assert!(v.contains("assign o = (a + a);"), "{}", v);
}

#[test]
fn an_inout_mixes_with_a_bound_output() {
    let v = compile(concat!(
        "fun step (acc: inout u8, carry: out u1)\n",
        "  acc = acc + 8'd1\n",
        "  carry = acc == 8'd0\n",
        "fun run (a: u8, o: out u8, c: out u1)\n",
        "  var acc: u8 = a\n",
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
        "fun bump (step: u8, acc: inout u8)\n",
        "  acc = acc + step\n",
        "fun total (a: u8, o: out u8)\n",
        "  bump(a, a + 8'd1)\n",
        "  o = a\n",
    ));
    assert!(text.contains("is `inout`, so it has to be a variable"), "{}", text);
}

#[test]
fn an_inout_argument_cannot_be_a_let() {
    let text = compile_err(concat!(
        "fun bump (step: u8, acc: inout u8)\n",
        "  acc = acc + step\n",
        "fun total (a: u8, o: out u8)\n",
        "  let acc: u8 = a\n",
        "  bump(a, acc)\n",
        "  o = acc\n",
    ));
    assert!(text.contains("cannot be assigned"), "{}", text);
    assert!(text.contains("declare it `var`"), "{}", text);
}

#[test]
fn a_call_with_outputs_still_has_to_bind_them() {
    let text = compile_err(concat!(
        "fun f (a: u8, o: out u8)\n",
        "  o = a\n",
        "fun g (a: u8, o: out u8)\n",
        "  f(a)\n",
        "  o = a\n",
    ));
    assert!(text.contains("results need binding"), "{}", text);
}

#[test]
fn a_call_that_produces_nothing_says_so() {
    let text = compile_err(concat!(
        "fun nothing (a: u8)\n",
        "  let x: u8 = a\n",
        "fun g (a: u8, o: out u8)\n",
        "  nothing(a)\n",
        "  o = a\n",
    ));
    assert!(text.contains("produces nothing"), "{}", text);
}

// ---- what is not a pipe kind ---------------------------------------------

#[test]
fn an_unrecognised_pipe_qualifier_is_refused() {
    // `buffer in`, `buffer out`, `port in`, `port out`, `in`, `out`, `inout`
    // and nothing else. An unknown word ahead of the type is not silently a
    // plain parameter: the qualifier parser falls through to `in`, the word
    // itself is then read as the type, and what follows it has nowhere to go.
    let text = compile_err(concat!(
        "sequence widen (src: buffer in u16, dst: fifo out u32)
",
        "  let a = @rcv(src)
",
        "  |||
",
        "  let w: u32 = @zext(a, 32)
",
        "  @send(dst, w)
",
    ));
    assert!(text.contains("expected a top-level"), "{}", text);
}

#[test]
fn every_pipe_has_all_three_legs() {
    // The anti-vacuous half: the test above says what is refused, and this
    // says the thing that is accepted really does carry all three legs. There
    // is no shape of pipe that emits two.
    let v = compile(concat!(
        "sequence widen (src: buffer in u16, dst: buffer out u32)
",
        "  let a = @rcv(src)
",
        "  |||
",
        "  let w: u32 = @zext(a, 32)
",
        "  @send(dst, w)
",
        "sequence sink_ (src: buffer in u32, dst: buffer out u32)
",
        "  let a = @rcv(src)
",
        "  |||
",
        "  @send(dst, a)
",
        "graph g (src: buffer in u16, dst: buffer out u32)
",
        "  let mid: buffer u32
",
        "  widen(src, mid)
",
        "  sink_(mid, dst)
",
    ));
    assert!(v.contains("wire [1:0] mid_wsalt;"), "{}", v);
    assert!(v.contains("wire [1:0] mid_rsalt;"), "{}", v);
    assert!(v.contains("wire [63:0] mid_data;"), "{}", v);
}
