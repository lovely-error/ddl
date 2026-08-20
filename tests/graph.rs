// `graph`: the only place hierarchy comes from.
//
// Everywhere else the compiler flattens -- a `fun` call is inlined, one
// declaration is one module. These tests are about the other direction: that a
// graph emits instances, that it wires them the way the pipes say, and that
// the four ways to miswire a pipe are all compile errors rather than netlists
// that read as intermittent hangs.

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

/// A one-stage sequence, i16 in and i16 out.
const DBL: &str = concat!(
    "sequence dbl (src: buffer in i16, dst: buffer out i16)\n",
    "  let a = @rcv(src)\n",
    "  |||\n",
    "  let d: i16 = a + a\n",
    "  @send(dst, d)\n",
);

/// i16 in, i32 out, so a graph has to keep the two apart.
const WIDEN: &str = concat!(
    "sequence widen (src: buffer in i16, dst: buffer out i32)\n",
    "  let a = @rcv(src)\n",
    "  |||\n",
    "  let w: i32 = @zext(a, 32)\n",
    "  @send(dst, w)\n",
);

#[test]
fn a_graph_instantiates_and_wires() {
    let v = compile(&format!(
        "{}{}{}",
        DBL,
        WIDEN,
        concat!(
            "graph quad (src: buffer in i16, dst: buffer out i32)\n",
            "  let once: buffer i16\n",
            "  let twice: buffer i16\n",
            "  dbl(src, once)\n",
            "  dbl(once, twice)\n",
            "  widen(twice, dst)\n",
        )
    ));

    // Three instances, the repeated one distinguished by a suffix rather than
    // by a counter nobody can read in a waveform.
    assert!(v.contains("dbl u_dbl ("), "{}", v);
    assert!(v.contains("dbl u_dbl_1 ("), "{}", v);
    assert!(v.contains("widen u_widen ("), "{}", v);

    // Every internal pipe is three wires, and the payload keeps its width.
    assert!(v.contains("wire once_valid;"), "{}", v);
    assert!(v.contains("wire once_ready;"), "{}", v);
    assert!(v.contains("wire [15:0] once_data;"), "{}", v);

    // Connected by name, and the second instance reads what the first drove.
    assert!(v.contains(".dst_data  (once_data)"), "{}", v);
    assert!(v.contains(".src_data  (once_data)"), "{}", v);
}

#[test]
fn a_graph_carries_clock_and_reset_to_every_instance() {
    let v = compile(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  dbl(src, dst)\n",
        )
    ));
    assert!(v.contains("input         clk,"), "{}", v);
    assert!(v.contains("input         rst_n,"), "{}", v);
    assert!(v.contains(".clk       (clk)"), "{}", v);
    assert!(v.contains(".rst_n     (rst_n)"), "{}", v);
}

#[test]
fn a_graph_port_connects_straight_through() {
    // A graph input is already produced from outside, so it needs a consumer
    // inside and no producer. The handshake legs line up without inversion:
    // the graph's `src_ready` is an output and the instance drives it.
    let v = compile(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  dbl(src, dst)\n",
        )
    ));
    assert!(v.contains("input         src_valid,"), "{}", v);
    assert!(v.contains("output        src_ready,"), "{}", v);
    assert!(v.contains("output        dst_valid,"), "{}", v);
    assert!(v.contains(".src_valid (src_valid)"), "{}", v);
    assert!(v.contains(".dst_valid (dst_valid)"), "{}", v);

    // A graph has no logic of its own: wires and instances, nothing else.
    let graph = v.split("module g (").nth(1).expect("the graph module");
    assert!(!graph.contains("assign"), "{}", graph);
    assert!(!graph.contains("always"), "{}", graph);
}

#[test]
fn a_cycle_is_allowed() {
    // desc.md:88 asks for this explicitly. A net is a net whichever order the
    // instances appear in, so a feedback path needs no special handling -- and
    // a retry queue or a credit return is not expressible without one.
    let v = compile(&format!(
        "{}{}",
        DBL,
        concat!(
            "sequence merge (a: buffer in i16, out_: buffer out i16)\n",
            "  let x = @rcv(a)\n",
            "  |||\n",
            "  @send(out_, x)\n",
            "graph loopy (src: buffer in i16, dst: buffer out i16)\n",
            "  let back: buffer i16\n",
            "  let fwd: buffer i16\n",
            "  merge(src, back)\n",
            "  dbl(back, fwd)\n",
            "  merge(fwd, dst)\n",
        )
    ));
    assert!(v.contains("module loopy ("), "{}", v);
    assert_eq!(v.matches("u_merge").count(), 2, "{}", v);
}

#[test]
fn a_graph_can_instantiate_a_graph() {
    let v = compile(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph inner (a: buffer in i16, b: buffer out i16)\n",
            "  dbl(a, b)\n",
            "graph outer (src: buffer in i16, dst: buffer out i16)\n",
            "  let mid: buffer i16\n",
            "  inner(src, mid)\n",
            "  inner(mid, dst)\n",
        )
    ));
    assert!(v.contains("inner u_inner ("), "{}", v);
    assert!(v.contains("inner u_inner_1 ("), "{}", v);
}

#[test]
fn a_process_is_instantiable_too() {
    let v = compile(concat!(
        "process adder (src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    let b = @rcv(src)\n",
        "    @send(dst, a + b)\n",
        "graph g (src: buffer in i32, dst: buffer out i32)\n",
        "  adder(src, dst)\n",
    ));
    assert!(v.contains("adder u_adder ("), "{}", v);
}

// ---- the ways to miswire a pipe ------------------------------------------

#[test]
fn two_producers_on_one_pipe_is_an_error() {
    // Verilog would take both drivers and resolve the net to `x`.
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  let mid: buffer i16\n",
            "  dbl(src, mid)\n",
            "  dbl(src, mid)\n",
            "  dbl(mid, dst)\n",
        )
    ));
    assert!(text.contains("`mid` has 2 producers"), "{}", text);
    assert!(text.contains("resolve to `x`"), "{}", text);
}

#[test]
fn two_consumers_says_what_it_would_take() {
    // desc.md allows several consumers by duplicating the sink. Saying that is
    // the point: the reader should know it is unbuilt, not disallowed.
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  let mid: buffer i16\n",
            "  dbl(src, mid)\n",
            "  dbl(mid, dst)\n",
            "  dbl(mid, dst)\n",
        )
    ));
    assert!(text.contains("consumers"), "{}", text);
    assert!(text.contains("duplicating the sink"), "{}", text);
}

#[test]
fn a_pipe_nothing_sends_to_is_an_error() {
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  let mid: buffer i16\n",
            "  dbl(src, dst)\n",
            "  dbl(mid, dst)\n",
        )
    ));
    assert!(text.contains("nothing sends to `mid`"), "{}", text);
}

#[test]
fn a_pipe_nothing_receives_from_is_an_error() {
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  let mid: buffer i16\n",
            "  dbl(src, mid)\n",
            "  dbl(src, dst)\n",
        )
    ));
    assert!(text.contains("nothing receives from `mid`"), "{}", text);
}

#[test]
fn a_width_mismatch_at_an_instance_port_is_caught_here() {
    // Silent truncation in every tool downstream, so it has to be caught here.
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i32, dst: buffer out i16)\n",
            "  dbl(src, dst)\n",
        )
    ));
    assert!(text.contains("carries `i32`"), "{}", text);
    assert!(text.contains("`dbl.src` carries `i16`"), "{}", text);
}

#[test]
fn a_stream_pipe_is_refused() {
    // The two kinds used to have to agree at both ends of a pipe, because
    // only one of them had a `ready`. There is one kind now, so the check is
    // gone and the word is what is refused.
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  let mid: stream i16\n",
            "  dbl(src, mid)\n",
            "  dbl(mid, dst)\n",
        )
    ));
    assert!(text.contains("is not a pipe kind; DDL has `buffer`"), "{}", text);
    // It blames the line, not the whole `graph` declaration.
    assert!(text.contains("let mid: stream i16"), "{}", text);
}

#[test]
fn the_arity_error_lists_the_pipes_it_wanted() {
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  dbl(src)\n",
        )
    ));
    assert!(text.contains("has 2 pipe parameters, but 1 was given"), "{}", text);
    assert!(text.contains("src: buffer in i16, dst: buffer out i16"), "{}", text);
}

#[test]
fn an_unknown_module_lists_the_real_ones() {
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  nosuch(src, dst)\n",
        )
    ));
    assert!(text.contains("`nosuch` is not a process, sequence or graph"), "{}", text);
    assert!(text.contains("dbl"), "{}", text);
}

#[test]
fn an_undeclared_pipe_name_says_how_to_declare_it() {
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)\n",
            "  dbl(src, nowhere)\n",
        )
    ));
    assert!(text.contains("`nowhere` is not a pipe of this graph"), "{}", text);
    assert!(text.contains("let <name>: buffer <T>"), "{}", text);
}

#[test]
fn a_pipe_with_no_kind_says_which_word_is_missing() {
    // `let mid: i16` is the shape of the mistake the `let` spelling invites:
    // a pipe declaration that names no kind reads as a wire.
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in i16, dst: buffer out i16)
",
            "  let mid: i16
",
            "  dbl(src, mid)
",
            "  dbl(mid, dst)
",
        )
    ));
    assert!(text.contains("does not say what kind of pipe it is"), "{}", text);
    assert!(text.contains("let <name>: buffer <T>"), "{}", text);
    // And it blames the line, not the whole `graph` declaration.
    assert!(text.contains("let mid: i16"), "{}", text);
}

#[test]
fn a_graph_cannot_instantiate_itself() {
    let text = compile_err(concat!(
        "graph g (src: buffer in i16, dst: buffer out i16)\n",
        "  g(src, dst)\n",
    ));
    assert!(text.contains("cannot instantiate itself"), "{}", text);
}

#[test]
fn a_plain_parameter_on_a_graph_is_refused() {
    // A graph connects pipes. A raw wire across the boundary is the thing the
    // language exists to stop.
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (n: i8, src: buffer in i16, dst: buffer out i16)\n",
            "  dbl(src, dst)\n",
        )
    ));
    assert!(text.contains("`n` is not a pipe"), "{}", text);
}

#[test]
fn an_empty_graph_is_refused() {
    let text = compile_err(concat!(
        "graph g (src: buffer in i16, dst: buffer out i16)\n",
        "  let mid: buffer i16\n",
    ));
    assert!(text.contains("instantiates nothing"), "{}", text);
}
