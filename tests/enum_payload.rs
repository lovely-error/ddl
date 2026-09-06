// Enums that carry a payload -- a tagged union.
//
// The layout is `{tag, payload}`, the tag in the HIGH bits, matching the way a
// struct puts its first field there. Every variant is the same width: a
// narrower payload is padded below, so the tag always sits in the same place
// and a `match` can read it without knowing yet what it is looking at.
//
// The thing that makes this more than a struct with a discriminant field is
// that the compiler will not let you read the payload without going through
// the tag. `match` is the only way in.

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

/// Two bits of tag, and the widest payload is 16 bits, so `req_e` is 18.
const REQ: &str = concat!(
    "struct addr_t\n",
    "  page: u8\n",
    "  off: u8\n",
    "enum req_e\n",
    "  Nop\n",
    "  Read(addr_t)\n",
    "  Write(u8)\n",
    "  Halt\n",
);

#[test]
fn the_width_is_the_tag_plus_the_widest_payload() {
    let v = compile(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun pass (r: req_e, o: out req_e)\n",
            "  o = r\n",
        )
    ));
    assert!(v.contains("input  [17:0] r,"), "{}", v);
    assert!(v.contains("output [17:0] o"), "{}", v);
}

#[test]
fn a_match_reads_the_tag_and_only_the_tag() {
    // Comparing the whole value would mean no arm ever fired, because the
    // payload differs from one item to the next.
    let v = compile(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun kind_of (r: req_e, o: out u2)\n",
            "  var k: u2 = 2'd0\n",
            "  match r\n",
            "    .Nop =>\n",
            "      k = 2'd0\n",
            "    .Read a =>\n",
            "      k = 2'd1\n",
            "    .Write d =>\n",
            "      k = 2'd2\n",
            "    .Halt =>\n",
            "      k = 2'd3\n",
            "  o = k\n",
        )
    ));
    assert!(v.contains("wire [1:0] req_e_tag = r[17:16];"), "{}", v);
    assert!(v.contains("case (req_e_tag)"), "{}", v);
}

#[test]
fn a_payload_binding_starts_below_the_tag_at_its_own_width() {
    let v = compile(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun payload (r: req_e, o: out u16)\n",
            "  var v: u16 = @zeroed()\n",
            "  match r\n",
            "    .Read a =>\n",
            "      v = @cast(a)\n",
            "    .Write d =>\n",
            "      v = @zext(d, 16)\n",
            "    _ =>\n",
            "      v = 16'd0\n",
            "  o = v\n",
        )
    ));
    // `addr_t` is 16 bits and `u8` is 8, and each arm takes exactly its own.
    assert!(v.contains("r[15:0]"), "{}", v);
    assert!(v.contains("= r[15:8];"), "{}", v);
}

#[test]
fn a_struct_payload_keeps_its_fields() {
    // The slice is a bag of bits until it is given the payload's type back;
    // without that, `a.page` would have nothing to select from.
    let v = compile(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun page_of (r: req_e, o: out u8)\n",
            "  var p: u8 = @zeroed()\n",
            "  match r\n",
            "    .Read a =>\n",
            "      p = a.page\n",
            "    _ =>\n",
            "      p = 8'd0\n",
            "  o = p\n",
        )
    ));
    // The payload gets a wire of its own at its own type, so the field select
    // reads off that rather than off the raw enum.
    assert!(v.contains("wire [15:0] a = r[15:0];"), "{}", v);
    assert!(v.contains("a[15:8]"), "{}", v);
}

#[test]
fn a_variant_is_built_with_its_payload() {
    let v = compile(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun mk (p: u8, off: u8, o: out req_e)\n",
            "  o = Read(addr_t(p, off))\n",
        )
    ));
    // Tag 1 over the 16-bit payload, and no padding because it is the widest.
    assert!(v.contains("{2'd1, {p, off}}"), "{}", v);
}

#[test]
fn a_narrow_payload_is_padded_below_itself() {
    // So the tag stays where the match expects it. Padded with zeros rather
    // than left undefined: `x` propagates through a comparison in simulation
    // and reads as a bug somewhere else entirely.
    let v = compile(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun mk (d: u8, o: out req_e)\n",
            "  o = Write(d)\n",
        )
    ));
    assert!(v.contains("{2'd2, d, 8'd0}"), "{}", v);
}

#[test]
fn a_payload_free_variant_is_its_tag_shifted_up() {
    let v = compile(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun idle (o: out req_e)\n",
            "  o = Nop\n",
            "fun stop (o: out req_e)\n",
            "  o = Halt\n",
        )
    ));
    assert!(v.contains("assign o = 18'd0;"), "{}", v);
    // Halt is discriminant 3, so 3 << 16.
    assert!(v.contains("18'h30000"), "{}", v);
}

#[test]
fn a_tagged_union_may_not_be_given_a_width() {
    // `enum req_e: u2` reads as "two bits wide" and the value is 18, because
    // the payload sits under the tag. Reading the number as the TAG width
    // instead is worse: the same syntax would mean the whole value for one
    // enum and part of it for the next, and a struct field budgeted from the
    // declaration would be short by the width of the payload.
    let text = compile_err(concat!(
        "struct addr_t\n",
        "  page: u8\n",
        "  off: u8\n",
        "enum req_e: u2\n",
        "  Nop\n",
        "  Read(addr_t)\n",
        "  Halt\n",
        "fun pass (r: req_e, o: out req_e)\n",
        "  o = r\n",
    ));
    assert!(text.contains("its width is not a choice"), "{}", text);
    assert!(text.contains("so a value is 18"), "{}", text);
}

#[test]
fn refusing_the_width_does_not_cascade() {
    // The enum is still registered with its derived width, so everything
    // naming it resolves and the annotation is the only complaint.
    let text = compile_err(concat!(
        "enum e_t: u4\n",
        "  X(u8)\n",
        "  Y\n",
        "fun f (x: e_t, o: out e_t)\n",
        "  o = x\n",
    ));
    assert_eq!(text.matches("error:").count(), 1, "{}", text);
    assert!(!text.contains("is not a type"), "{}", text);
}

#[test]
fn a_width_on_an_enum_with_no_payloads_is_still_honoured() {
    // Nothing that worked before moves: with no payloads the annotation is
    // still the width of the value, which is what k2g_types.ddl depends on to
    // match its SystemVerilog counterpart.
    let v = compile(concat!(
        "enum fault_e: u5\n",
        "  NONE\n",
        "  BAD\n",
        "fun f (x: fault_e, o: out u1)\n",
        "  o = x == BAD\n",
    ));
    assert!(v.contains("input  [4:0] x,"), "{}", v);
}

#[test]
fn an_enum_with_no_payloads_is_unchanged() {
    // The whole point of shifting by the payload width is that it is zero when
    // there are no payloads, so nothing that worked before moves.
    let v = compile(concat!(
        "enum op_e: u2\n",
        "  ADD\n",
        "  SUB\n",
        "  AND_\n",
        "  OR_\n",
        "fun f (o: op_e, r: out u1)\n",
        "  r = o == SUB\n",
    ));
    assert!(v.contains("assign r = (o == 2'd1);"), "{}", v);
}

// ---- resolution order ----------------------------------------------------

#[test]
fn a_payload_may_name_a_struct_declared_after_it() {
    // Enums used to be built before structs, so this could not resolve. They
    // resolve together now, whichever order they are written in.
    let v = compile(concat!(
        "enum e_t\n",
        "  X(later_t)\n",
        "  Y\n",
        "struct later_t\n",
        "  a: u8\n",
        "  b: u8\n",
        "fun f (x: e_t, o: out u8)\n",
        "  var v: u8 = @zeroed()\n",
        "  match x\n",
        "    .X p =>\n",
        "      v = p.a\n",
        "    .Y =>\n",
        "      v = 8'd0\n",
        "  o = v\n",
    ));
    assert!(v.contains("input  [16:0] x,"), "{}", v);
}

#[test]
fn a_struct_field_may_name_a_payload_carrying_enum() {
    let v = compile(concat!(
        "enum e_t\n",
        "  X(u8)\n",
        "  Y\n",
        "struct wrap_t\n",
        "  tag: u4\n",
        "  inner: e_t\n",
        "fun f (w: wrap_t, o: out e_t)\n",
        "  o = w.inner\n",
    ));
    // 4 + (1 + 8) = 13, and the field keeps its enum type across the boundary.
    assert!(v.contains("input  [12:0] w,"), "{}", v);
    assert!(v.contains("output [8:0]  o"), "{}", v);
    assert!(v.contains("w[8:0]"), "{}", v);
}

#[test]
fn a_payload_that_contains_its_own_enum_cannot_be_sized() {
    let text = compile_err(concat!(
        "struct s_t\n",
        "  f: e_t\n",
        "enum e_t\n",
        "  X(s_t)\n",
        "  Y\n",
        "fun f (x: e_t, o: out u1)\n",
        "  o = 1'd0\n",
    ));
    assert!(text.contains("cannot be sized"), "{}", text);
    assert!(text.contains("needs `e_t`'s size to know its own"), "{}", text);
    // And not "is not a type", which would send the reader hunting a typo in a
    // declaration that is right there.
    assert!(!text.contains("`s_t` is not a type"), "{}", text);
}

#[test]
fn a_payload_naming_nothing_still_reads_as_a_typo() {
    let text = compile_err(concat!(
        "enum e_t\n",
        "  X(nosuch_t)\n",
        "  Y\n",
        "fun f (x: e_t, o: out u1)\n",
        "  o = 1'd0\n",
    ));
    assert!(text.contains("`nosuch_t` is not a type"), "{}", text);
}

// ---- what it refuses -----------------------------------------------------

#[test]
fn comparing_a_tagged_union_compares_its_payload_too() {
    // `r == Nop` reads as "is it a Nop" and is not: it also demands that bits
    // meaning nothing for `Nop` are zero.
    let text = compile_err(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun is_nop (r: req_e, o: out u1)\n",
            "  o = r == Nop\n",
        )
    ));
    assert!(text.contains("comparing it compares those too"), "{}", text);
    assert!(text.contains("use `match`"), "{}", text);
}

#[test]
fn a_variant_that_carries_something_needs_it() {
    let text = compile_err(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun mk (o: out req_e)\n",
            "  o = Read\n",
        )
    ));
    assert!(text.contains("`Read` carries a `addr_t`, so it needs one"), "{}", text);
    assert!(text.contains("would leave the payload undefined"), "{}", text);
}

#[test]
fn a_variant_that_carries_nothing_takes_no_arguments() {
    let text = compile_err(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun mk (d: u8, o: out req_e)\n",
            "  o = Nop(d)\n",
        )
    ));
    assert!(text.contains("`Nop` carries no payload"), "{}", text);
}

#[test]
fn a_payload_of_the_wrong_type_is_caught() {
    let text = compile_err(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun mk (d: u16, o: out req_e)\n",
            "  o = Write(d)\n",
        )
    ));
    assert!(text.contains("carries a `u8` but a `u16` was given"), "{}", text);
}

#[test]
fn binding_a_payload_a_variant_does_not_have_is_an_error() {
    let text = compile_err(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun f (r: req_e, o: out u8)\n",
            "  var v: u8 = @zeroed()\n",
            "  match r\n",
            "    .Nop x =>\n",
            "      v = 8'd0\n",
            "    _ =>\n",
            "      v = 8'd1\n",
            "  o = v\n",
        )
    ));
    assert!(text.contains("`Nop` carries no payload to bind"), "{}", text);
}

#[test]
fn a_memory_cannot_be_a_payload() {
    // It is storage rather than a value, so there is nothing to pack.
    let text = compile_err(concat!(
        "enum e_t\n",
        "  X(#[impl(lutram)] [u8; 4])\n",
        "  Y\n",
        "fun f (x: e_t, o: out u1)\n",
        "  o = 1'd0\n",
    ));
    assert!(text.contains("a memory cannot be an enum payload"), "{}", text);
}

#[test]
fn exhaustiveness_still_counts_variants_not_bit_patterns() {
    let text = compile_err(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun f (r: req_e, o: out u2)\n",
            "  var k: u2 = 2'd0\n",
            "  match r\n",
            "    .Nop =>\n",
            "      k = 2'd0\n",
            "    .Read a =>\n",
            "      k = 2'd1\n",
            "  o = k\n",
        )
    ));
    assert!(text.contains("does not cover Write, Halt"), "{}", text);
}

#[test]
fn a_variant_cannot_pin_a_tag_value_and_carry_a_payload() {
    // The two forms are alternatives. A tag value names the bits the variant
    // IS; a payload sits beside a tag the compiler assigns and whose width it
    // picks, so a variant writing both would be fixing a number in a layout it
    // does not control.
    let text = compile_err(concat!(
        "enum e\n",
        "  A(u8) = 5\n",
        "  B\n",
        "fun f (x: e, o: out u1)\n",
        "  o = 1'd1\n",
    ));
    assert!(text.contains("does not belong in an `enum` body"), "{}", text);
    // It points at the `=`, which is the half that does not belong.
    assert!(text.contains("t.ddl:2:9"), "{}", text);
    assert!(text.contains("never both"), "{}", text);
}

#[test]
fn a_discriminant_that_divides_by_zero_says_so() {
    // Not "discriminant must be a constant": `8 / 0` is one, and has no value.
    let text = compile_err(concat!(
        "enum e\n",
        "  A = 8 / 0\n",
        "  B\n",
        "fun f (x: e, o: out u1)\n",
        "  o = 1'd1\n",
    ));
    assert!(text.contains("discriminant divides by zero"), "{}", text);
}
