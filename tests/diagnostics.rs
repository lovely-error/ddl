// Where diagnostics point.
//
// Lowering has no idea where it is unless something tells it: the AST carries
// a span on every identifier and on nothing else, so there is no statement
// node to hang one on. Every diagnostic raised from the middle of a lowering
// pass therefore had no location and rendered at line 1, whichever line it was
// actually about -- which for a module with thirty statements means reading
// all thirty.
//
// These tests assert the LINE rather than the column. The anchor is the
// identifier that reads as the statement's subject, so the column is usually
// good and occasionally approximate; the line is the part a reader needs and
// the part that must not regress.

use ddl::diag::SourceMap;
use ddl::driver::compile_to_verilog;
use ddl::verilog::EmitOptions;

/// The `line:col` of the first diagnostic, and its message.
fn first_error(src: &str) -> (u32, String) {
    let map = SourceMap::new("t.ddl", src);
    let diags = match compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(v) => panic!("expected failure, got:\n{}", v),
        Err(d) => d,
    };
    let text = map.render_all(&diags);
    let line = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("--> t.ddl:"))
        .and_then(|rest| rest.split(':').next().map(str::to_string))
        .unwrap_or_else(|| panic!("no location in:\n{}", text))
        .parse()
        .expect("a line number");
    (line, text)
}

/// Two lines of preamble, so an error reported at line 1 is unmistakably the
/// old "nowhere" behaviour rather than a coincidence.
fn assert_at(line: u32, src: &str) {
    let (got, text) = first_error(src);
    assert_eq!(got, line, "expected line {}, got {} in:\n{}", line, got, text);
}

#[test]
fn a_width_mismatch_points_at_the_operator_that_has_it() {
    assert_at(
        4,
        concat!(
            "fun f (a: u32, b: u5, o: out u32)\n",
            "  let p = a\n",
            "  let q = a\n",
            "  o = a + b\n",
        ),
    );
}

#[test]
fn a_latch_points_at_the_branch_that_causes_it() {
    assert_at(
        4,
        concat!(
            "fun f (c: u1, a: u8, o: out u8)\n",
            "  let p = a\n",
            "  let q = a\n",
            "  if c then\n",
            "    o = a\n",
        ),
    );
}

#[test]
fn a_non_boolean_condition_points_at_the_condition() {
    assert_at(
        4,
        concat!(
            "fun f (a: u8, o: out u8)\n",
            "  let p = a\n",
            "  let q = a\n",
            "  if a then\n",
            "    o = a\n",
            "  else\n",
            "    o = a\n",
        ),
    );
}

#[test]
fn a_match_on_the_wrong_type_points_at_the_scrutinee() {
    assert_at(
        4,
        concat!(
            "fun f (a: u8, o: out u8)\n",
            "  var v: u8 = @zeroed()\n",
            "  let q = a\n",
            "  match a\n",
            "    _ =>\n",
            "      v = a\n",
            "  o = v\n",
        ),
    );
}

#[test]
fn assigning_a_let_points_at_the_assignment() {
    assert_at(
        4,
        concat!(
            "fun f (a: u8, o: out u8)\n",
            "  let z: u8 = a\n",
            "  let q = a\n",
            "  z = a\n",
            "  o = z\n",
        ),
    );
}

#[test]
fn a_for_bound_points_at_the_loop() {
    assert_at(
        4,
        concat!(
            "fun f (v: u8, o: out u8)\n",
            "  var acc: u8 = @zeroed()\n",
            "  let q = v\n",
            "  for i in 0..v\n",
            "    acc = acc + 8'd1\n",
            "  o = acc\n",
        ),
    );
}

#[test]
fn a_scheduler_refusal_points_at_the_statement_it_refuses() {
    // The scheduler runs before lowering, so it has no anchor stack -- it
    // reads the same anchor off the statement in front of it.
    //
    // A blocking receive buried in an expression is what it refuses now. An
    // `if`, a `match` and a `loop` can all hold one as a statement; what none
    // of them can do is say that the rest of an expression waits for it.
    assert_at(
        4,
        concat!(
            "process p (src: buffer in u32, dst: buffer out u32)\n",
            "  loop\n",
            "    let a = @rcv(src)\n",
            "    let b = @rcv(src) + a\n",
            "    @send(dst, b)\n",
        ),
    );
}

#[test]
fn a_pipe_that_is_not_a_pipe_points_at_the_operation() {
    assert_at(
        4,
        concat!(
            "process p (src: buffer in u32, dst: buffer out u32)\n",
            "  loop\n",
            "    let a = @rcv(src)\n",
            "    let b = @rcv(nosuch)\n",
            "    @send(dst, a + b)\n",
        ),
    );
}

#[test]
fn a_send_of_the_wrong_type_points_at_the_send() {
    assert_at(
        5,
        concat!(
            "process p (src: buffer in u8, dst: buffer out u32)\n",
            "  loop\n",
            "    let a = @rcv(src)\n",
            "    let b = @rcv(src)\n",
            "    @send(dst, a + b)\n",
        ),
    );
}

#[test]
fn a_blocking_receive_nested_in_a_send_says_so_and_points_at_it() {
    // `@send(dst, @rcv(src))`. The rule is the one every other position
    // already states: a blocking transfer is a STATEMENT, because a state is
    // built from a statement and nothing inside an expression can say that the
    // rest of that expression waits. An `if` condition and a `match` scrutinee
    // refuse one for exactly this reason; a `@send`'s operand list did not,
    // because the scheduler peels the payload off before that check runs and
    // hands it to expression lowering, which had no case for a transfer.
    //
    // It came out as "this operator takes two operands" -- the arithmetic
    // fall-through, counting the `@rcv`'s single argument -- reported at line
    // 1 column 1, the `process` header, because the payload is lowered outside
    // any statement and so had no anchor. Neither half named anything real.
    let (line, text) = first_error(concat!(
        "process chk (src: buffer in u16, dst: buffer out u16)\n",
        "  loop\n",
        "    @send(dst, @rcv(src))\n",
    ));
    assert_eq!(line, 3, "expected the `@send` line, got {} in:\n{}", line, text);
    assert!(
        text.contains("a blocking `@rcv` has to be a statement of its own"),
        "{}",
        text
    );
    // The column matters here where the line alone does not: both operations
    // are on line 3, and the one to fix is the inner one.
    assert!(text.contains("t.ddl:3:21"), "expected the `@rcv`, got:\n{}", text);
    // The split form the note describes is the rewrite, and it compiles.
    let split = SourceMap::new(
        "t.ddl",
        concat!(
            "process chk (src: buffer in u16, dst: buffer out u16)\n",
            "  loop\n",
            "    let v = @rcv(src)\n",
            "    @send(dst, v)\n",
        ),
    );
    assert!(compile_to_verilog(&split, &EmitOptions::default()).is_ok());
}

#[test]
fn an_error_inside_a_send_payload_points_at_the_payload() {
    // The same missing anchor, without the nesting. The scheduler takes a
    // `@send` apart, so its payload is lowered with no statement around it,
    // and every diagnostic the payload raised landed on the declaration
    // header -- for this one, "width mismatch: `u8` and `u32`" at line 1.
    assert_at(
        5,
        concat!(
            "process p (a: buffer in u8, b: buffer in u32, dst: buffer out u32)\n",
            "  loop\n",
            "    let x = @rcv(a)\n",
            "    let y = @rcv(b)\n",
            "    @send(dst, x + y)\n",
        ),
    );
}

#[test]
fn a_sequence_stage_error_points_into_the_stage() {
    assert_at(
        5,
        concat!(
            "sequence s (src: buffer in u16, dst: buffer out u16)\n",
            "  let a = @rcv(src)\n",
            "  |||\n",
            "  let b: u16 = a + a\n",
            "  let c = @rcv(src)\n",
            "  @send(dst, b)\n",
        ),
    );
}

#[test]
fn nothing_reports_at_line_one_by_accident() {
    // A guard against the whole class coming back: if a diagnostic loses its
    // anchor it lands on line 1, and every case above has three lines of
    // preamble precisely so that shows up as a failure rather than a pass.
    for src in [
        concat!(
            "fun f (a: u32, b: u5, o: out u32)\n",
            "  let p = a\n",
            "  o = a + b\n",
        ),
        concat!(
            "fun f (a: u8, o: out u8)\n",
            "  let p = a\n",
            "  o = nope\n",
        ),
    ] {
        let (line, text) = first_error(src);
        assert!(line > 1, "reported at line 1:\n{}", text);
    }
}

// ---- parse errors --------------------------------------------------------
//
// A declaration whose header parsed and whose body had one bad line used to
// fail whole, so the top-level loop reported ``unexpected `fun` `` against the
// declaration keyword -- the one line that was fine.

#[test]
fn a_bad_statement_blames_its_own_line_not_the_declaration() {
    let (line, text) = first_error(concat!(
        "fun f (a: u8, o: out u8)
",
        "  let x = a
",
        "  ??? broken
",
        "  o = x
",
    ));
    assert_eq!(line, 3, "{}", text);
    assert!(text.contains("does not belong in a `fun` body"), "{}", text);
}

#[test]
fn each_kind_of_body_says_what_it_holds() {
    let cases = [
        (
            concat!("struct s_t
", "  a: u8
", "  ??? junk
"),
            "a struct body names one field per line",
        ),
        (
            concat!(
                "graph g (src: buffer in u16, dst: buffer out u16)
",
                "  ??? junk
",
            ),
            "a graph body declares a pipe",
        ),
        (
            concat!("enum e_t: u2
", "  A
", "  ??? junk
"),
            "an enum body names one variant per line",
        ),
    ];
    for (src, hint) in cases {
        let (line, text) = first_error(src);
        assert!(line > 1, "reported at line 1:
{}", text);
        assert!(text.contains(hint), "expected {:?} in:
{}", hint, text);
    }
}

#[test]
fn leftovers_on_a_body_line_are_caught_there() {
    // `B ~~ C` parses `B` and stops. The remainder used to fall out of the
    // declaration and be reported as top-level garbage, with a note about
    // top-level declarations, three lines away from the problem.
    let (line, text) = first_error(concat!(
        "enum e_t: u2
",
        "  A
",
        "  B ~~ C
",
        "  D
",
    ));
    assert_eq!(line, 3, "{}", text);
    assert!(text.contains("an `enum` body"), "{}", text);
}

#[test]
fn genuine_top_level_garbage_still_reads_as_that() {
    let (line, text) = first_error("gibberish Bad
");
    assert_eq!(line, 1, "{}", text);
    assert!(text.contains("unexpected `gibberish`"), "{}", text);
    assert!(text.contains("expected a top-level"), "{}", text);
}

#[test]
fn a_trailing_comment_on_a_body_line_is_not_leftovers() {
    let map = SourceMap::new("t.ddl", concat!(
        "enum e_t: u2
",
        "  A       -- the quiet one
",
        "  B
",
        "fun f (x: e_t, o: out u1)
",
        "  o = x == B
",
    ));
    compile_to_verilog(&map, &EmitOptions::default()).expect("should compile");
}

#[test]
fn a_range_that_will_not_fold_is_reported_rather_than_dropped() {
    // `const_eval(..).ok()?` returned `None` with nothing in the sink, so the
    // whole declaration vanished and the build still succeeded: `ddl build` on
    // this file wrote a Verilog file containing only `good`, exit code 0.
    let (line, msg) = first_error(concat!(
        "fun broken (a: u8, hi: u3, o: out u8)\n",
        "  o = a[hi..0]\n",
        "fun good (a: u1, o: out u1)\n",
        "  o = a\n",
    ));
    assert_eq!(line, 2);
    assert!(msg.contains("must be a constant known at compile time"), "{}", msg);
}

#[test]
fn a_declaration_that_fails_to_lower_cannot_leave_the_build_succeeding() {
    // The point is the exit status, not the wording: a file whose function
    // does not compile must not produce Verilog. Asserted separately from the
    // message because the silent-drop bug was invisible to every test that
    // only looked at diagnostics.
    let map = SourceMap::new(
        "t.ddl",
        concat!(
            "fun broken (a: u8, hi: u3, o: out u8)\n",
            "  o = a[hi..0]\n",
            "fun good (a: u1, o: out u1)\n",
            "  o = a\n",
        ),
    );
    let out = compile_to_verilog(&map, &EmitOptions::default());
    assert!(out.is_err(), "compiled to:\n{}", out.unwrap_or_default());
}
