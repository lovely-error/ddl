// `--emit=dot`.
//
// A `graph` is the one declaration whose meaning IS its shape, and neither
// existing view shows it: the source lists instances and leaves the reader to
// match pipe names, the Verilog lists them again with three wires per pipe in
// between. These tests pin the two things a picture has to get right --- who
// drives what, and where the design meets the outside.

use ddl::diag::SourceMap;
use ddl::driver::{Emit, compile};
use ddl::verilog::EmitOptions;

fn dot(src: &str) -> String {
    let map = SourceMap::new("t.ddl", src);
    match compile(&map, &EmitOptions::default(), Emit::Dot) {
        Ok(v) => v,
        Err(diags) => panic!("compile failed:\n{}", map.render_all(&diags)),
    }
}

const BLOCKS: &str = concat!(
    "sequence dbl (src: buffer in i16, dst: buffer out i16)\n",
    "  let a = @rcv(src)\n",
    "  |||\n",
    "  @send(dst, a)\n",
    "sequence widen (src: buffer in i16, dst: buffer out i32)\n",
    "  let a = @rcv(src)\n",
    "  |||\n",
    "  let w: i32 = @zext(a, 32)\n",
    "  @send(dst, w)\n",
);

#[test]
fn an_edge_runs_from_the_producer_to_the_consumer() {
    let g = dot(&format!(
        "{}{}",
        BLOCKS,
        concat!(
            "graph chain (src: buffer in i16, dst: buffer out i32)\n",
            "  let mid: buffer i16\n",
            "  dbl(src, mid)\n",
            "  widen(mid, dst)\n",
        )
    ));
    // Direction comes from which end the callee declared, not from the order
    // the instances were written in.
    assert!(g.contains(r#""chain__u_dbl" -> "chain__u_widen" [label="mid"]"#), "{}", g);
    assert!(!g.contains(r#""chain__u_widen" -> "chain__u_dbl""#), "{}", g);
}

#[test]
fn the_graphs_own_pipes_are_drawn_as_the_boundary() {
    let g = dot(&format!(
        "{}{}",
        BLOCKS,
        concat!(
            "graph chain (src: buffer in i16, dst: buffer out i32)\n",
            "  let mid: buffer i16\n",
            "  dbl(src, mid)\n",
            "  widen(mid, dst)\n",
        )
    ));
    // Different shapes, and pointing the right way: an input is produced from
    // outside and an output consumed outside.
    assert!(g.contains(r#""chain__src" [shape=invhouse"#), "{}", g);
    assert!(g.contains(r#""chain__dst" [shape=house"#), "{}", g);
    assert!(g.contains(r#""chain__src" -> "chain__u_dbl""#), "{}", g);
    assert!(g.contains(r#""chain__u_widen" -> "chain__dst""#), "{}", g);
}

#[test]
fn one_edge_per_pipe_not_one_per_wire() {
    // Three edges labelled valid, ready and data would say nothing a reader
    // does not know about how a pipe is built, and would triple the picture.
    let g = dot(&format!(
        "{}{}",
        BLOCKS,
        concat!(
            "graph chain (src: buffer in i16, dst: buffer out i32)\n",
            "  let mid: buffer i16\n",
            "  dbl(src, mid)\n",
            "  widen(mid, dst)\n",
        )
    ));
    assert_eq!(g.matches(" -> ").count(), 3, "{}", g);
    assert!(!g.contains("_valid\"]"), "{}", g);
    assert!(!g.contains("_ready\"]"), "{}", g);
}

#[test]
fn a_feedback_path_is_visible_as_one() {
    // The reason to draw the thing at all: a cycle is invisible in a list of
    // instantiations and obvious in a picture.
    let g = dot(&format!(
        "{}{}",
        BLOCKS,
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
    assert!(g.contains(r#"[label="back"]"#), "{}", g);
    assert!(g.contains(r#"[label="fwd"]"#), "{}", g);
    // Two instances of the same module, told apart.
    assert!(g.contains("u_merge\\nmerge"), "{}", g);
    assert!(g.contains("u_merge_1\\nmerge"), "{}", g);
}

#[test]
fn a_module_with_no_structure_is_not_drawn() {
    // One node per `fun` would bury the graphs, which are the only things with
    // a shape to show.
    let g = dot(&format!(
        "{}{}",
        BLOCKS,
        concat!(
            "fun helper (a: i8, o: out i8)\n",
            "  o = a\n",
            "graph chain (src: buffer in i16, dst: buffer out i32)\n",
            "  let mid: buffer i16\n",
            "  dbl(src, mid)\n",
            "  widen(mid, dst)\n",
        )
    ));
    assert!(!g.contains("helper"), "{}", g);
    assert!(g.contains("cluster_chain"), "{}", g);
}

#[test]
fn a_file_with_no_graph_says_so_rather_than_drawing_nothing() {
    // An empty picture looks exactly like a broken tool.
    let g = dot(concat!(
        "fun f (a: i8, o: out i8)\n",
        "  o = a\n",
    ));
    assert!(g.contains("no `graph` declarations"), "{}", g);
}

#[test]
fn several_graphs_each_get_their_own_cluster() {
    let g = dot(&format!(
        "{}{}",
        BLOCKS,
        concat!(
            "graph one (src: buffer in i16, dst: buffer out i16)\n",
            "  dbl(src, dst)\n",
            "graph two (src: buffer in i16, dst: buffer out i32)\n",
            "  widen(src, dst)\n",
        )
    ));
    assert!(g.contains("cluster_one"), "{}", g);
    assert!(g.contains("cluster_two"), "{}", g);
    // Node names are qualified by graph, so two graphs naming a pipe `src`
    // do not collapse into one node.
    assert!(g.contains(r#""one__src""#), "{}", g);
    assert!(g.contains(r#""two__src""#), "{}", g);
}

#[test]
fn the_output_is_syntactically_a_digraph() {
    let g = dot(&format!(
        "{}{}",
        BLOCKS,
        concat!(
            "graph chain (src: buffer in i16, dst: buffer out i32)\n",
            "  let mid: buffer i16\n",
            "  dbl(src, mid)\n",
            "  widen(mid, dst)\n",
        )
    ));
    assert!(g.starts_with("digraph ddl {"), "{}", g);
    assert!(g.trim_end().ends_with('}'), "{}", g);
    assert_eq!(g.matches('{').count(), g.matches('}').count(), "unbalanced braces:\n{}", g);
}
