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

/// A one-stage sequence, u16 in and u16 out.
const DBL: &str = concat!(
    "sequence dbl (src: buffer in u16, dst: buffer out u16)\n",
    "  let a = @rcv(src)\n",
    "  |||\n",
    "  let d: u16 = a + a\n",
    "  @send(dst, d)\n",
);

/// u16 in, u32 out, so a graph has to keep the two apart.
const WIDEN: &str = concat!(
    "sequence widen (src: buffer in u16, dst: buffer out u32)\n",
    "  let a = @rcv(src)\n",
    "  |||\n",
    "  let w: u32 = @zext(a, 32)\n",
    "  @send(dst, w)\n",
);

#[test]
fn a_graph_instantiates_and_wires() {
    let v = compile(&format!(
        "{}{}{}",
        DBL,
        WIDEN,
        concat!(
            "graph quad (src: buffer in u16, dst: buffer out u32)\n",
            "  let once: buffer u16\n",
            "  let twice: buffer u16\n",
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
    assert!(v.contains("wire [1:0] once_wsalt;"), "{}", v);
    assert!(v.contains("wire [1:0] once_rsalt;"), "{}", v);
    // Two entries on the wire, so the net is twice the payload.
    assert!(v.contains("wire [31:0] once_data;"), "{}", v);

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
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
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
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
            "  dbl(src, dst)\n",
        )
    ));
    assert!(v.contains("input  [1:0]  src_wsalt,"), "{}", v);
    assert!(v.contains("output [1:0]  src_rsalt,"), "{}", v);
    assert!(v.contains("output [1:0]  dst_wsalt,"), "{}", v);
    assert!(v.contains(".src_wsalt (src_wsalt)"), "{}", v);
    assert!(v.contains(".dst_wsalt (dst_wsalt)"), "{}", v);

    // A graph has no logic of its own: wires and instances, nothing else.
    let graph = v.split("module g (").nth(1).expect("the graph module");
    assert!(!graph.contains("assign"), "{}", graph);
    assert!(!graph.contains("always"), "{}", graph);
}

#[test]
fn a_cycle_is_allowed() {
    // desc.md:111 asks for this explicitly. A net is a net whichever order the
    // instances appear in, so a feedback path needs no special handling -- and
    // a retry queue or a credit return is not expressible without one.
    let v = compile(&format!(
        "{}{}",
        DBL,
        concat!(
            "sequence merge (a: buffer in u16, out_: buffer out u16)\n",
            "  let x = @rcv(a)\n",
            "  |||\n",
            "  @send(out_, x)\n",
            "graph loopy (src: buffer in u16, dst: buffer out u16)\n",
            "  let back: buffer u16\n",
            "  let fwd: buffer u16\n",
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
            "graph inner (a: buffer in u16, b: buffer out u16)\n",
            "  dbl(a, b)\n",
            "graph outer (src: buffer in u16, dst: buffer out u16)\n",
            "  let mid: buffer u16\n",
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
        "process adder (src: buffer in u32, dst: buffer out u32)\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    let b = @rcv(src)\n",
        "    @send(dst, a + b)\n",
        "graph g (src: buffer in u32, dst: buffer out u32)\n",
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
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
            "  let mid: buffer u16\n",
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
    // desc.md allows several consumers by duplicating the sink, and `@split`
    // is that duplication. Naming one pipe twice is still not how to ask for
    // it -- that is two drivers on one salt -- so the note names the thing
    // that does it rather than saying it cannot be done.
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
            "  let mid: buffer u16\n",
            "  dbl(src, mid)\n",
            "  dbl(mid, dst)\n",
            "  dbl(mid, dst)\n",
        )
    ));
    assert!(text.contains("consumers"), "{}", text);
    assert!(text.contains("@split(p, a, b)"), "{}", text);
    assert!(text.contains("its own pair of entries"), "{}", text);
}

#[test]
fn a_pipe_nothing_sends_to_is_an_error() {
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
            "  let mid: buffer u16\n",
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
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
            "  let mid: buffer u16\n",
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
            "graph g (src: buffer in u32, dst: buffer out u16)\n",
            "  dbl(src, dst)\n",
        )
    ));
    assert!(text.contains("carries `u32`"), "{}", text);
    assert!(text.contains("`dbl.src` carries `u16`"), "{}", text);
}

#[test]
fn a_pipe_naming_a_kind_that_does_not_exist_is_refused() {
    // `buffer` is the only pipe kind, so a `let` naming another one is not a
    // pipe declaration at all -- it does not reach the graph lowering, and the
    // body-item loop blames the line rather than the `graph`.
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
            "  let mid: fifo u16\n",
            "  dbl(src, mid)\n",
            "  dbl(mid, dst)\n",
        )
    ));
    assert!(text.contains("does not belong in a `graph` body"), "{}", text);
    assert!(text.contains("`let <name>: buffer <T>`"), "{}", text);
    // It blames the line, not the whole `graph` declaration.
    assert!(text.contains("let mid: fifo u16"), "{}", text);
}

#[test]
fn the_arity_error_lists_the_pipes_it_wanted() {
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
            "  dbl(src)\n",
        )
    ));
    assert!(text.contains("has 2 parameters, but 1 was given"), "{}", text);
    assert!(text.contains("src: buffer in u16, dst: buffer out u16"), "{}", text);
}

#[test]
fn an_unknown_module_lists_the_real_ones() {
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
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
            "graph g (src: buffer in u16, dst: buffer out u16)\n",
            "  dbl(src, nowhere)\n",
        )
    ));
    assert!(text.contains("`nowhere` is not a pipe of this graph"), "{}", text);
    assert!(text.contains("let <name>: buffer <T>"), "{}", text);
}

#[test]
fn a_pipe_with_no_kind_says_which_word_is_missing() {
    // `let mid: u16` is the shape of the mistake the `let` spelling invites:
    // a pipe declaration that names no kind reads as a wire.
    let text = compile_err(&format!(
        "{}{}",
        DBL,
        concat!(
            "graph g (src: buffer in u16, dst: buffer out u16)
",
            "  let mid: u16
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
    assert!(text.contains("let mid: u16"), "{}", text);
}

#[test]
fn a_graph_cannot_instantiate_itself() {
    let text = compile_err(concat!(
        "graph g (src: buffer in u16, dst: buffer out u16)\n",
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
            "graph g (n: u8, src: buffer in u16, dst: buffer out u16)\n",
            "  dbl(src, dst)\n",
        )
    ));
    assert!(text.contains("`n` is not a pipe"), "{}", text);
}

#[test]
fn an_empty_graph_is_refused() {
    let text = compile_err(concat!(
        "graph g (src: buffer in u16, dst: buffer out u16)\n",
        "  let mid: buffer u16\n",
    ));
    assert!(text.contains("instantiates nothing"), "{}", text);
}

// ---- extern ---------------------------------------------------------------
//
// A graph could only instantiate modules DDL had compiled, which left DDL a
// guest inside a SystemVerilog top level: the SoC instantiated the generated
// modules and wired the salt triples by hand. Hand-wiring a `valid`/`ready`
// pair is something an RTL engineer does correctly from memory; hand-wiring a
// pair of gray-coded salts is not, and the failure mode is a design that looks
// connected and deadlocks.
//
// `extern` is the other direction: DDL owns the hierarchy, and the pieces that
// stay SystemVerilog because they are the BOARD -- a PLL, a PSRAM controller,
// two clock domains -- are named rather than reproduced.

#[test]
fn a_graph_can_instantiate_a_module_ddl_did_not_compile() {
    let v = compile(concat!(
        "extern psram (req: buffer in u32, rsp: buffer out u32)\n",
        "sequence dbl (a: buffer in u32, b: buffer out u32)\n",
        "  let x = @rcv(a)\n",
        "  |||\n",
        "  @send(b, x + x)\n",
        "graph top (src: buffer in u32, dst: buffer out u32)\n",
        "  let mid: buffer u32\n",
        "  dbl(src, mid)\n",
        "  psram(mid, dst)\n",
    ));
    // A plain FIFO, and no salt anywhere on it. This is the whole feature:
    // the hand-written module is an ordinary handshake, not a protocol its
    // author has to reconstruct from a document.
    assert!(v.contains("psram u_psram ("), "{}", v);
    for port in [
        ".req_can_receive",
        ".req_receive_en",
        ".req_data_write_in",
        ".rsp_has_data",
        ".rsp_drop_item",
        ".rsp_data_read_out",
    ] {
        assert!(v.contains(port), "{} missing from
{}", port, v);
    }
    assert!(!v.contains(".req_wsalt"), "{}", v);
    assert!(!v.contains(".rsp_data  ("), "{}", v);
    // The translation is a module the compiler wrote, one per shape.
    assert!(v.contains("module ddl_salt_to_wport_32 ("), "{}", v);
    assert!(v.contains("module ddl_rport_to_salt_32 ("), "{}", v);
}

#[test]
fn an_extern_emits_no_module_of_its_own() {
    let v = compile(concat!(
        "extern sink (a: buffer in u32)\n",
        "sequence src_ (a: buffer in u32, b: buffer out u32)\n",
        "  let x = @rcv(a)\n",
        "  |||\n",
        "  @send(b, x)\n",
        "graph top (i: buffer in u32)\n",
        "  let mid: buffer u32\n",
        "  src_(i, mid)\n",
        "  sink(mid)\n",
    ));
    // The body is somebody else's. Emitting a stub would be a module that
    // silently does nothing, which is worse than a link error.
    assert!(!v.contains("module sink"), "{}", v);
    assert!(v.contains("sink u_sink ("), "{}", v);
}

#[test]
fn the_connections_to_an_extern_are_still_checked() {
    let text = compile_err(concat!(
        "extern psram (req: buffer in u32, rsp: buffer out u32)\n",
        "graph top (src: buffer in u32, dst: buffer out u32)\n",
        "  psram(src)\n",
    ));
    assert!(text.contains("has 2 parameters, but 1 was given"), "{}", text);
}

#[test]
fn two_producers_on_a_pipe_are_refused_even_when_one_is_extern() {
    let text = compile_err(concat!(
        "extern psram (req: buffer in u32, rsp: buffer out u32)\n",
        "extern other (rsp: buffer out u32)\n",
        "graph top (src: buffer in u32, dst: buffer out u32)\n",
        "  let mid: buffer u32\n",
        "  psram(src, mid)\n",
        "  other(mid)\n",
    ));
    assert!(text.contains("producer"), "{}", text);
}

#[test]
fn an_extern_declares_pipes_and_wires_and_nothing_else() {
    // A graph connects pipes and wires. A parameter of any other kind would be
    // a port the graph has no way to reach, left unconnected in the
    // instantiation -- a floating wire, which is the shape of bug that shows
    // up as a hang.
    let text = compile_err(concat!(
        "extern pll (lock: inout u1, rsp: buffer out u32)
",
        "graph top (dst: buffer out u32)
",
        "  pll(dst)
",
    ));
    assert!(text.contains("is not a pipe or a wire"), "{}", text);
}

#[test]
fn an_extern_takes_a_wire_and_the_graph_carries_it_to_its_boundary() {
    // The whole point of a `wire`: a pin, or a sideband on a piece of vendor
    // IP, reaching the outside of the design without a handshake wrapped
    // around it. It is one port of the declared width, and the graph passes it
    // straight through.
    let v = compile(concat!(
        "extern pll (locked: wire out u1, rsp: buffer out u32)
",
        "graph top (dst: buffer out u32, lock_led: wire out u1)
",
        "  pll(lock_led, dst)
",
    ));
    // The padding is the emitter's column alignment, so match on the
    // parts rather than the spacing between them.
    assert!(v.contains("output") && v.contains("lock_led
);"), "{}", v);
    assert!(v.contains(".locked") && v.contains("(lock_led)"), "{}", v);
    // No enable beside it, and no salt: a wire is the bare signal.
    assert!(!v.contains("lock_led_en"), "{}", v);
    assert!(!v.contains("lock_led_wsalt"), "{}", v);
}

#[test]
fn a_wire_driven_by_two_instances_is_refused() {
    let text = compile_err(concat!(
        "extern a (o: wire out u1)
",
        "extern b (o: wire out u1)
",
        "extern sink_ (i: buffer in u32)
",
        "graph top (src: buffer in u32, led: wire out u1)
",
        "  a(led)
",
        "  b(led)
",
        "  sink_(src)
",
    ));
    assert!(text.contains("driven by 2 instances"), "{}", text);
}

#[test]
fn a_wire_on_a_process_names_where_a_wire_is_allowed() {
    let text = compile_err("process p (led: wire out u1, src: buffer in u32)
  loop
    let took = @drop(src)
");
    assert!(text.contains("is a `wire`, which this declaration cannot take"), "{}", text);
    assert!(text.contains("only an `extern` or a `graph`"), "{}", text);
}

// ---- @merge and @split ----------------------------------------------------
//
// Two requesters sharing one pipe, and one producer reaching two consumers.
// Both could be written as a `process`, and both would then cost a cycle per
// hop, because a process is a state machine and a state is a cycle. For
// something whose whole job is to pass an item along that is the wrong price,
// so these are modules the compiler writes: a datapath, no states.

const DOUBLER: &str = concat!(
    "sequence dbl (a: buffer in u32, b: buffer out u32)\n",
    "  let x = @rcv(a)\n",
    "  |||\n",
    "  @send(b, x + x)\n",
);

#[test]
fn a_merge_arbitrates_in_rotation() {
    let v = compile(&format!(
        "{}{}",
        DOUBLER,
        concat!(
            "graph top (p: buffer in u32, q: buffer in u32, o: buffer out u32)\n",
            "  let m: buffer u32\n",
            "  @merge(p, q, m)\n",
            "  dbl(m, o)\n",
        )
    ));
    assert!(v.contains("module ddl_merge_2x32 ("), "{}", v);
    // Rotating, not fixed. A fixed priority is smaller and starves input 1
    // whenever input 0 is busy, which for two requesters sharing a bus is the
    // bug an arbiter exists to not have.
    assert!(v.contains("reg turn;"), "{}", v);
    assert!(
        v.contains("wire grant0 = i0_offered & ((turn == 1'b0) | ((turn == 1'b1) & (!i1_offered)));"),
        "{}",
        v
    );
    // Whoever went this time goes last next time.
    assert!(v.contains("turn <="), "{}", v);
}

#[test]
fn only_one_input_of_a_merge_is_granted() {
    let v = compile(&format!(
        "{}{}",
        DOUBLER,
        concat!(
            "graph top (p: buffer in u32, q: buffer in u32, o: buffer out u32)\n",
            "  let m: buffer u32\n",
            "  @merge(p, q, m)\n",
            "  dbl(m, o)\n",
        )
    ));
    // Each input advances only on its OWN transfer. Advancing both would drop
    // one of the two items, which is the failure the whole handshake exists to
    // make impossible.
    assert!(v.contains("wire take0 = grant0 & push;"), "{}", v);
    assert!(v.contains("wire take1 = grant1 & push;"), "{}", v);
    assert!(v.contains("i0_rsalt_q <= (take0 ?"), "{}", v);
    assert!(v.contains("i1_rsalt_q <= (take1 ?"), "{}", v);
}

#[test]
fn a_split_takes_only_when_every_sink_has_room() {
    let v = compile(&format!(
        "{}{}",
        DOUBLER,
        concat!(
            "graph top (p: buffer in u32, o1: buffer out u32, o2: buffer out u32)\n",
            "  let d: buffer u32\n",
            "  dbl(p, d)\n",
            "  @split(d, o1, o2)\n",
        )
    ));
    assert!(v.contains("module ddl_split_2x32 ("), "{}", v);
    // A slot per sink, and the input waits for the slowest. ANDing the sinks'
    // READYS instead would put each consumer's logic in every other one's
    // timing path -- the coupling the salt protocol exists to remove.
    assert!(v.contains("wire all_room = (!o0_full) & (!o1_full);"), "{}", v);
    assert!(v.contains("wire take = (!i_empty) & all_room;"), "{}", v);
    assert!(v.contains("reg [31:0] o0_e0;"), "{}", v);
    assert!(v.contains("reg [31:0] o1_e0;"), "{}", v);
}

#[test]
fn a_combinator_never_depends_on_the_ready_coming_back() {
    let v = compile(&format!(
        "{}{}",
        DOUBLER,
        concat!(
            "graph top (p: buffer in u32, o1: buffer out u32, o2: buffer out u32)\n",
            "  let d: buffer u32\n",
            "  dbl(p, d)\n",
            "  @split(d, o1, o2)\n",
        )
    ));
    // Every published salt is a register read, which is what makes rule 3
    // hold by construction rather than by review.
    assert!(v.contains("assign o0_wsalt = o0_wsalt_q;"), "{}", v);
    assert!(v.contains("assign o1_wsalt = o1_wsalt_q;"), "{}", v);
    assert!(v.contains("assign i_rsalt = i_rsalt_q;"), "{}", v);
}

#[test]
fn a_three_way_merge_rotates_over_three_starts() {
    let v = compile(&format!(
        "{}{}",
        DOUBLER,
        concat!(
            "graph top (p: buffer in u32, q: buffer in u32, r: buffer in u32, o: buffer out u32)\n",
            "  let m: buffer u32\n",
            "  @merge(p, q, r, m)\n",
            "  dbl(m, o)\n",
        )
    ));
    assert!(v.contains("module ddl_merge_3x32 ("), "{}", v);
    assert!(v.contains("grant2"), "{}", v);
    // Under start 2 the order is 2, 0, 1: input 0 waits only on input 2.
    assert!(v.contains("((turn == 2'd2) & (!i2_offered))"), "{}", v);
}

#[test]
fn one_module_serves_every_use_of_the_same_shape() {
    let v = compile(&format!(
        "{}{}",
        DOUBLER,
        concat!(
            "graph top (a: buffer in u32, b: buffer in u32, c: buffer in u32, d: buffer in u32, o: buffer out u32)\n",
            "  let m1: buffer u32\n",
            "  let m2: buffer u32\n",
            "  let m3: buffer u32\n",
            "  @merge(a, b, m1)\n",
            "  @merge(c, d, m2)\n",
            "  @merge(m1, m2, m3)\n",
            "  dbl(m3, o)\n",
        )
    ));
    assert_eq!(v.matches("module ddl_merge_2x32 (").count(), 1, "{}", v);
    assert_eq!(v.matches("ddl_merge_2x32 u_").count(), 3, "{}", v);
}

#[test]
fn a_combinator_needs_two_sides() {
    let text = compile_err(concat!(
        "graph top (p: buffer in u32, o: buffer out u32)\n",
        "  @merge(p)\n",
    ));
    assert!(text.contains("needs at least two pipes"), "{}", text);
}

#[test]
fn an_extern_parameter_whose_type_is_unknown_is_reported() {
    // `signature_of` skips a parameter it cannot resolve, on the grounds that
    // lowering the declaration will report it. An `extern` has no body and is
    // never lowered, so nothing ever did: the port silently left the interface
    // and the instantiation below connected `ok` and nothing else, leaving a
    // hardware input floating because of a typo.
    let e = compile_err(concat!(
        "extern ext (ok: wire in u1, bad: wire in MissingType)\n",
        "graph g (x: wire in u1)\n",
        "  ext(x)\n",
    ));
    assert!(e.contains("`MissingType` is not a type"), "{}", e);
}

#[test]
fn an_extern_is_checked_even_when_nothing_instantiates_it() {
    // The interface is the whole of the declaration, so there is no later pass
    // that would catch this one.
    let e = compile_err(concat!(
        "extern ext (ok: wire in u1, bad: wire in MissingType)\n",
        "fun h (a: u1, o: out u1)\n",
        "  o = a\n",
    ));
    assert!(e.contains("`MissingType` is not a type"), "{}", e);
}

#[test]
fn two_graphs_cannot_instantiate_each_other() {
    // Self-instantiation was caught while lowering one declaration, which is
    // the only cycle visible from inside one declaration. This pair compiled
    // into two Verilog modules instantiating one another -- not a finite piece
    // of hardware, and the compiler did not loop, so nothing complained.
    let text = compile_err(concat!(
        "graph a (x: buffer in u8, y: buffer out u8)\n",
        "  b(x, y)\n",
        "graph b (x: buffer in u8, y: buffer out u8)\n",
        "  a(x, y)\n",
    ));
    assert!(text.contains("instantiates itself through"), "{}", text);
    assert!(text.contains("a -> b -> a"), "the whole cycle is named\n{}", text);
}

#[test]
fn a_longer_instantiation_cycle_is_reported_whole() {
    let text = compile_err(concat!(
        "graph a (x: buffer in u8, y: buffer out u8)\n",
        "  b(x, y)\n",
        "graph b (x: buffer in u8, y: buffer out u8)\n",
        "  c(x, y)\n",
        "graph c (x: buffer in u8, y: buffer out u8)\n",
        "  a(x, y)\n",
    ));
    assert!(text.contains("a -> b -> c -> a"), "{}", text);
}

#[test]
fn a_deep_acyclic_hierarchy_is_not_a_cycle() {
    // The check must not fire on nesting, only on a hierarchy that reaches
    // itself. Three graphs deep over a process.
    let map = SourceMap::new(
        "t.ddl",
        concat!(
            "process p (x: buffer in u8, y: buffer out u8)\n",
            "  loop\n",
            "    let v = @rcv(x)\n",
            "    @send(y, v)\n",
            "graph c (x: buffer in u8, y: buffer out u8)\n",
            "  p(x, y)\n",
            "graph b (x: buffer in u8, y: buffer out u8)\n",
            "  c(x, y)\n",
            "graph a (x: buffer in u8, y: buffer out u8)\n",
            "  b(x, y)\n",
        ),
    );
    let opts = EmitOptions {
        export: ddl::ir_export::ExportFlags {
            export: Vec::new(),
            bare: vec!["a".to_string()],
            crossings: Vec::new(),
        },
        ..EmitOptions::default()
    };
    compile_to_verilog(&map, &opts).expect("a nested hierarchy is not a cycle");
}

#[test]
fn two_merges_of_one_width_and_different_types_share_one_module() {
    // `ir_comb::module_name` names a combinator by the width it routes, so the
    // list of combinators to build has to be keyed on the width too. Keying it
    // on the whole `Ty` made `u16` and a struct of two `u8`s two uses, both of
    // which built `ddl_merge_2x16`, and the design was rejected for defining
    // it twice -- the same defect as the boundary adapters', in the file the
    // adapters' comment points at.
    let v = compile(concat!(
        "struct pair_t\n",
        "  lo: u8\n",
        "  hi: u8\n",
        "graph top (a: buffer in u16, b: buffer in u16, c: buffer in pair_t, d: buffer in pair_t, o1: buffer out u16, o2: buffer out pair_t)\n",
        "  @merge(a, b, o1)\n",
        "  @merge(c, d, o2)\n",
    ));
    assert_eq!(
        v.matches("\nmodule ddl_merge_2x16 (").count(),
        1,
        "one module for the shape, not one per type:\n{}",
        v
    );
    assert_eq!(
        v.matches("  ddl_merge_2x16 u_").count(),
        2,
        "both merges instantiate it:\n{}",
        v
    );
}

// ---- pipe loops that can never carry their first item ---------------------
//
// `check_graph_cycles` refuses a hierarchy that contains itself and says, in
// as many words, that feedback through a channel is a different thing and
// stays legal. It is -- except around a loop of sequences, where every one of
// them is waiting for an item only its predecessor can make, and every pipe
// starts empty. That is provably dead, and these pin both the proof and its
// two deliberate limits.

/// The accumulator shape: one stream in, one feedback input, a result out and
/// the same result fed back. `{}` is how the feedback is received.
fn accumulator(feedback: &str, tail: &str) -> String {
    format!(
        concat!(
            "sequence acc (x: buffer in u16, fb: buffer in u16, o: buffer out u16, fbo: buffer out u16)\n",
            "  let a = @rcv(x)\n",
            "{}",
            "{}",
            "  |||\n",
            "  @send(o, s)\n",
            "  @send(fbo, s)\n",
            "sequence hold (i: buffer in u16, o: buffer out u16)\n",
            "  let a = @rcv(i)\n",
            "  |||\n",
            "  @send(o, a)\n",
            "graph accum (src: buffer in u16, dst: buffer out u16)\n",
            "  let fwd: buffer u16\n",
            "  let back: buffer u16\n",
            "  acc(src, back, dst, fwd)\n",
            "  hold(fwd, back)\n",
        ),
        feedback, tail,
    )
}

#[test]
fn a_loop_of_blocking_sequences_is_rejected_and_names_the_whole_loop() {
    let text = compile_err(&accumulator(
        "  let b = @rcv(fb)\n",
        "  let s: u16 = a + b\n",
    ));
    assert!(text.contains("`acc` waits on itself through"), "{}", text);
    assert!(text.contains("acc -> hold -> acc"), "{}", text);
    // The note has to name the way out, or the diagnostic is only a refusal.
    assert!(text.contains("@try_rcv"), "{}", text);
}

#[test]
fn the_same_loop_is_live_once_the_feedback_is_optional() {
    // The point of the check: a `@try_rcv` fires without waiting, so this loop
    // has no first item to be missing. It is an accumulator that reads last
    // cycle's result on the cycles there is one.
    let v = compile(&accumulator(
        "  let (b, ok) = @try_rcv(fb)\n",
        "  var s: u16 = a\n  if ok then\n    s = a + b\n",
    ));
    assert!(v.contains("module acc ("), "{}", v);
    assert!(v.contains("module hold ("), "{}", v);
    // The feedback pipe is optional, so it is not part of what paces `acc`.
    assert!(v.contains("wire take = x_present & shift0;"), "{}", v);
}

/// The same loop with `acc`'s whole body up to the caller, which decides how
/// its head fires. `x` comes from outside the graph; `fb` is the feedback.
fn feedback_loop(acc_body: &str) -> String {
    format!(
        concat!(
            "sequence acc (x: buffer in u16, fb: buffer in u16, o: buffer out u16, fbo: buffer out u16)\n",
            "{}",
            "sequence hold (i: buffer in u16, o: buffer out u16)\n",
            "  let a = @rcv(i)\n",
            "  |||\n",
            "  @send(o, a)\n",
            "graph accum (src: buffer in u16, dst: buffer out u16)\n",
            "  let fwd: buffer u16\n",
            "  let back: buffer u16\n",
            "  acc(src, back, dst, fwd)\n",
            "  hold(fwd, back)\n",
        ),
        acc_body,
    )
}

#[test]
fn a_head_that_only_samples_the_feedback_is_a_deadlock_too() {
    // No blocking receive, so the head fires on any input its FIRST stage
    // reads -- and the only one is the feedback. `x` is read one stage later,
    // where it samples for an item that can never have entered.
    let text = compile_err(&feedback_loop(concat!(
        "  let (b, ok) = @try_rcv(fb)\n",
        "  |||\n",
        "  let (a, aok) = @try_rcv(x)\n",
        "  @send(o, b)\n",
        "  @send(fbo, b)\n",
    )));
    assert!(text.contains("`acc` waits on itself through acc -> hold -> acc"), "{}", text);
    assert!(text.contains("any pipe its first stage reads"), "{}", text);
}

#[test]
fn a_head_that_peeks_and_drops_the_feedback_is_a_deadlock_too() {
    let text = compile_err(&feedback_loop(concat!(
        "  let (b, here) = @peek(fb)\n",
        "  @drop(fb)\n",
        "  |||\n",
        "  @drop(x)\n",
        "  @send(o, b)\n",
        "  @send(fbo, b)\n",
    )));
    assert!(text.contains("`acc` waits on itself through"), "{}", text);
}

#[test]
fn a_head_that_blocks_on_the_feedback_is_dead_whatever_else_it_samples() {
    // An AND head fires without its optional inputs, so an outside `@try_rcv`
    // beside a blocking feedback receive changes nothing.
    let text = compile_err(&feedback_loop(concat!(
        "  let b = @rcv(fb)\n",
        "  let (a, ok) = @try_rcv(x)\n",
        "  |||\n",
        "  @send(o, b)\n",
        "  @send(fbo, b)\n",
    )));
    assert!(text.contains("`acc` waits on itself through"), "{}", text);
}

#[test]
fn a_head_that_samples_an_outside_input_beside_the_feedback_is_live() {
    let v = compile(&feedback_loop(concat!(
        "  let (a, aok) = @try_rcv(x)\n",
        "  let (b, bok) = @try_rcv(fb)\n",
        "  |||\n",
        "  @send(o, a)\n",
        "  @send(fbo, b)\n",
    )));
    assert!(v.contains("module accum"), "{}", v);
}

#[test]
fn a_loop_is_dead_however_its_producers_send() {
    // Conditional sends and offers produce less than a plain `@send`, never
    // more, so they cannot bring a dead loop to life.
    let text = compile_err(&feedback_loop(concat!(
        "  let a = @rcv(x)\n",
        "  let b = @rcv(fb)\n",
        "  |||\n",
        "  if a != 16'd0 then\n",
        "    @send(o, a)\n",
        "  let ok = @try_send(fbo, b)\n",
    )));
    assert!(text.contains("`acc` waits on itself through"), "{}", text);
}

#[test]
fn a_dead_chain_hanging_off_a_dead_loop_is_one_report() {
    // `tail` can never fire either, but only because of the loop, and the loop
    // is what there is to fix.
    let src = concat!(
        "sequence acc (x: buffer in u16, fb: buffer in u16, o: buffer out u16, fbo: buffer out u16)\n",
        "  let b = @rcv(fb)\n",
        "  let (a, ok) = @try_rcv(x)\n",
        "  |||\n",
        "  @send(o, b)\n",
        "  @send(fbo, b)\n",
        "sequence hold (i: buffer in u16, o: buffer out u16)\n",
        "  let a = @rcv(i)\n",
        "  |||\n",
        "  @send(o, a)\n",
        "graph accum (src: buffer in u16, dst: buffer out u16)\n",
        "  let fwd: buffer u16\n",
        "  let back: buffer u16\n",
        "  let mid: buffer u16\n",
        "  acc(src, back, mid, fwd)\n",
        "  hold(fwd, back)\n",
        "  hold(mid, dst)\n",
    );
    let text = compile_err(src);
    assert_eq!(text.matches("waits on itself").count(), 1, "{}", text);
}

#[test]
fn a_loop_closed_through_a_process_is_left_alone() {
    // Soundness, and the check's first limit. A process may send before it
    // ever receives -- this one seeds the loop with a zero -- so the wait is
    // not certain and refusing it would be a false positive.
    let src = concat!(
        "sequence acc (x: buffer in u16, fb: buffer in u16, o: buffer out u16, fbo: buffer out u16)\n",
        "  let a = @rcv(x)\n",
        "  let b = @rcv(fb)\n",
        "  |||\n",
        "  let s: u16 = a + b\n",
        "  @send(o, s)\n",
        "  @send(fbo, s)\n",
        "process seeder (i: buffer in u16, o: buffer out u16)\n",
        "  @send(o, 16'd0)\n",
        "  loop\n",
        "    let a = @rcv(i)\n",
        "    @send(o, a)\n",
        "graph accum (src: buffer in u16, dst: buffer out u16)\n",
        "  let fwd: buffer u16\n",
        "  let back: buffer u16\n",
        "  acc(src, back, dst, fwd)\n",
        "  seeder(fwd, back)\n",
    );
    let v = compile(src);
    assert!(v.contains("module accum"), "{}", v);
}

#[test]
fn a_sequence_wired_back_to_itself_reads_as_what_it_is() {
    // One node, and "waits on itself through selfy -> selfy" would be a worse
    // way to say it.
    let text = compile_err(concat!(
        "sequence selfy (x: buffer in u16, fb: buffer in u16, o: buffer out u16, fbo: buffer out u16)\n",
        "  let a = @rcv(x)\n",
        "  let b = @rcv(fb)\n",
        "  |||\n",
        "  let s: u16 = a + b\n",
        "  @send(o, s)\n",
        "  @send(fbo, s)\n",
        "graph g (src: buffer in u16, dst: buffer out u16)\n",
        "  let back: buffer u16\n",
        "  selfy(src, back, dst, back)\n",
    ));
    assert!(
        text.contains("`selfy` waits on an item it is the only source of"),
        "{}",
        text
    );
}

#[test]
fn a_feed_forward_graph_of_sequences_is_not_a_loop() {
    // Anti-vacuous: the walk must not report reconvergence. `dbl` and `widen`
    // both read `src`'s stream through a split and meet nowhere.
    let src = format!(
        "{}{}{}",
        DBL,
        WIDEN,
        concat!(
            "sequence pair (a: buffer in u16, b: buffer in u32, o: buffer out u32)\n",
            "  let x = @rcv(a)\n",
            "  let y = @rcv(b)\n",
            "  |||\n",
            "  @send(o, y + @zext(x, 32))\n",
            "graph fanin (src: buffer in u16, dst: buffer out u32)\n",
            "  let one: buffer u16\n",
            "  let two: buffer u16\n",
            "  let doubled: buffer u16\n",
            "  let widened: buffer u32\n",
            "  @split(src, one, two)\n",
            "  dbl(one, doubled)\n",
            "  widen(two, widened)\n",
            "  pair(doubled, widened, dst)\n",
        ),
    );
    let v = compile(&src);
    assert!(v.contains("module fanin"), "{}", v);
}
