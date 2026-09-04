// `inout` parameters, and what a parameter is allowed to be.
//
// `inout` is desc.md:57 -- by reference and readable, which is what lets a
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

// ---- `port in` / `port out` -----------------------------------------------
//
// A pipe with no back-pressure: a data port and an enable, and nothing coming
// back. It is reached the way every other pipe is -- `@rcv` to wait for one,
// `@try_rcv` or `@peek` to look at what is there, `@send` and `@try_send` to
// put one out. A port is NOT a value: binding its name to the wire would make
// it the one channel in the language you read by naming it, and would lose the
// distinction between "the value" and "a value that means something this
// cycle" that `@try_rcv`'s pair exists to carry.
//
// It is deliberately not the bare `T` spelling either: that already means
// something else and quietly. A plain parameter is configuration, folded at
// compile time and gone, and two spellings one space apart -- one a constant,
// one a per-cycle channel -- is the trap this language exists to not set.
//
// README's rule for a sink that genuinely cannot refuse is that it ties
// `ready` high AND SAYS SO AT THE BOUNDARY, which puts the claim where a
// reader can check it. `port` is how it is said. Between two things DDL
// compiled, a `buffer` is still the answer.

#[test]
fn a_port_is_a_data_port_and_an_enable() {
    let v = compile(concat!(
        "process tap (lvl: port in u8, out_: port out u32)\n",
        "  let (x, got) = @try_rcv(lvl)\n",
        "  if got then\n",
        "    let _s = @try_send(out_, @zext(x, 32))\n",
    ));
    assert!(v.contains("input  [7:0]  lvl"), "{}", v);
    assert!(v.contains("input         lvl_en"), "{}", v);
    assert!(v.contains("output [31:0] out_"), "{}", v);
    assert!(v.contains("output        out__en"), "{}", v);
    // One entry on the wire, not two: there is no slot to skid into, so the
    // data port is the payload's own width rather than twice it.
    assert!(!v.contains("[15:0] lvl"), "{}", v);
}

#[test]
fn try_rcv_on_a_port_answers_with_its_enable() {
    let v = compile(concat!(
        "process tap (a: port in u16, b: port in u16, sum: port out u32)\n",
        "  let (x, gx) = @try_rcv(a)\n",
        "  let (y, gy) = @try_rcv(b)\n",
        "  if gx & gy then\n",
        "    let _s = @try_send(sum, @zext(x, 32) + @zext(y, 32))\n",
    ));
    // `got` is the wire and nothing else. On a pipe the question is "did I
    // TAKE one", which needs this side to have accepted; a port claims
    // nothing, so the honest answer is what the enable says.
    assert!(v.contains("(a_en & b_en)"), "{}", v);
    assert!(v.contains("assign sum = "), "{}", v);
}

#[test]
fn try_send_on_a_port_always_succeeds() {
    // There is no `ready` coming back, which is the whole of what a `port`
    // declares, so the answer never varies -- and the enable carries the path
    // instead, exactly as a pipe's offer does.
    let v = compile(concat!(
        "process tap (src: buffer in u32, sum: port out u32)\n",
        "  let (v, got) = @try_rcv(src)\n",
        "  if got then\n",
        "    let _s = @try_send(sum, v)\n",
    ));
    assert!(v.contains("assign sum_en = "), "{}", v);
    assert!(v.contains("src_take"), "{}", v);
}

#[test]
fn rcv_on_a_port_waits_for_the_enable() {
    let v = compile(concat!(
        "process relay (src: port in u32, dst: port out u32)\n",
        "  loop\n",
        "    let v = @rcv(src)\n",
        "    @send(dst, v + 32'd1)\n",
    ));
    // The receive waits on the only thing a port has to say.
    assert!(v.contains("wire fire_s0 = in_s0 & src_en;"), "{}", v);
    // The send does not wait at all: there is no `ready` to wait for, so its
    // state costs its cycle and no more.
    assert!(v.contains("assign dst_en = "), "{}", v);
    assert!(v.contains("v_r <= (fire_s0 ? src : v_r);"), "{}", v);
}

#[test]
fn a_port_is_not_a_value() {
    let text = compile_err(concat!(
        "process tap (lvl: port in u8, dst: buffer out u32)\n",
        "  let _s = @try_send(dst, @zext(lvl, 32))\n",
    ));
    assert!(text.contains("is a `port in`, which is a pipe rather than a value"), "{}", text);
    assert!(text.contains("@try_rcv(lvl)"), "{}", text);
}

#[test]
fn a_port_out_is_not_assigned() {
    let text = compile_err(concat!(
        "process tap (src: buffer in u32, sum: port out u32)\n",
        "  let (v, got) = @try_rcv(src)\n",
        "  sum = v\n",
    ));
    assert!(text.contains("is a `port out`, which is a pipe rather than a value"), "{}", text);
    assert!(text.contains("@send(sum, v)"), "{}", text);
}

#[test]
fn a_port_has_nothing_to_drop() {
    let text = compile_err(concat!(
        "process tap (lvl: port in u8, dst: buffer out u32)\n",
        "  let _d = @drop(lvl)\n",
        "  let _s = @try_send(dst, 32'd0)\n",
    ));
    assert!(text.contains("which has nothing to drop"), "{}", text);
}

#[test]
fn a_send_on_one_branch_puts_that_branch_in_the_enable() {
    let v = compile(concat!(
        "process tap (src: buffer in u32, sum: port out u32)\n",
        "  let (v, got) = @try_rcv(src)\n",
        "  if got then\n",
        "    let _s = @try_send(sum, v)\n",
    ));
    // The enable is the path, and only the enable: on a cycle it is low the
    // data is a don't-care, so muxing the data to zero as well would be a mux
    // for nobody.
    assert!(v.contains("wire sum_en_1 = src_take & "), "{}", v);
}

#[test]
fn a_port_out_nothing_sends_to_still_drives() {
    // An output left undriven would be a floating wire, which is the thing the
    // `if` rule exists to keep out of the language.
    let v = compile(concat!(
        "process tap (src: buffer in u32, dst: buffer out u32, spare: port out u32)\n",
        "  let (v, got) = @try_rcv(src)\n",
        "  let _s = @try_send(dst, v)\n",
    ));
    assert!(v.contains("assign spare = 32'd0;"), "{}", v);
    assert!(v.contains("assign spare_en = 1'b0;"), "{}", v);
}

#[test]
fn a_port_out_in_a_state_machine_is_driven_by_the_state_that_sent() {
    let v = compile(concat!(
        "process pump (cmd: buffer in u8, din: buffer in u32, stat: port out u32)\n",
        "  var n: u32 = @zeroed()\n",
        "  loop\n",
        "    let c = @rcv(cmd)\n",
        "    if c[0] then\n",
        "      let d = @rcv(din)\n",
        "      n += d\n",
        "      let _s = @try_send(stat, n)\n",
        "    else\n",
        "      let _s = @try_send(stat, 32'd0)\n",
    ));
    // Sent on both arms, so the enable is true in both of their states -- and
    // it is the state FIRING, not merely being current, because a state with a
    // barrier does its work in the cycle that barrier completes.
    assert!(v.contains("assign stat_en = "), "{}", v);
    assert!(v.contains("assign stat = "), "{}", v);
}

#[test]
fn a_plain_parameter_is_still_a_folded_constant() {
    // The whole reason `port` is spelled with a word.
    let v = compile(concat!(
        "process p (k: u8 = 8'd3, src: buffer in u32, dst: buffer out u32)\n",
        "  let (a, got) = @try_rcv(src)\n",
        "  let _s = @try_send(dst, a + @zext(k, 32))\n",
    ));
    assert!(!v.contains("input  [7:0]  k"), "k became a port:\n{}", v);
    assert!(v.contains("8'd3"), "{}", v);
}

#[test]
fn a_memory_cannot_be_a_port() {
    let text = compile_err(concat!(
        "process p (m: port in #[impl(lutram)] [u32; 4], dst: buffer out u32)\n",
        "  let _s = @try_send(dst, 32'd0)\n",
    ));
    assert!(text.contains("cannot be a parameter"), "{}", text);
}

#[test]
fn a_process_of_ports_alone_is_a_plain_verilog_module() {
    // What `port` is for: a module with no pipes at all, so DDL can write the
    // ordinary sequential blocks a design needs beside the dataflow ones.
    let v = compile(concat!(
        "process counter (step: port in u8, n: port out u32)\n",
        "  var c: u32 = @zeroed()\n",
        "  loop\n",
        "    let (s, got) = @try_rcv(step)\n",
        "    c += @zext(s, 32)\n",
        "    let _x = @try_send(n, c)\n",
    ));
    // No salt, no entries, no state register: a register, a clock and the
    // ports the source asked for.
    assert!(!v.contains("salt"), "{}", v);
    assert!(!v.contains("_e0"), "{}", v);
    assert!(v.contains("input  [7:0]  step"), "{}", v);
    assert!(v.contains("output [31:0] n"), "{}", v);
    assert!(v.contains("reg [31:0] c;"), "{}", v);
}

// ---- ports in a sequence --------------------------------------------------

#[test]
fn a_port_in_works_in_a_sequence_stage() {
    let v = compile(concat!(
        "sequence tagit (src: buffer in u32, lvl: port in u8, dst: buffer out u32)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let (l, got) = @try_rcv(lvl)\n",
        "  @send(dst, x + @zext(l, 32))\n",
    ));
    assert!(v.contains("input  [7:0]  lvl"), "{}", v);
    assert!(v.contains("input         lvl_en"), "{}", v);
}

#[test]
fn a_port_out_in_a_sequence_belongs_to_the_stage_that_sent() {
    // A process drives one from the state that sent to it, gated on that state
    // firing. There are no states here -- every stage is live at once, each
    // holding a different item -- so what stands in for "the state fired" is
    // "this stage has a valid item and the pipeline is moving".
    let v = compile(concat!(
        "sequence tagit (src: buffer in u32, sum: port out u32, dst: buffer out u32)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  let _s = @try_send(sum, x + x)\n",
        "  @send(dst, x)\n",
    ));
    assert!(v.contains("output [31:0] sum"), "{}", v);
    // Stage 1's validity is `v0`, and the same `shift` that pushes the item
    // out gates the port -- a stalled pipeline is holding, not producing.
    assert!(v.contains("wire sum_en_1 = v0 & shift;"), "{}", v);
    assert!(v.contains("wire [31:0] sum_1 = x_s1 + x_s1;"), "{}", v);
}

#[test]
fn a_port_out_in_the_head_stage_tracks_the_input() {
    let v = compile(concat!(
        "sequence early (src: buffer in u32, seen: port out u32, dst: buffer out u32)\n",
        "  let x = @rcv(src)\n",
        "  let _s = @try_send(seen, x)\n",
        "  |||\n",
        "  @send(dst, x + x)\n",
    ));
    // Stage 0 holds an item when the input is offering one, so its enable is
    // the same predicate that takes the item.
    assert!(v.contains("wire src_take = "), "{}", v);
    assert!(v.contains("assign seen_en = "), "{}", v);
}

#[test]
fn a_port_out_under_an_if_keeps_the_branch_in_its_enable() {
    let v = compile(concat!(
        "sequence flagged (src: buffer in u32, big: port out u32, dst: buffer out u32)\n",
        "  let x = @rcv(src)\n",
        "  |||\n",
        "  if x[31] then\n",
        "    let _s = @try_send(big, x)\n",
        "  @send(dst, x)\n",
    ));
    // Three things and all of them needed: the branch it was written on, the
    // stage holding an item, and the pipeline moving.
    assert!(v.contains("& v0) & shift"), "{}", v);
}

#[test]
fn a_port_out_sent_to_in_two_stages_is_refused() {
    // Every stage of a pipeline is live at once holding a different item, so
    // two stages driving one port is two answers for one wire -- and unlike a
    // process, where only one state is current, there is nothing to choose
    // between them.
    let text = compile_err(concat!(
        "sequence both (src: buffer in u32, tap: port out u32, dst: buffer out u32)\n",
        "  let x = @rcv(src)\n",
        "  let _a = @try_send(tap, x)\n",
        "  |||\n",
        "  let _b = @try_send(tap, x + x)\n",
        "  @send(dst, x)\n",
    ));
    assert!(text.contains("is sent to in stage 0 and stage 1"), "{}", text);
    assert!(text.contains("one port per stage"), "{}", text);
}
