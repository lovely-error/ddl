// M2 regression tests: enums, `match`, and calls to user-defined functions.
//
// In their own file rather than inside driver.rs because they exercise the
// whole pipeline, not the driver specifically.

use crate::diag::SourceMap;
use crate::driver::compile_to_verilog;
use crate::verilog::EmitOptions;

fn compile(src: &str) -> String {
    let map = SourceMap::new("t.ddl", src);
    let verilog = match compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(v) => v,
        Err(diags) => panic!("compile failed:\n{}", map.render_all(&diags)),
    };
    assert_no_undeclared_nets(&verilog);
    verilog
}

/// Every generated net must be declared before it is read.
///
/// Worth checking on every compile: folding single-use values and aliasing
/// no-op casts were two mechanisms that each removed a wire, and together they
/// once emitted `assign k = n1;` with no `n1` anywhere -- Verilog that no test
/// asserting on substrings would have noticed.
fn assert_no_undeclared_nets(verilog: &str) {
    for module in verilog.split("module ").skip(1) {
        let mut declared: Vec<&str> = Vec::new();
        for line in module.lines() {
            let t = line.trim();
            for kw in ["wire ", "reg "] {
                if let Some(rest) = t.strip_prefix(kw) {
                    // `wire x = e;` and `reg x;` both declare their last name
                    // before the `=` or the `;`.
                    let head = rest.split('=').next().unwrap_or(rest);
                    let head = head.trim_end_matches(';').trim_end();
                    // `reg [31:0] vals [0:31];` declares `vals`, which is the
                    // second-to-last token rather than the last.
                    let head = match head.rfind(" [") {
                        Some(at) if head.ends_with(']') => &head[..at],
                        _ => head,
                    };
                    if let Some(last) = head.split_whitespace().last() {
                        declared.push(last);
                    }
                }
            }
            let is_port = t.starts_with("input ") || t.starts_with("output");
            if is_port {
                let cleaned = t.trim_end_matches([',', ')', ';']);
                if let Some(last) = cleaned.split_whitespace().last() {
                    declared.push(last);
                }
            }
        }
        // Generated temporaries are the ones at risk; `let` names and ports
        // are declared by construction.
        for tok in module.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
            let looks_generated = tok.len() >= 2
                && tok.starts_with('n')
                && tok[1..].bytes().all(|b| b.is_ascii_digit());
            if looks_generated {
                assert!(
                    declared.contains(&tok),
                    "`{}` is read but never declared:\nmodule {}",
                    tok,
                    module
                );
            }
        }
    }
}

fn compile_err(src: &str) -> String {
    let map = SourceMap::new("t.ddl", src);
    match compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(v) => panic!("expected failure, got:\n{}", v),
        Err(diags) => map.render_all(&diags),
    }
}

const OPS: &str = concat!(
    "enum op_e: i2\n",
    "  OP_ADD\n",
    "  OP_SUB\n",
    "  OP_AND\n",
    "  OP_OR\n",
);

// ---- enums ---------------------------------------------------------------

#[test]
fn implicit_discriminants_count_up() {
    let v = compile(&format!(
        "{}fun f (op: op_e, o: out i1)\n  o = op == OP_AND\n",
        OPS
    ));
    // OP_AND is the third variant, so 2.
    assert!(v.contains("2'd2"), "{}", v);
    // Two bits wide, so the port is [1:0].
    assert!(v.contains("input  [1:0] op"), "{}", v);
}

#[test]
fn explicit_discriminants_are_honoured() {
    let v = compile(concat!(
        "enum lb_e: i6\n",
        "  LB_ADD = 6'b110101\n",
        "  LB_EP1 = 6'b111111\n",
        "fun f (op: lb_e, o: out i1)\n",
        "  o = op == LB_ADD\n",
    ));
    assert!(v.contains("6'h35"), "{}", v);
    assert!(v.contains("input  [5:0] op"), "{}", v);
}

#[test]
fn the_tag_width_is_inferred_when_not_written() {
    let v = compile(concat!(
        "enum small_e\n",
        "  A\n",
        "  B\n",
        "  C\n",
        "fun f (x: small_e, o: out i1)\n",
        "  o = x == C\n",
    ));
    // Largest discriminant 2, so two bits.
    assert!(v.contains("input  [1:0] x"), "{}", v);
}

#[test]
fn a_tag_too_narrow_for_its_variants_is_rejected() {
    let text = compile_err(concat!(
        "enum bad_e: i1\n",
        "  A\n",
        "  B\n",
        "  C\n",
        "fun f (x: bad_e, o: out i1)\n",
        "  o = x == A\n",
    ));
    assert!(text.contains("1 bits wide"), "{}", text);
    assert!(text.contains("needs 2"), "{}", text);
}

#[test]
fn duplicate_discriminant_values_are_rejected() {
    // SystemVerilog rejects duplicate labels but accepts duplicate values,
    // which is the dangerous direction.
    let text = compile_err(concat!(
        "enum dup_e: i2\n",
        "  A = 1\n",
        "  B = 1\n",
        "fun f (x: dup_e, o: out i1)\n",
        "  o = x == A\n",
    ));
    assert!(text.contains("already used"), "{}", text);
}

#[test]
fn enums_do_arithmetic_nowhere() {
    let text = compile_err(&format!(
        "{}fun f (op: op_e, o: out i2)\n  o = op + op\n",
        OPS
    ));
    assert!(text.contains("expected an integer"), "{}", text);
}

// ---- match ---------------------------------------------------------------

#[test]
fn match_becomes_a_case_statement() {
    let v = compile(&format!(
        concat!(
            "{}fun f (op: op_e, a: i8, b: i8, o: out i8)\n",
            "  match op\n",
            "    .OP_ADD =>\n",
            "      o = a + b\n",
            "    .OP_SUB =>\n",
            "      o = a - b\n",
            "    _ =>\n",
            "      o = a & b\n",
        ),
        OPS
    ));
    // A ternary chain is a PRIORITY structure synthesis must honour in
    // order; a case says the arms are parallel. Measured on the GW1NR-9C:
    // 536 cells as nested ternaries against 210 as a case.
    assert!(v.contains("always @* begin"), "{}", v);
    assert!(v.contains("case (op)"), "{}", v);
    assert!(v.contains("2'd0: "), "{}", v);
    assert!(v.contains("2'd1: "), "{}", v);
    assert!(v.contains("default: "), "{}", v);
    assert!(v.contains("a + b"), "{}", v);
    assert!(v.contains("a - b"), "{}", v);
    assert!(v.contains("a & b"), "{}", v);
}

#[test]
fn a_match_emits_no_mux_for_bindings_no_arm_touched() {
    // Every arm used to emit `cond ? x : x` for every name in scope.
    let v = compile(&format!(
        concat!(
            "{}fun f (op: op_e, a: i8, b: i8, c: i8, d: i8, o: out i8)\n",
            "  let untouched = c & d\n",
            "  match op\n",
            "    .OP_ADD =>\n",
            "      o = a\n",
            "    _ =>\n",
            "      o = b\n",
        ),
        OPS
    ));
    // Only `o` differs between the arms, so only `o` gets a case.
    assert_eq!(v.matches("always @* begin").count(), 1, "{}", v);
    assert!(!v.contains("untouched"), "an untouched binding needs no case:\n{}", v);
}

#[test]
fn a_non_exhaustive_match_is_rejected_as_a_latch() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (op: op_e, a: i8, o: out i8)\n",
            "  match op\n",
            "    .OP_ADD =>\n",
            "      o = a\n",
            "    .OP_SUB =>\n",
            "      o = a\n",
        ),
        OPS
    ));
    assert!(text.contains("does not cover"), "{}", text);
    assert!(text.contains("OP_AND"), "{}", text);
    assert!(text.contains("OP_OR"), "{}", text);
    assert!(text.contains("latch"), "{}", text);
}

#[test]
fn covering_every_variant_needs_no_catch_all() {
    let v = compile(concat!(
        "enum two_e: i1\n",
        "  LO\n",
        "  HI\n",
        "fun f (x: two_e, a: i8, b: i8, o: out i8)\n",
        "  match x\n",
        "    .LO =>\n",
        "      o = a\n",
        "    .HI =>\n",
        "      o = b\n",
    ));
    assert!(v.contains("case (x)"), "{}", v);
}

#[test]
fn an_unknown_variant_lists_the_real_ones() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (op: op_e, a: i8, o: out i8)\n",
            "  match op\n",
            "    .OP_NOPE =>\n",
            "      o = a\n",
            "    _ =>\n",
            "      o = a\n",
        ),
        OPS
    ));
    assert!(text.contains("not a variant"), "{}", text);
    assert!(text.contains("OP_ADD, OP_SUB, OP_AND, OP_OR"), "{}", text);
}

#[test]
fn a_variant_matched_twice_is_rejected() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (op: op_e, a: i8, o: out i8)\n",
            "  match op\n",
            "    .OP_ADD =>\n",
            "      o = a\n",
            "    .OP_ADD =>\n",
            "      o = a\n",
            "    _ =>\n",
            "      o = a\n",
        ),
        OPS
    ));
    assert!(text.contains("matched twice"), "{}", text);
}

#[test]
fn an_arm_after_a_catch_all_is_unreachable() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (op: op_e, a: i8, o: out i8)\n",
            "  match op\n",
            "    _ =>\n",
            "      o = a\n",
            "    .OP_ADD =>\n",
            "      o = a\n",
        ),
        OPS
    ));
    assert!(text.contains("unreachable"), "{}", text);
}

#[test]
fn matching_on_a_plain_integer_is_rejected() {
    let text = compile_err(concat!(
        "fun f (x: i2, a: i8, o: out i8)\n",
        "  match x\n",
        "    _ =>\n",
        "      o = a\n",
    ));
    assert!(text.contains("needs an enum"), "{}", text);
}

// ---- calls ---------------------------------------------------------------

#[test]
fn a_call_is_inlined() {
    let v = compile(concat!(
        "fun helper (t: i3, o: out i1)\n",
        "  o = t[2]\n",
        "fun caller (t: i3, o: out i1)\n",
        "  o = helper(t)\n",
    ));
    let caller = v.split("module caller").nth(1).expect("caller emitted");
    assert!(caller.contains("t[2]"), "inlined, not instantiated:\n{}", v);
}

#[test]
fn a_function_declared_later_is_still_callable() {
    let v = compile(concat!(
        "fun caller (t: i3, o: out i1)\n",
        "  o = helper(t)\n",
        "fun helper (t: i3, o: out i1)\n",
        "  o = t[0]\n",
    ));
    assert!(v.contains("module caller"), "{}", v);
}

#[test]
fn call_arity_is_checked() {
    let text = compile_err(concat!(
        "fun helper (a: i3, b: i3, o: out i1)\n",
        "  o = a[0] & b[0]\n",
        "fun caller (t: i3, o: out i1)\n",
        "  o = helper(t)\n",
    ));
    assert!(text.contains("takes 2 argument(s), found 1"), "{}", text);
}

#[test]
fn call_argument_types_are_checked() {
    let text = compile_err(concat!(
        "fun helper (a: i3, o: out i1)\n",
        "  o = a[0]\n",
        "fun caller (t: i8, o: out i1)\n",
        "  o = helper(t)\n",
    ));
    assert!(text.contains("is `i3` but `i8` was given"), "{}", text);
}

#[test]
fn a_recursive_call_is_rejected_as_a_circuit_that_never_settles() {
    let text = compile_err(concat!(
        "fun loopy (a: i3, o: out i1)\n",
        "  o = loopy(a)\n",
    ));
    assert!(text.contains("calls itself"), "{}", text);
    assert!(text.contains("never settles"), "{}", text);
}

#[test]
fn a_multi_output_function_cannot_be_an_expression() {
    let text = compile_err(concat!(
        "fun two (a: i3, x: out i1, y: out i1)\n",
        "  x = a[0]\n",
        "  y = a[1]\n",
        "fun caller (t: i3, o: out i1)\n",
        "  o = two(t)\n",
    ));
    assert!(text.contains("cannot be used as an expression"), "{}", text);
}

#[test]
fn calling_something_that_is_not_a_function_is_rejected() {
    let text = compile_err(concat!(
        "fun caller (t: i3, o: out i1)\n",
        "  o = nope(t)\n",
    ));
    assert!(text.contains("is not a function"), "{}", text);
}

// ---- whole-file --------------------------------------------------------

#[test]
fn a_duplicate_declaration_is_rejected() {
    let text = compile_err(concat!(
        "fun f (a: i1, o: out i1)\n",
        "  o = a\n",
        "fun f (a: i1, o: out i1)\n",
        "  o = a\n",
    ));
    assert!(text.contains("declared more than once"), "{}", text);
}

#[test]
fn the_banner_appears_once_for_a_multi_module_file() {
    let v = compile(concat!(
        "fun a1 (x: i1, o: out i1)\n",
        "  o = x\n",
        "fun a2 (x: i1, o: out i1)\n",
        "  o = x\n",
    ));
    assert_eq!(v.matches("GENERATED FILE").count(), 1, "{}", v);
    assert_eq!(v.matches("endmodule").count(), 2, "{}", v);
}

/// Every example must keep compiling, from its own path, resolving its own
/// imports -- which is how a person compiles it. Bit-exactness is proven by
/// examples/verify.sh against Questa; this catches a compiler change that
/// stops them building at all.
///
/// The k2g_* ports need `k2g_pkg.ddl`, which is generated into the consumer's
/// tree by emu/src/ddl_gen.rs and is not part of this repository. Those are
/// skipped when it is absent.
#[cfg_attr(miri, ignore = "reads files; Miri has no Windows path shims")]
#[test]
fn the_examples_still_compile() {
    let k2g = std::path::PathBuf::from("../KAMASUTRA2G/rtl");
    let have_pkg = k2g.join("k2g_pkg.ddl").is_file();

    let mut compiled = 0;
    for entry in std::fs::read_dir("examples").expect("examples/ is beside Cargo.toml") {
        let path = entry.expect("readable entry").path();
        if path.extension().is_none_or(|e| e != "ddl") {
            continue;
        }
        let name = path.file_name().expect("a file").to_string_lossy().to_string();
        // A types-only file emits no module, and "nothing to emit" is a real
        // error for a build but not for this one.
        if name == "k2g_types.ddl" {
            continue;
        }
        if name.starts_with("k2g_") && !have_pkg {
            continue;
        }

        let search = if have_pkg { vec![k2g.clone()] } else { Vec::new() };
        let (map, load_diags) =
            crate::source::load_program(&[path.display().to_string()], search)
                .expect("the example is readable");
        assert!(
            load_diags.is_empty(),
            "{} has an unresolved import:
{}",
            name,
            map.render_all(&load_diags)
        );
        if let Err(diags) = compile_to_verilog(&map, &EmitOptions::default()) {
            panic!("{} stopped compiling:
{}", name, map.render_all(&diags));
        }
        compiled += 1;
    }

    // An anti-vacuous guard: the standalone examples need nothing external, so
    // a run that compiled none of them found no examples at all.
    assert!(compiled >= 3, "only {} examples compiled", compiled);
}

/// The checked-in Verilog is generated, so it can go stale. `ddl build
/// --check` exists for exactly that and was only ever run by hand, on the one
/// machine with Questa on it.
///
/// The comparison is against the banner the file already carries, because the
/// banner has to be the command that reproduces the file -- which is what
/// makes running it from anywhere give the same bytes.
#[cfg_attr(miri, ignore = "reads files; Miri has no Windows path shims")]
#[test]
fn the_checked_in_verilog_is_up_to_date() {
    let k2g = std::path::PathBuf::from("../KAMASUTRA2G/rtl");
    let have_pkg = k2g.join("k2g_pkg.ddl").is_file();

    let mut checked = 0;
    for name in [
        "mul3", "fsm_adder", "k3g_stage", "pipeline_graph", "reg_port", "tagged",
        "bram_lookup", "k2g_shift",
        "k2g_alu",
        "k2g_decode",
        "k2g_xstage",
    ] {
        if name.starts_with("k2g_") && !have_pkg {
            continue;
        }
        let src = format!("examples/{}.ddl", name);
        let out = format!("examples/{}.v", name);
        let search = if name.starts_with("k2g_") { vec![k2g.clone()] } else { Vec::new() };
        let (map, load_diags) = crate::source::load_program(std::slice::from_ref(&src), search)
            .expect("the example is readable");
        assert!(load_diags.is_empty(), "{}", map.render_all(&load_diags));

        let current = std::fs::read_to_string(&out)
            .unwrap_or_else(|e| panic!("cannot read {}: {}", out, e))
            .replace("\r\n", "\n");
        let opts = EmitOptions { regenerate_cmd: banner_cmd(&current) };
        let fresh = match compile_to_verilog(&map, &opts) {
            Ok(v) => v,
            Err(diags) => panic!("{} stopped compiling:
{}", src, map.render_all(&diags)),
        };
        assert_eq!(
            current, fresh,
            "{} is out of date; regenerate with `{}`",
            out, opts.regenerate_cmd
        );
        checked += 1;
    }
    assert!(checked >= 3, "only {} generated files checked", checked);
}

/// The command out of a generated file's banner.
fn banner_cmd(verilog: &str) -> String {
    verilog
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("// Regenerate with: "))
        .unwrap_or_default()
        .to_string()
}

// ---- compound assignment -------------------------------------------------

#[test]
fn a_compound_assignment_reads_then_writes() {
    let v = compile(concat!(
        "fun bump (a: i8, o: out i8)
",
        "  var acc: i8 = a
",
        "  acc += 8'd3
",
        "  acc <<= 8'd1
",
        "  acc ^= 8'hF0
",
        "  o = acc
",
    ));
    assert!(v.contains("(((a + 8'd3) << 8'd1) ^ 8'hF0)"), "{}", v);
}

#[test]
fn a_compound_assignment_to_a_field_splices_the_same_bits() {
    // The whole reason to desugar in lowering rather than in the parser: the
    // read of `r.lo` and the write of it go through one field-path resolution,
    // so this is the plain form's splice with an adder in the middle.
    let v = compile(&format!(
        "{}{}",
        REQ,
        concat!(
            "fun bump (r: req_t, o: out req_t)
",
            "  var q: req_t = r
",
            "  q.data += 8'd1
",
            "  o = q
",
        )
    ));
    assert!(v.contains("r[7:0] + 8'd1"), "{}", v);
    assert!(v.contains("{r[25:8],"), "{}", v);
}

#[test]
fn every_arithmetic_and_bitwise_operator_has_a_compound_form() {
    for (op, verilog) in [
        ("+=", " + "),
        ("-=", " - "),
        ("&=", " & "),
        ("|=", " | "),
        ("^=", " ^ "),
        ("<<=", " << "),
        (">>=", " >> "),
    ] {
        let v = compile(&format!(
            "fun f (a: i8, o: out i8)
  var x: i8 = a
  x {} 8'd1
  o = x
",
            op
        ));
        assert!(v.contains(verilog), "`{}` did not lower to `{}`:
{}", op, verilog, v);
    }
}

#[test]
fn a_compound_assignment_is_width_checked_like_a_plain_one() {
    let text = compile_err(concat!(
        "fun bad (a: i8, b: i32, o: out i8)
",
        "  var x: i8 = a
",
        "  x += b
",
        "  o = x
",
    ));
    assert!(text.contains("width mismatch"), "{}", text);
}

#[test]
fn tilde_assign_is_refused_by_name() {
    // `~` is unary inversion, so `x ~= y` is a guess between "invert" and
    // "not equal". The diagnostic says so rather than picking one.
    let text = compile_err(concat!(
        "fun bad (a: i8, o: out i8)
",
        "  var x: i8 = a
",
        "  x ~= 8'd1
",
        "  o = x
",
    ));
    assert!(text.contains("`~=` is not supported yet"), "{}", text);
}

#[test]
fn a_let_binding_still_cannot_be_compound_assigned() {
    let text = compile_err(concat!(
        "fun bad (a: i8, o: out i8)
",
        "  let x: i8 = a
",
        "  x += 8'd1
",
        "  o = x
",
    ));
    assert!(text.contains("cannot be assigned"), "{}", text);
}

// ---- structs -------------------------------------------------------------

const REQ: &str = concat!(
    "enum kind_e: i2\n",
    "  K_LOAD\n",
    "  K_STORE\n",
    "  K_ALU\n",
    "struct req_t\n",
    "  kind: kind_e\n",
    "  addr: i16\n",
    "  data: i8\n",
);

#[test]
fn a_struct_packs_first_field_into_the_high_bits() {
    // Matching SystemVerilog packed structs, so a DDL struct and its SV
    // counterpart have the same layout across a module boundary.
    let v = compile(&format!(
        "{}fun unpack (r: req_t, k: out kind_e, a: out i16, d: out i8)\n  k = r.kind\n  a = r.addr\n  d = r.data\n",
        REQ
    ));
    assert!(v.contains("input  [25:0] r"), "26 bits total:\n{}", v);
    assert!(v.contains("r[25:24]"), "kind is highest:\n{}", v);
    assert!(v.contains("r[23:8]"), "then addr:\n{}", v);
    assert!(v.contains("r[7:0]"), "then data:\n{}", v);
}

#[test]
fn a_struct_is_built_with_call_syntax() {
    let v = compile(&format!(
        "{}fun pack (k: kind_e, a: i16, d: i8, r: out req_t)\n  r = req_t(k, a, d)\n",
        REQ
    ));
    assert!(v.contains("{k, a, d}"), "{}", v);
    assert!(v.contains("output [25:0] r"), "{}", v);
}

#[test]
fn a_field_keeps_its_own_type() {
    // The slice is a bag of bits, but `kind` must come back out as an enum or
    // it could not be matched on.
    let v = compile(&format!(
        concat!(
            "{}fun f (r: req_t, yes: out i1)\n",
            "  match r.kind\n",
            "    .K_STORE =>\n",
            "      yes = 1'b1\n",
            "    _ =>\n",
            "      yes = 1'b0\n",
        ),
        REQ
    ));
    assert!(v.contains("r[25:24]"), "{}", v);
    assert!(v.contains("case ("), "{}", v);
}

#[test]
fn an_unknown_field_lists_the_real_ones() {
    let text = compile_err(&format!(
        "{}fun f (r: req_t, o: out i8)\n  o = r.nope\n",
        REQ
    ));
    assert!(text.contains("has no field `nope`"), "{}", text);
    assert!(text.contains("kind, addr, data"), "{}", text);
}

#[test]
fn struct_construction_checks_arity_and_types() {
    let text = compile_err(&format!(
        "{}fun f (k: kind_e, a: i16, r: out req_t)\n  r = req_t(k, a)\n",
        REQ
    ));
    assert!(text.contains("has 3 field(s), found 2"), "{}", text);

    let text = compile_err(&format!(
        "{}fun f (k: kind_e, a: i16, d: i32, r: out req_t)\n  r = req_t(k, a, d)\n",
        REQ
    ));
    assert!(text.contains("field `data`"), "{}", text);
    assert!(text.contains("`i8` but `i32` was given"), "{}", text);
}

#[test]
fn taking_a_field_of_a_non_struct_is_rejected() {
    let text = compile_err("fun f (x: i8, o: out i8)\n  o = x.nope\n");
    assert!(text.contains("has no fields"), "{}", text);
}

#[test]
fn a_struct_round_trips_through_pack_and_unpack() {
    let v = compile(&format!(
        concat!(
            "{}fun pack (k: kind_e, a: i16, d: i8, r: out req_t)\n",
            "  r = req_t(k, a, d)\n",
            "fun roundtrip (k: kind_e, a: i16, d: i8, o: out i16)\n",
            "  let packed = req_t(k, a, d)\n",
            "  o = packed.addr\n",
        ),
        REQ
    ));
    // The concat and the slice cancel out in synthesis, but both must appear.
    assert!(v.contains("{k, a, d}"), "{}", v);
    assert!(v.contains("[23:8]"), "{}", v);
}

#[test]
fn a_struct_field_may_be_another_struct() {
    let v = compile(concat!(
        "struct inner_t\n",
        "  lo: i4\n",
        "  hi: i4\n",
        "struct outer_t\n",
        "  tag: i2\n",
        "  body: inner_t\n",
        "fun f (o: outer_t, r: out i4)\n",
        "  r = o.body.hi\n",
    ));
    assert!(v.contains("input  [9:0] o"), "2 + 8 bits:\n{}", v);
    // body is [7:0]; hi is its high nibble, so [7:4] of the whole.
    assert!(v.contains("[7:0]"), "{}", v);
}

#[test]
fn a_sparse_enum_needs_a_wildcard_even_when_every_variant_is_covered() {
    // 3 variants in a 2-bit tag leaves 2'b11 naming no variant. Covering all
    // three arms used to be accepted, and the LAST arm silently became the
    // fallback -- so reordering otherwise-equivalent arms changed what 2'b11
    // produced.
    let text = compile_err(concat!(
        "enum logic_e: i2\n",
        "  LOGIC_AND\n",
        "  LOGIC_OR\n",
        "  LOGIC_XOR\n",
        "fun f (op: logic_e, a: i8, b: i8, o: out i8)\n",
        "  match op\n",
        "    .LOGIC_AND =>\n",
        "      o = a & b\n",
        "    .LOGIC_OR =>\n",
        "      o = a | b\n",
        "    .LOGIC_XOR =>\n",
        "      o = a ^ b\n",
    ));
    assert!(text.contains("covers every variant"), "{}", text);
    assert!(text.contains("1 pattern(s) that name no variant"), "{}", text);
    assert!(text.contains("reordering the arms"), "{}", text);
}

#[test]
fn a_dense_enum_still_needs_no_wildcard() {
    // 4 variants in a 2-bit tag: every pattern names a variant, so covering
    // them all really is exhaustive and the check stays out of the way.
    let v = compile(&format!(
        concat!(
            "{}fun f (op: op_e, a: i8, b: i8, o: out i8)\n",
            "  match op\n",
            "    .OP_ADD =>\n",
            "      o = a + b\n",
            "    .OP_SUB =>\n",
            "      o = a - b\n",
            "    .OP_AND =>\n",
            "      o = a & b\n",
            "    .OP_OR =>\n",
            "      o = a | b\n",
        ),
        OPS
    ));
    assert!(v.contains("case (op)"), "{}", v);
    assert_eq!(v.matches("      2'd").count(), 3, "three labelled arms:\n{}", v);
}

// ---- @unreachable, and CRLF ----------------------------------------------

const SPARSE: &str = concat!(
    "enum logic_e: i2\n",
    "  LOGIC_AND\n",
    "  LOGIC_OR\n",
    "  LOGIC_XOR\n",
);

#[test]
fn unreachable_lets_a_sparse_enum_be_covered_by_name() {
    // The three variants sit in a 2-bit tag, so 2'b11 names no variant. The
    // author asserts it cannot occur, which is what `unique case` means.
    let v = compile(&format!(
        concat!(
            "{}fun f (op: logic_e, a: i8, b: i8, o: out i8)\n",
            "  match op\n",
            "    .LOGIC_AND =>\n",
            "      o = a & b\n",
            "    .LOGIC_OR =>\n",
            "      o = a | b\n",
            "    .LOGIC_XOR =>\n",
            "      o = a ^ b\n",
            "    _ => @unreachable\n",
        ),
        SPARSE
    ));
    // The last real arm serves the impossible pattern, which is sound
    // precisely because it was declared impossible.
    assert!(v.contains("case (op)"), "{}", v);
    assert!(v.contains("default: "), "{}", v);
    assert!(v.contains("a ^ b"), "{}", v);
}

#[test]
fn unreachable_does_not_excuse_a_missing_variant() {
    // A declared variant is a value that can occur, so `@unreachable` must not
    // be usable to skip one.
    let text = compile_err(&format!(
        concat!(
            "{}fun f (op: logic_e, a: i8, b: i8, o: out i8)\n",
            "  match op\n",
            "    .LOGIC_AND =>\n",
            "      o = a & b\n",
            "    _ => @unreachable\n",
        ),
        SPARSE
    ));
    assert!(text.contains("does not cover"), "{}", text);
    assert!(text.contains("LOGIC_OR"), "{}", text);
    assert!(text.contains("LOGIC_XOR"), "{}", text);
}

#[test]
fn unreachable_must_sit_on_a_catch_all() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (op: logic_e, a: i8, b: i8, o: out i8)\n",
            "  match op\n",
            "    .LOGIC_AND =>\n",
            "      o = a & b\n",
            "    .LOGIC_OR =>\n",
            "      o = a | b\n",
            "    .LOGIC_XOR => @unreachable\n",
            "    _ =>\n",
            "      o = a\n",
        ),
        SPARSE
    ));
    assert!(text.contains("belongs on a `_` arm"), "{}", text);
}

#[test]
fn the_sparse_enum_error_names_unreachable_as_an_option() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (op: logic_e, a: i8, b: i8, o: out i8)\n",
            "  match op\n",
            "    .LOGIC_AND =>\n",
            "      o = a & b\n",
            "    .LOGIC_OR =>\n",
            "      o = a | b\n",
            "    .LOGIC_XOR =>\n",
            "      o = a ^ b\n",
        ),
        SPARSE
    ));
    assert!(text.contains("@unreachable"), "{}", text);
}

#[test]
fn a_match_diagnostic_points_at_the_match_not_the_file() {
    // It used to anchor at 1:1, which is useless in a module with several.
    let text = compile_err(&format!(
        concat!(
            "{}fun f (op: logic_e, a: i8, b: i8, o: out i8)\n",
            "  match op\n",
            "    .LOGIC_AND =>\n",
            "      o = a & b\n",
            "    .LOGIC_OR =>\n",
            "      o = a | b\n",
            "    .LOGIC_XOR =>\n",
            "      o = a ^ b\n",
        ),
        SPARSE
    ));
    assert!(text.contains("t.ddl:7:6"), "should point at .LOGIC_AND:\n{}", text);
}

#[test]
fn every_unhandled_match_is_reported_not_just_the_first() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (op: logic_e, a: i8, b: i8, x: out i8, y: out i8)\n",
            "  match op\n",
            "    .LOGIC_AND =>\n",
            "      x = a\n",
            "    .LOGIC_OR =>\n",
            "      x = b\n",
            "    .LOGIC_XOR =>\n",
            "      x = a\n",
            "  match op\n",
            "    .LOGIC_AND =>\n",
            "      y = a\n",
            "    .LOGIC_OR =>\n",
            "      y = b\n",
            "    .LOGIC_XOR =>\n",
            "      y = a\n",
        ),
        SPARSE
    ));
    assert_eq!(
        text.matches("covers every variant").count(),
        2,
        "both matches should be reported:\n{}",
        text
    );
}

/// Blocks are delimited by indentation, so a line-ending bug does not produce
/// a syntax error -- it changes which block a statement belongs to, or makes a
/// body vanish. This repo is on Windows, so CRLF is the default hazard.
#[test]
fn crlf_parses_the_same_as_lf() {
    let lf = format!(
        concat!(
            "{}-- a comment, which must not disturb the block probe\n",
            "fun f (op: logic_e, a: i8, b: i8, o: out i8)\n",
            "  match op\n",
            "    .LOGIC_AND =>\n",
            "      o = a & b\n",
            "    .LOGIC_OR =>\n",
            "      o = a | b\n",
            "    .LOGIC_XOR =>\n",
            "      o = a ^ b\n",
            "    _ => @unreachable\n",
        ),
        SPARSE
    );
    let crlf = lf.replace('\n', "\r\n");

    let from_lf = compile(&lf);
    let from_crlf = compile(&crlf);
    assert_eq!(from_lf, from_crlf, "CRLF must generate identical Verilog");
}

#[test]
fn crlf_survives_nested_blocks() {
    let lf = concat!(
        "fun f (c: i1, a: i8, b: i8, o: out i8)\n",
        "  if c then\n",
        "    o = a\n",
        "  else\n",
        "    o = b\n",
    );
    let from_lf = compile(lf);
    let from_crlf = compile(&lf.replace('\n', "\r\n"));
    assert_eq!(from_lf, from_crlf);
}

// ---- or-patterns ---------------------------------------------------------

/// Dense: four variants filling a two-bit tag exactly.
const LB4: &str = concat!(
    "enum lb_e: i2\n",
    "  LB_ADD\n",
    "  LB_SUB\n",
    "  LB_AND\n",
    "  LB_OR\n",
);

#[test]
fn an_or_pattern_makes_a_grouped_match_exhaustive() {
    // The point of the feature: k2g_decode.sv groups opcodes into eight case
    // items. Without `|` that is either duplicated bodies or a `_` that turns
    // the check off, on the one enum whose drift history motivated it.
    let v = compile(&format!(
        concat!(
            "{}fun f (lb: lb_e, a: i8, b: i8, o: out i8)\n",
            "  match lb\n",
            "    .LB_ADD | .LB_SUB =>\n",
            "      o = a + b\n",
            "    .LB_AND | .LB_OR =>\n",
            "      o = a & b\n",
        ),
        LB4
    ));
    assert!(v.contains("a + b"), "{}", v);
    assert!(v.contains("a & b"), "{}", v);
}

#[test]
fn an_or_pattern_becomes_several_labels_on_one_arm() {
    // `LB_ADD, LB_SUB, LB_MUL, LB_DIV:` is exactly how the SystemVerilog
    // groups opcodes, and is why or-patterns were worth building.
    let v = compile(&format!(
        concat!(
            "{}fun f (lb: lb_e, a: i8, b: i8, o: out i8)\n",
            "  match lb\n",
            "    .LB_ADD | .LB_SUB | .LB_AND =>\n",
            "      o = a + b\n",
            "    .LB_OR =>\n",
            "      o = a & b\n",
        ),
        LB4
    ));
    assert!(v.contains("case (lb)"), "{}", v);
    assert!(v.contains("2'd0, 2'd1, 2'd2:"), "one arm, three labels:\n{}", v);
}

#[test]
fn a_final_or_pattern_costs_no_logic() {
    // The last arm is the default by elimination, so its selector is dead and
    // liveness drops it -- grouping must not cost gates.
    let v = compile(&format!(
        concat!(
            "{}fun f (lb: lb_e, a: i8, b: i8, o: out i8)\n",
            "  match lb\n",
            "    .LB_ADD =>\n",
            "      o = a + b\n",
            "    .LB_SUB | .LB_AND | .LB_OR =>\n",
            "      o = a & b\n",
        ),
        LB4
    ));
    // The grouped arm is last, so it is the default and costs no labels.
    assert!(v.contains("default: "), "{}", v);
    assert_eq!(v.matches("2'd").count(), 1, "one label only:\n{}", v);
}

#[test]
fn an_or_pattern_still_reports_a_missing_variant() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (lb: lb_e, a: i8, o: out i8)\n",
            "  match lb\n",
            "    .LB_ADD | .LB_SUB =>\n",
            "      o = a\n",
            "    .LB_AND =>\n",
            "      o = a\n",
        ),
        LB4
    ));
    assert!(text.contains("does not cover LB_OR"), "{}", text);
}

#[test]
fn a_variant_repeated_across_alternatives_is_rejected() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (lb: lb_e, a: i8, o: out i8)\n",
            "  match lb\n",
            "    .LB_ADD | .LB_ADD =>\n",
            "      o = a\n",
            "    _ =>\n",
            "      o = a\n",
        ),
        LB4
    ));
    assert!(text.contains("matched twice"), "{}", text);
}

#[test]
fn a_variant_repeated_in_a_later_arm_is_rejected() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (lb: lb_e, a: i8, o: out i8)\n",
            "  match lb\n",
            "    .LB_ADD | .LB_SUB =>\n",
            "      o = a\n",
            "    .LB_SUB =>\n",
            "      o = a\n",
            "    _ =>\n",
            "      o = a\n",
        ),
        LB4
    ));
    assert!(text.contains("matched twice"), "{}", text);
}

#[test]
fn an_alternative_cannot_mix_a_binding_with_variants() {
    // `.LB_ADD | x` would match everything and leave the named one dead.
    let text = compile_err(&format!(
        concat!(
            "{}fun f (lb: lb_e, a: i8, o: out i8)\n",
            "  match lb\n",
            "    .LB_ADD | x =>\n",
            "      o = a\n",
            "    _ =>\n",
            "      o = a\n",
        ),
        LB4
    ));
    assert!(text.contains("cannot mix a catch-all"), "{}", text);
}

#[test]
fn an_unknown_variant_inside_an_alternative_is_caught() {
    let text = compile_err(&format!(
        concat!(
            "{}fun f (lb: lb_e, a: i8, o: out i8)\n",
            "  match lb\n",
            "    .LB_ADD | .LB_NOPE =>\n",
            "      o = a\n",
            "    _ =>\n",
            "      o = a\n",
        ),
        LB4
    ));
    assert!(text.contains("`LB_NOPE` is not a variant"), "{}", text);
}

#[test]
fn or_patterns_compose_with_unreachable_on_a_sparse_enum() {
    let v = compile(&format!(
        concat!(
            "{}fun f (op: logic_e, a: i8, b: i8, o: out i8)\n",
            "  match op\n",
            "    .LOGIC_AND | .LOGIC_OR =>\n",
            "      o = a & b\n",
            "    .LOGIC_XOR =>\n",
            "      o = a ^ b\n",
            "    _ => @unreachable\n",
        ),
        SPARSE
    ));
    assert!(v.contains("a & b"), "{}", v);
    assert!(v.contains("a ^ b"), "{}", v);
}

// ---- field assignment and @zeroed() --------------------------------------

const UOP: &str = concat!(
    "enum kind_e: i2\n",
    "  K_NOP\n",
    "  K_ADD\n",
    "  K_SUB\n",
    "  K_LD\n",
    "struct uop_t\n",
    "  kind: kind_e\n",
    "  dst: i5\n",
    "  imm: i8\n",
);

#[test]
fn a_struct_is_built_field_by_field() {
    // How k2g_decode writes a 28-field uop: zero it, then assign the fields
    // that this opcode uses.
    let v = compile(&format!(
        concat!(
            "{}fun build (k: kind_e, d: i5, i: i8, o: out uop_t)\n",
            "  var u: uop_t = @zeroed()\n",
            "  u.kind = k\n",
            "  u.dst = d\n",
            "  u.imm = i\n",
            "  o = u\n",
        ),
        UOP
    ));
    assert!(v.contains("15'd0"), "starts from zero:\n{}", v);
    assert!(v.contains("output [14:0] o"), "2 + 5 + 8 bits:\n{}", v);
    // Each write keeps the bits it did not touch.
    assert!(v.contains("{k, "), "{}", v);
    assert!(v.contains(", d, "), "{}", v);
    assert!(v.contains(", i}"), "{}", v);
}

/// Verilog-2005 allows a part-select only on a name -- neither `15'd0[12:0]`
/// nor `{a, b}[14:8]` is legal. Both fell out of folding: the first from
/// always folding constants, the second from field assignment rebuilding a
/// struct by concatenation and slicing it apart again.
#[test]
fn a_part_select_always_reads_from_a_name() {
    let v = compile(&format!(
        concat!(
            "{}fun build (k: kind_e, d: i5, i: i8, o: out uop_t)\n",
            "  var u: uop_t = @zeroed()\n",
            "  u.kind = k\n",
            "  u.dst = d\n",
            "  u.imm = i\n",
            "  o = u\n",
        ),
        UOP
    ));
    for line in v.lines() {
        if let Some(open) = line.find('[') {
            let before = &line[..open];
            let is_select = before.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
            let is_decl = before.trim_start().starts_with("wire")
                || before.trim_start().starts_with("input")
                || before.trim_start().starts_with("output");
            if is_select && !is_decl {
                let base: String = before
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                let base: String = base.chars().rev().collect();
                assert!(
                    !base.chars().next().unwrap_or('a').is_ascii_digit(),
                    "part-select on a literal:\n{}",
                    line
                );
            }
        }
        assert!(!line.contains("}["), "part-select on a concatenation:\n{}", line);
    }
}

#[test]
fn a_nested_field_can_be_assigned() {
    let v = compile(concat!(
        "struct inner_t\n",
        "  lo: i4\n",
        "  hi: i4\n",
        "struct outer_t\n",
        "  tag: i2\n",
        "  body: inner_t\n",
        "fun f (t: i2, x: i4, o: out outer_t)\n",
        "  var v: outer_t = @zeroed()\n",
        "  v.tag = t\n",
        "  v.body.hi = x\n",
        "  o = v\n",
    ));
    assert!(v.contains("output [9:0] o"), "{}", v);
    assert!(v.contains("10'd0"), "{}", v);
}

#[test]
fn assigning_an_unknown_field_is_rejected() {
    let text = compile_err(&format!(
        "{}fun f (k: kind_e, o: out uop_t)\n  var u: uop_t = @zeroed()\n  u.nope = k\n  o = u\n",
        UOP
    ));
    assert!(text.contains("has no field `nope`"), "{}", text);
    assert!(text.contains("kind, dst, imm"), "{}", text);
}

#[test]
fn a_field_assignment_is_type_checked() {
    let text = compile_err(&format!(
        "{}fun f (x: i8, o: out uop_t)\n  var u: uop_t = @zeroed()\n  u.dst = x\n  o = u\n",
        UOP
    ));
    assert!(text.contains("cannot assign"), "{}", text);
}

#[test]
fn taking_a_field_of_a_non_struct_target_is_rejected() {
    let text = compile_err(concat!(
        "fun f (x: i8, o: out i8)\n",
        "  var v: i8 = @zeroed()\n",
        "  v.nope = x\n",
        "  o = v\n",
    ));
    assert!(text.contains("has no fields"), "{}", text);
}

#[test]
fn zeroed_needs_a_type_it_can_take_from_context() {
    let text = compile_err("fun f (o: out i8)\n  let x = @zeroed()\n  o = x\n");
    assert!(text.contains("needs a type from its context"), "{}", text);
}

#[test]
fn zeroed_takes_the_type_of_an_assignment_target() {
    let v = compile(&format!(
        "{}fun f (o: out uop_t)\n  var u: uop_t = @zeroed()\n  u.dst = 5'd0\n  o = u\n",
        UOP
    ));
    assert!(v.contains("15'd0"), "{}", v);
}

// ---- clocked processes ---------------------------------------------------
//
// A process takes data through pipes and nothing else, so these all share one
// boundary: `src` carries the stimulus and `got` is "an item arrived this
// cycle" -- which is exactly what a bare `go: i1` input used to mean, spelled
// so the compiler owns the protocol. `o` is a `buffer out` because these are
// observations of state, and a stream sink never stalls its producer.

/// The header every register test uses, plus the receive that drives it.
const PROC_IN: &str = concat!(
    "process p (src: buffer in i8, o: buffer out i8)\n",
    "  var c: i8 = @zeroed()\n",
    "  let (x, go) = @try_rcv(src)\n",
);

#[test]
fn a_process_gets_implicit_clock_and_reset() {
    // "A process has channel ports and clock/reset. Nothing else."
    let v = compile(&format!(
        "{}{}",
        PROC_IN,
        concat!("  if go then\n", "    c = c + 8'd1\n", "  let _s = @try_send(o, c)\n")
    ));
    assert!(v.contains("input        clk"), "{}", v);
    assert!(v.contains("input        rst_n"), "{}", v);
    assert!(v.contains("always @(posedge clk)"), "{}", v);
    assert!(v.contains("if (!rst_n) begin"), "sync active-low reset:\n{}", v);
}

#[test]
fn a_var_in_a_process_becomes_a_register() {
    let v = compile(concat!(
        "process p (src: buffer in i8, o: buffer out i8)\n",
        "  var c: i8 = 8'd7\n",
        "  let (x, go) = @try_rcv(src)\n",
        "  if go then\n",
        "    c = c + 8'd1\n",
        "  let _s = @try_send(o, c)\n",
    ));
    assert!(v.contains("reg [7:0] c;"), "{}", v);
    assert!(v.contains("c <= 8'd7;"), "reset value from the declaration:\n{}", v);
}

#[test]
fn an_unassigned_register_keeps_its_value() {
    // This is what makes a conditional assignment a clock enable: the else
    // branch of the mux is the register itself.
    let v = compile(&format!(
        "{}{}",
        PROC_IN,
        concat!("  if go then\n", "    c = c + 8'd1\n", "  let _s = @try_send(o, c)\n")
    ));
    assert!(v.contains("? (c + 8'd1) : c"), "{}", v);
}

/// Reads see the value at the start of the cycle plus whatever the body has
/// already assigned. That is the SystemVerilog `_next` shadow idiom without
/// the shadow, and it means the order of statements decides whether an output
/// carries the current or the next value.
#[test]
fn a_register_read_sees_earlier_assignments_in_the_same_cycle() {
    let before = compile(&format!(
        "{}{}",
        PROC_IN,
        concat!("  let _s = @try_send(o, c)\n", "  if go then\n", "    c = c + 8'd1\n")
    ));
    // Sent first: what leaves is the registered value, untouched.
    assert!(before.contains("? c : o_hold"), "{}", before);

    let after = compile(&format!(
        "{}{}",
        PROC_IN,
        concat!("  if go then\n", "    c = c + 8'd1\n", "  let _s = @try_send(o, c)\n")
    ));
    // Sent after: what leaves is what will be clocked in.
    assert!(!after.contains("? c : o_hold"), "{}", after);
    assert!(after.contains("(c + 8'd1)"), "{}", after);
}

#[test]
fn a_register_can_hold_a_struct_and_be_updated_field_by_field() {
    let v = compile(&format!(
        concat!(
            "{}process p (src: buffer in kind_e, o: buffer out uop_t)\n",
            "  var acc: uop_t = @zeroed()\n",
            "  let (k, go) = @try_rcv(src)\n",
            "  if go then\n",
            "    acc.kind = k\n",
            "  let _s = @try_send(o, acc)\n",
        ),
        UOP
    ));
    assert!(v.contains("reg [14:0] acc;"), "{}", v);
    assert!(v.contains("always @(posedge clk)"), "{}", v);
}

#[test]
fn an_enum_register_resets_to_its_named_variant() {
    let v = compile(concat!(
        "enum st_e: i2\n",
        "  S_IDLE\n",
        "  S_RUN\n",
        "  S_DONE\n",
        "process p (src: buffer in i8, o: buffer out st_e)\n",
        "  var st: st_e = S_RUN\n",
        "  let (x, go) = @try_rcv(src)\n",
        "  if go then\n",
        "    st = S_DONE\n",
        "  let _s = @try_send(o, st)\n",
    ));
    assert!(v.contains("st <= 2'd1;"), "S_RUN is 1:\n{}", v);
}

#[test]
fn a_register_reset_value_must_be_constant() {
    let text = compile_err(concat!(
        "process p (src: buffer in i8, o: buffer out i1)\n",
        "  var c: i1 = clk\n",
        "  let _s = @try_send(o, c)\n",
    ));
    assert!(text.contains("must be a constant"), "{}", text);
}

#[test]
fn a_constant_parameter_can_be_a_reset_value() {
    // The counterpart: a plain parameter is a compile-time constant, so it is
    // exactly what a reset value is allowed to be.
    let v = compile(concat!(
        "process p (seed: i8 = 8'd9, src: buffer in i8, o: buffer out i8)\n",
        "  var c: i8 = seed\n",
        "  let (x, got) = @try_rcv(src)\n",
        "  let _s = @try_send(o, c)\n",
    ));
    assert!(v.contains("c <= 8'd9;"), "{}", v);
    // Named in the header comment so the file is readable, and folded away
    // everywhere else: it is not a port.
    assert!(v.contains("//   seed : i8 = 8'd9"), "{}", v);
    let body = v.split("module p (").nth(1).expect("a module");
    assert!(!body.contains("seed"), "{}", body);
}

#[test]
fn a_register_needs_a_declared_type() {
    let text = compile_err(concat!(
        "process p (src: buffer in i8, o: buffer out i8)\n",
        "  var c = 8'd0\n",
        "  let _s = @try_send(o, c)\n",
    ));
    assert!(text.contains("needs a declared type"), "{}", text);
}

#[test]
fn a_process_needs_at_least_one_pipe() {
    // Constants alone are not a boundary: nothing can observe this.
    let text = compile_err(concat!(
        "process p (a: i8 = 8'd1)\n",
        "  var c: i8 = @zeroed()\n",
        "  c = a\n",
    ));
    assert!(text.contains("at least one pipe"), "{}", text);
}

#[test]
fn a_process_has_no_plain_data_ports() {
    // The rule the language is built on: a process cannot present a raw wire
    // and hand-roll a protocol over it.
    let ins = compile_err(concat!(
        "process p (cp: i16, dst: buffer out i16)\n",
        "  var c: i16 = @zeroed()\n",
        "  c = cp\n",
        "  let _s = @try_send(dst, c)\n",
    ));
    assert!(ins.contains("has no value"), "{}", ins);
    assert!(ins.contains("buffer in"), "{}", ins);

    let outs = compile_err(concat!(
        "process p (src: buffer in i16, o: out i16)\n",
        "  let (x, got) = @try_rcv(src)\n",
        "  o = x\n",
    ));
    assert!(outs.contains("no plain outputs"), "{}", outs);
}

#[test]
fn clk_and_rst_n_cannot_be_declared_by_hand() {
    let text = compile_err(concat!(
        "process p (clk: i1, o: buffer out i8)\n",
        "  var c: i8 = @zeroed()\n",
        "  let _s = @try_send(o, c)\n",
    ));
    assert!(text.contains("implicit on a process"), "{}", text);
}

/// Dedenting two levels at once after a nested bare `if` used to be a hard
/// parse error blamed on the enclosing declaration, because the `else` probe
/// treated "next token is shallower" as a failure instead of "there is no
/// else".
#[test]
fn a_two_level_dedent_after_a_nested_if_parses() {
    let v = compile(concat!(
        "process p (src: buffer in i8, o: buffer out i8)\n",
        "  var c: i8 = @zeroed()\n",
        "  let (x, got) = @try_rcv(src)\n",
        "  let clear: i1 = x[0]\n",
        "  let go: i1 = x[1]\n",
        "  if clear then\n",
        "    c = @zeroed()\n",
        "  else\n",
        "    if go then\n",
        "      c = c + 8'd1\n",
        "  let _s = @try_send(o, c)\n",
    ));
    assert!(v.contains("clear ?"), "{}", v);
    assert!(v.contains("go ?"), "{}", v);
}

// ---- channels ------------------------------------------------------------

const PIPE: &str = concat!(
    "struct item_t\n",
    "  tag: i4\n",
    "  payload: i16\n",
);

#[test]
fn a_pipe_becomes_a_valid_ready_data_triple() {
    // The flattening k3g_chan.sv:60 pre-commits to for the yosys-slang risk.
    let v = compile(&format!(
        concat!(
            "{}process p (src: buffer in item_t, dst: buffer out item_t)\n",
            "  loop\n",
            "    let (it, got) = @try_rcv(src)\n",
            "    let _ok = @try_send(dst, it)\n",
        ),
        PIPE
    ));
    assert!(v.contains("input         src_valid"), "{}", v);
    assert!(v.contains("output        src_ready"), "{}", v);
    assert!(v.contains("input  [19:0] src_data"), "{}", v);
    assert!(v.contains("output        dst_valid"), "{}", v);
    assert!(v.contains("input         dst_ready"), "{}", v);
    assert!(v.contains("output [19:0] dst_data"), "{}", v);
}

/// Channel rule 3: `valid` must not depend combinationally on `ready`. It
/// cannot here -- an output's `valid` IS the busy register and nothing else is
/// allowed to drive it. k2g_chan.sv records the bug this prevents: routing a
/// stall into `cp_valid` closed a loop through stall -> decode -> CSP request
/// -> stall.
#[test]
fn an_output_valid_is_a_register_output() {
    let v = compile(&format!(
        concat!(
            "{}process p (src: buffer in item_t, dst: buffer out item_t)\n",
            "  loop\n",
            "    let (it, got) = @try_rcv(src)\n",
            "    let _ok = @try_send(dst, it)\n",
        ),
        PIPE
    ));
    assert!(v.contains("reg dst_busy;"), "{}", v);
    assert!(v.contains("assign dst_valid = dst_busy;"), "valid is the register:\n{}", v);
    // And `ready` is a register output too, which is the reason for the second
    // entry: with one, the only honest answer was "I am empty, or my consumer
    // is taking it this cycle", and that put the consumer's `ready` on a wire
    // straight through to the producer's.
    assert!(v.contains("wire dst_room = !dst_skid_busy;"), "{}", v);
    assert!(v.contains("assign src_ready = dst_room;"), "{}", v);
    assert!(!v.contains("| dst_ready"), "ready must not reach ready:\n{}", v);
}

#[test]
fn a_buffer_is_two_deep() {
    // desc.md:105 calls a pipe a fifo and desc.md:109 says the producer stalls
    // "when no slots available" -- plural. One entry is not that, and it is
    // also what forced `ready` to be combinational.
    let v = compile(&format!(
        concat!(
            "{}process p (src: buffer in item_t, dst: buffer out item_t)\n",
            "  loop\n",
            "    let (it, got) = @try_rcv(src)\n",
            "    let _ok = @try_send(dst, it)\n",
        ),
        PIPE
    ));
    assert!(v.contains("reg dst_busy;"), "head:\n{}", v);
    assert!(v.contains("reg dst_skid_busy;"), "skid:\n{}", v);
    assert!(v.contains("reg [19:0] dst_hold;"), "{}", v);
    assert!(v.contains("reg [19:0] dst_skid;"), "{}", v);
    // The skid drains into the head, never straight out.
    assert!(v.contains("dst_hold <= "), "{}", v);
    assert!(v.contains("dst_skid"), "{}", v);
}

#[test]
fn every_output_pipe_is_two_deep() {
    // There is one kind of pipe now, so there is one depth: head and skid.
    // `stream` used to be the exception that stayed one deep, and the reason
    // it is gone is that the exception was the lossy one.
    let v = compile(concat!(
        "process p (src: buffer in i32, o: buffer out i32)\n",
        "  let (x, got) = @try_rcv(src)\n",
        "  let _s = @try_send(o, x)\n",
    ));
    assert!(v.contains("reg o_busy;"), "{}", v);
    assert!(v.contains("reg o_skid_busy;"), "{}", v);
}

#[test]
fn a_slot_holds_until_it_drains() {
    let v = compile(&format!(
        concat!(
            "{}process p (src: buffer in item_t, dst: buffer out item_t)\n",
            "  loop\n",
            "    let (it, got) = @try_rcv(src)\n",
            "    let _ok = @try_send(dst, it)\n",
        ),
        PIPE
    ));
    assert!(v.contains("always @(posedge clk)"), "{}", v);
    // The head keeps its item unless the consumer takes it; when it does, the
    // skid moves down rather than the item being lost.
    assert!(v.contains("wire dst_pop = dst_busy & dst_ready;"), "{}", v);
    assert!(v.contains("dst_busy <= "), "{}", v);
    assert!(v.contains("dst_hold <= "), "{}", v);
    // An offer can only land while the skid is free, so "push into the skid as
    // the skid drains into the head" is unreachable by construction.
    assert!(v.contains("dst_skid_busy <= "), "{}", v);
}

#[test]
fn receiving_from_an_output_pipe_is_rejected() {
    let text = compile_err(&format!(
        "{}process p (dst: buffer out item_t)\n  let (it, got) = @try_rcv(dst)\n  let _o = @try_send(dst, it)\n",
        PIPE
    ));
    assert!(text.contains("cannot be received from"), "{}", text);
}

#[test]
fn sending_to_an_input_pipe_is_rejected() {
    let text = compile_err(&format!(
        "{}process p (src: buffer in item_t)\n  let (it, got) = @try_rcv(src)\n  let _o = @try_send(src, it)\n",
        PIPE
    ));
    assert!(text.contains("cannot be sent to"), "{}", text);
}

#[test]
fn an_output_pipe_that_is_never_sent_to_is_rejected() {
    let text = compile_err(&format!(
        "{}process p (src: buffer in item_t, dst: buffer out item_t)\n  let (it, got) = @try_rcv(src)\n",
        PIPE
    ));
    assert!(text.contains("is never sent to"), "{}", text);
}

#[test]
fn sending_twice_in_one_cycle_is_rejected() {
    let text = compile_err(&format!(
        concat!(
            "{}process p (src: buffer in item_t, dst: buffer out item_t)\n",
            "  let (it, got) = @try_rcv(src)\n",
            "  let _a = @try_send(dst, it)\n",
            "  let _b = @try_send(dst, it)\n",
        ),
        PIPE
    ));
    assert!(text.contains("sent to more than once"), "{}", text);
}

#[test]
fn a_pipe_payload_is_type_checked() {
    let text = compile_err(&format!(
        concat!(
            "{}process p (src: buffer in item_t, dst: buffer out item_t)\n",
            "  let (it, got) = @try_rcv(src)\n",
            "  let _o = @try_send(dst, it.tag)\n",
        ),
        PIPE
    ));
    assert!(text.contains("carries `item_t`"), "{}", text);
}

#[test]
fn only_try_rcv_produces_a_pair() {
    let text = compile_err("fun f (a: i8, o: out i8)\n  let (x, y) = a\n  o = x\n");
    assert!(text.contains("only `@try_rcv(p)` produces a pair"), "{}", text);
}

// ---- blocking channel operations -----------------------------------------

const ADDER: &str = concat!(
    "process p (src: buffer in i32, dst: buffer out i32)\n",
    "  loop\n",
    "    let a = @rcv(src)\n",
    "    let b = @rcv(src)\n",
    "    @send(dst, a + b)\n",
);

#[test]
fn blocking_ops_become_one_state_each() {
    let v = compile(ADDER);
    assert!(v.contains("reg [1:0] state;"), "three states need two bits:\n{}", v);
    assert!(v.contains("in_s0"), "{}", v);
    assert!(v.contains("in_s1"), "{}", v);
    assert!(v.contains("in_s2"), "{}", v);
}

/// Rule 3 again, now for the FSM form: in a send state `valid` is `state == i`
/// and state is a register, so it still cannot depend on `ready`.
#[test]
fn a_send_states_valid_is_a_function_of_state() {
    let v = compile(ADDER);
    assert!(v.contains("assign dst_valid = in_s2;"), "{}", v);
    assert!(v.contains("assign src_ready = (in_s0 | in_s1);"), "{}", v);
}

#[test]
fn a_value_crossing_a_state_becomes_a_register() {
    // `a` is received in state 0 and read in state 2, so it cannot be a wire.
    let v = compile(ADDER);
    assert!(v.contains("reg [31:0] a_r;"), "{}", v);
    assert!(v.contains("a_r <= (fire_s0 ? src_data : a_r);"), "{}", v);
    assert!(v.contains("dst_data = (a_r + b_r)"), "{}", v);
}

#[test]
fn a_barrier_inside_a_conditional_gets_its_own_state() {
    let v = compile(concat!(
        "process p (go: i1 = 1'b1, src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    if go then\n",
        "      let a = @rcv(src)\n",
        "    @send(dst, 32'd0)\n",
    ));
    // Three states: the branch, the guarded receive, and the send.
    assert!(v.contains("in_s0"), "{}", v);
    assert!(v.contains("in_s2"), "{}", v);
    assert!(v.contains("assign src_ready ="), "{}", v);
}

#[test]
fn a_loop_with_no_blocking_operation_repeats_every_cycle() {
    // Not an error and not a state machine: `loop` with no barrier is the
    // per-cycle form, which is what most of this compiler's output is.
    let v = compile(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let (x, got) = @try_rcv(src)\n",
        "    @try_send(dst, x)\n",
    ));
    assert!(v.contains("always @(posedge clk)"), "{}", v);
    // No `state`, and no `done`: it never stops.
    assert!(!v.contains("reg [1:0] state;"), "{}", v);
    assert!(!v.contains("reg done;"), "{}", v);
}

#[test]
fn a_linear_body_runs_once_and_stops() {
    // desc.md:37 -- a process "may stop (reach terminal state)". `loop` is
    // what makes a body repeat; without one it is a program that runs once.
    let v = compile(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  var seen: i32 = @zeroed()\n",
        "  let (x, got) = @try_rcv(src)\n",
        "  seen = x\n",
        "  @try_send(dst, seen)\n",
    ));
    assert!(v.contains("reg done;"), "{}", v);
    assert!(v.contains("done <= 1'b1;"), "{}", v);
    // Refuses everything once it has stopped, and holds its state.
    assert!(v.contains("(!done)"), "{}", v);
    assert!(v.contains("seen <= (done ? seen :"), "{}", v);
}

#[test]
fn a_linear_body_with_barriers_ends_in_a_terminal_state() {
    let v = compile(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  let a = @rcv(src)\n",
        "  @send(dst, a)\n",
    ));
    // Two segments, so states 0 and 1 -- plus a third the machine parks in.
    // Nothing drives a `valid` or a `ready` there, so it is terminal by
    // construction rather than by a rule written out somewhere.
    assert!(v.contains("reg [1:0] state;"), "{}", v);
    assert!(v.contains("2'd2"), "{}", v);
    assert!(v.contains("assign src_ready = in_s0;"), "{}", v);
    assert!(v.contains("assign dst_valid = in_s1;"), "{}", v);
}

#[test]
fn break_in_a_per_cycle_loop_has_nothing_to_leave() {
    // A `loop` with no blocking operation is the per-cycle form: it has no
    // states, so there is no state machine to leave.
    let text = compile_err(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let (x, got) = @try_rcv(src)\n",
        "    @try_send(dst, x)\n",
        "    break\n",
    ));
    assert!(text.contains("nothing here to `break` out of"), "{}", text);
}

#[test]
fn break_leaves_a_blocking_loop_for_its_terminal_state() {
    let v = compile(concat!(
        "process until_zero (src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    if a == 32'd0 then\n",
        "      break\n",
        "    @send(dst, a)\n",
    ));
    // Two states of the program plus the terminal one it breaks to. Nothing
    // drives a handshake there, so the machine parks.
    assert!(v.contains("branch_s0 ? 2'd2 : 2'd1"), "{}", v);
    assert!(!v.contains("in_s2"), "{}", v);
}

#[test]
fn statements_after_the_last_barrier_run_when_it_fires() {
    // They used to be an error ("the last statement must be an `@rcv` or
    // `@send`"). They are the send state's post scope now: the same place a
    // statement between a receive and a branch goes, and the same cycle.
    let v = compile(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  var n: i32 = @zeroed()\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    @send(dst, a)\n",
        "    n = n + 32'd1\n",
    ));
    // Two states, and the counter advances only when the send completes.
    assert!(v.contains("n <= (fire_s1 ? (n + 32'd1) : n);"), "{}", v);
}

#[test]
fn receiving_from_an_output_pipe_is_rejected_in_a_loop() {
    let text = compile_err(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  loop\n",
        "    let a = @rcv(dst)\n",
        "    @send(dst, a)\n",
    ));
    assert!(text.contains("can only be sent to"), "{}", text);
}

// ---- sequences -----------------------------------------------------------

const PIPE3: &str = concat!(
    "sequence s (src: buffer in i16, dst: buffer out i32)\n",
    "  let a = @rcv(src)\n",
    "  let doubled: i16 = a + a\n",
    "  |||\n",
    "  let wide: i32 = @zext(doubled, 32)\n",
    "  |||\n",
    "  let scaled: i32 = wide + wide\n",
    "  @send(dst, scaled)\n",
);

#[test]
fn a_stage_cut_becomes_a_register_bank_and_a_validity_bit() {
    // `|||` was parsed and thrown away since before this work started.
    let v = compile(PIPE3);
    assert!(v.contains("reg v0;"), "{}", v);
    assert!(v.contains("reg v1;"), "{}", v);
    assert!(v.contains("reg v2;"), "{}", v);
    assert!(v.contains("reg [15:0] doubled_s1;"), "{}", v);
    assert!(v.contains("reg [31:0] wide_s2;"), "{}", v);
}

#[test]
fn the_pipeline_shifts_when_its_sink_has_a_slot() {
    let v = compile(PIPE3);
    // The slot is two entries deep, so the room it reports is the skid being
    // empty rather than the sink taking something this cycle.
    assert!(v.contains("wire shift = !out_skid_busy;"), "{}", v);
    assert!(v.contains("assign src_ready = shift;"), "{}", v);
    // Rule 3: the output valid is the last validity bit, a register.
    assert!(v.contains("assign dst_valid = v2;"), "{}", v);
    assert!(v.contains("v1 <= (shift ? v0 : v1);"), "{}", v);
}

#[test]
fn the_producer_side_ready_is_not_a_wire_to_the_sink() {
    // What the second entry is for. With one entry `shift` was
    // `(!v2) | dst_ready` and `src_ready` was `shift`, so `scaler` in
    // examples/pipeline_graph.ddl put one combinational path through three
    // modules -- rule 3 satisfied and the path there anyway.
    //
    // Anti-vacuous: `dst_ready` does appear in this module, in the head's own
    // drain condition, so a test that merely grepped for it would pass on the
    // one-entry version too.
    let v = compile(PIPE3);
    assert!(v.contains("dst_ready"), "{}", v);

    let ready_line = v
        .lines()
        .find(|l| l.contains("assign src_ready"))
        .expect("a buffer input has a ready");
    let feeds_ready: Vec<&str> = v
        .lines()
        .filter(|l| l.trim_start().starts_with("wire shift ="))
        .collect();
    assert!(!ready_line.contains("dst_ready"), "{}", v);
    for l in &feeds_ready {
        assert!(!l.contains("dst_ready"), "{}", v);
    }
}

#[test]
fn a_stream_qualifier_is_refused_with_a_note_on_what_it_cost() {
    // Recognised rather than deleted from the lexer: `dst: stream out i32`
    // failing as an unrecognised type would blame the wrong word.
    let text = compile_err(concat!(
        "sequence s (src: buffer in i16, dst: stream out i32)\n",
        "  let a = @rcv(src)\n",
        "  @send(dst, @zext(a, 32))\n",
    ));
    assert!(text.contains("`stream out` is not a pipe kind; DDL has `buffer`"), "{}", text);
    assert!(text.contains("A `buffer` holds two and stalls instead"), "{}", text);
}

#[test]
fn the_item_leaving_is_registered_alongside_its_validity_bit() {
    // Without this the pipeline would offer the CURRENT input while
    // advertising the validity of one three cycles older.
    let v = compile(PIPE3);
    assert!(v.contains("reg [31:0] out_hold;"), "{}", v);
    assert!(v.contains("assign dst_data = out_hold;"), "{}", v);
}

#[test]
fn a_blocking_read_outside_the_head_stage_is_rejected() {
    let text = compile_err(concat!(
        "sequence s (src: buffer in i16, dst: buffer out i32)\n",
        "  let a = @rcv(src)\n",
        "  |||\n",
        "  let b = @rcv(src)\n",
        "  @send(dst, @zext(b, 32))\n",
    ));
    assert!(text.contains("only the first stage"), "{}", text);
}

#[test]
fn a_sequence_must_end_by_sending() {
    let text = compile_err(concat!(
        "sequence s (src: buffer in i16, dst: buffer out i32)\n",
        "  let a = @rcv(src)\n",
        "  |||\n",
        "  let b: i32 = @zext(a, 32)\n",
    ));
    assert!(text.contains("ends by sending"), "{}", text);
}


// ---- M5: memories, assertions, and the emit modes ------------------------

const CMD: &str = concat!(
    "struct cmd_t\n",
    "  we: i1\n",
    "  addr: i5\n",
    "  data: i32\n",
);

const REGFILE: &str = concat!(
    "struct cmd_t\n",
    "  we: i1\n",
    "  addr: i5\n",
    "  data: i32\n",
    "process rf (cmd: buffer in cmd_t, rd: buffer out i32)\n",
    "  var vals: #[impl(lutram)] [i32; 32] = @zeroed()\n",
    "  loop\n",
    "    let (c, got) = @try_rcv(cmd)\n",
    "    let _s = @try_send(rd, vals[c.addr])\n",
    "    if c.we then\n",
    "      vals[c.addr] = c.data\n",
);

#[test]
fn a_memory_becomes_an_unpacked_array_with_one_write_port() {
    let v = compile(REGFILE);
    assert!(v.contains("reg [31:0] vals [0:31];"), "{}", v);
    // ONE write, not one per source assignment. Two write ports infer no RAM
    // at all on this device -- the K2G value array collapsed to ~3700 LUTs
    // when a second was added.
    assert_eq!(v.matches("vals[").count() - v.matches("vals[vals_ix]").count(), 2, "{}", v);
    assert!(v.contains("wire [31:0] "), "{}", v);
}

#[test]
fn a_memory_read_is_asynchronous() {
    let v = compile(REGFILE);
    // The read is a continuous assignment, not something clocked: that is the
    // shape SSRAM is inferred from, and it is what lets one cycle do
    // read -> forward -> add.
    assert!(v.contains("= vals[cmd_data[36:32]];"), "{}", v);
}

#[test]
fn a_conditional_write_becomes_a_write_enable() {
    let v = compile(REGFILE);
    assert!(v.contains("end else if (cmd_data[37]) begin"), "{}", v);
    assert!(v.contains("vals[cmd_data[36:32]] <= cmd_data[31:0];"), "{}", v);
    // The address and the data must NOT carry the enable's mux as well: the
    // write does not happen when the enable is low, so muxing them is pure
    // area. The SSA join produces those muxes and they are dropped again.
    assert!(!v.contains("? cmd_data[36:32] :"), "{}", v);
}

#[test]
fn an_initialised_memory_gets_a_reset_loop() {
    let v = compile(REGFILE);
    assert!(
        v.contains("for (vals_ix = 0; vals_ix < 32; vals_ix = vals_ix + 1) vals[vals_ix] <= 32'd0;"),
        "{}",
        v
    );
    assert!(v.contains("integer vals_ix;"), "{}", v);
}

#[test]
fn a_memory_with_no_initialiser_has_no_reset_loop() {
    // Not a default: the reset loop costs about 85 LUTs and some extra RAM
    // primitives on the K2G value array, so the source decides.
    let v = compile(&format!(
        "{}{}",
        CMD,
        concat!(
            "process rf (cmd: buffer in cmd_t, rd: buffer out i32)\n",
            "  var vals: #[impl(lutram)] [i32; 32]\n",
            "  loop\n",
            "    let (c, got) = @try_rcv(cmd)\n",
            "    let _s = @try_send(rd, vals[c.addr])\n",
            "    if c.we then\n",
            "      vals[c.addr] = c.data\n",
        )
    ));
    assert!(!v.contains("integer vals_ix;"), "{}", v);
    assert!(!v.contains("for ("), "{}", v);
    assert!(v.contains("if (cmd_data[37]) begin"), "{}", v);
}

#[test]
fn the_element_type_can_be_an_enum() {
    let v = compile(&format!("{}{}", OPS, concat!(
        "process rf (addr: buffer in i5, rd: buffer out op_e)\n",
        "  var tags: #[impl(lutram)] [op_e; 32] = OP_SUB\n",
        "  let (a, got) = @try_rcv(addr)\n",
        "  let _s = @try_send(rd, tags[a])\n",
    )));
    assert!(v.contains("reg [1:0] tags [0:31];"), "{}", v);
    assert!(v.contains("<= 2'd1;"), "{}", v);
}

#[test]
fn a_narrow_index_is_widened_and_a_wide_one_is_refused() {
    let v = compile(concat!(
        "process rf (addr: buffer in i3, rd: buffer out i32)\n",
        "  var vals: #[impl(lutram)] [i32; 32] = @zeroed()\n",
        "  let (a, got) = @try_rcv(addr)\n",
        "  let _s = @try_send(rd, vals[a])\n",
    ));
    assert!(v.contains("vals["), "{}", v);

    let text = compile_err(concat!(
        "process rf (addr: buffer in i8, rd: buffer out i32)\n",
        "  var vals: #[impl(lutram)] [i32; 32] = @zeroed()\n",
        "  let (a, got) = @try_rcv(addr)\n",
        "  let _s = @try_send(rd, vals[a])\n",
    ));
    assert!(text.contains("addressed by 5"), "{}", text);
    assert!(text.contains("@trunc"), "{}", text);
}

#[test]
fn an_unknown_impl_is_rejected() {
    let text = compile_err(concat!(
        "process rf (addr: buffer in i5, rd: buffer out i32)\n",
        "  var vals: #[impl(sram)] [i32; 32] = @zeroed()\n",
        "  let (a, got) = @try_rcv(addr)\n",
        "  let _s = @try_send(rd, vals[a])\n",
    ));
    assert!(text.contains("lutram"), "{}", text);
}

#[test]
fn a_memory_cannot_be_a_parameter() {
    let text = compile_err(concat!(
        "process rf (vals: #[impl(lutram)] [i32; 32], rd: buffer out i32)\n",
        "  var acc: i32 = 0\n",
        "  let _s = @try_send(rd, acc)\n",
    ));
    assert!(text.contains("cannot be a parameter"), "{}", text);
}

#[test]
fn a_memory_needs_a_constant_reset() {
    let text = compile_err(concat!(
        "process rf (addr: buffer in i5, rd: buffer out i1)\n",
        "  var vals: #[impl(lutram)] [i1; 32] = clk\n",
        "  let (a, got) = @try_rcv(addr)\n",
        "  let _s = @try_send(rd, vals[a])\n",
    ));
    assert!(text.contains("same constant"), "{}", text);
}

#[test]
fn a_memory_in_a_blocking_process_writes_only_in_the_state_that_writes_it() {
    // The accesses are scheduled into the states now. A write is gated by the
    // firing of the state that performs it, which is what makes a scratchpad
    // in a blocking process mean what it reads as: written once per item, not
    // once per clock.
    let v = compile(concat!(
        "process p (src: buffer in i32, dst: buffer out i32)\n",
        "  var vals: #[impl(lutram)] [i32; 32] = @zeroed()\n",
        "  loop\n",
        "    let a = @rcv(src)\n",
        "    vals[5'd0] = a\n",
        "    @send(dst, vals[5'd1])\n",
    ));
    assert!(v.contains("reg [31:0] vals [0:31];"), "{}", v);
    assert!(v.contains("end else if (fire_s0) begin"), "{}", v);
    assert!(v.contains("vals[5'd0] <= a_r;") || v.contains("vals[5'd0] <= src_data;"), "{}", v);
}

#[test]
fn a_memory_is_not_a_value() {
    let text = compile_err(concat!(
        "fun f (x: i32, y: out i32)\n",
        "  let vals: #[impl(lutram)] [i32; 32] = @zeroed()\n",
        "  y = x\n",
    ));
    assert!(text.contains("state rather than a value"), "{}", text);
}

#[test]
fn an_assertion_is_guarded_on_simulation() {
    let v = compile(concat!(
        "fun f (x: i8, y: out i8)\n",
        "  @assert(x != 8'd0, \"x must not be zero\")\n",
        "  y = x\n",
    ));
    // GowinSynthesis does not define SYNTHESIS, so SIMULATION is the guard
    // that works on this toolchain.
    assert!(v.contains("`ifdef SIMULATION"), "{}", v);
    assert!(v.contains("`endif"), "{}", v);
    assert!(v.contains("$error(\"%m: x must not be zero\")"), "{}", v);
}

#[test]
fn a_clocked_assertion_runs_on_the_edge_and_not_during_reset() {
    let v = compile(concat!(
        "process p (src: buffer in i8, o: buffer out i8)\n",
        "  var n: i8 = 0\n",
        "  let (x, got) = @try_rcv(src)\n",
        "  @assert(x != 8'd0, \"x must not be zero\")\n",
        "  n = x\n",
        "  let _s = @try_send(o, n)\n",
    ));
    assert!(v.contains("always @(posedge clk) begin"), "{}", v);
    // Registers hold their reset value during reset, so an assertion about
    // what the design computes has nothing to say then.
    assert!(v.contains("if (rst_n) begin"), "{}", v);
}

#[test]
fn an_assertion_inside_an_if_is_an_implication() {
    let v = compile(concat!(
        "fun f (c: i1, x: i8, y: out i8)\n",
        "  if c then\n",
        "    @assert(x != 8'd0, \"x must not be zero when c\")\n",
        "    y = x\n",
        "  else\n",
        "    y = 8'd1\n",
    ));
    // Written as a plain condition it would fire on the other branch too.
    assert!(v.contains("(!c)"), "{}", v);
    assert!(v.contains("| (x != 8'd0)"), "{}", v);
}

#[test]
fn an_assertion_in_a_match_arm_is_guarded_by_that_arm() {
    let v = compile(&format!("{}{}", OPS, concat!(
        "fun f (op: op_e, x: i8, y: out i8)\n",
        "  match op\n",
        "    .OP_ADD =>\n",
        "      @assert(x != 8'd0, \"no zero on add\")\n",
        "      y = x\n",
        "    _ =>\n",
        "      y = 8'd0\n",
    )));
    assert!(v.contains("$error(\"%m: no zero on add\")"), "{}", v);
    // The guard is the arm's own label test, so the assertion says nothing
    // about the other opcodes.
    assert!(v.contains("2'd0"), "{}", v);
}

#[test]
fn fatal_uses_the_fatal_task() {
    let v = compile(concat!(
        "fun f (x: i8, y: out i8)\n",
        "  @fatal(x != 8'd0, \"x must not be zero\")\n",
        "  y = x\n",
    ));
    assert!(v.contains("$fatal(1, \"%m: x must not be zero\")"), "{}", v);
}

#[test]
fn an_assertion_message_is_escaped() {
    // A bare `%` would be read by `$error` as a format specifier and would
    // consume an argument that is not there.
    let v = compile(concat!(
        "fun f (x: i8, y: out i8)\n",
        "  @assert(x != 8'd0, \"100% of the time\")\n",
        "  y = x\n",
    ));
    assert!(v.contains("100%% of the time"), "{}", v);
}

#[test]
fn an_assertion_condition_must_be_i1() {
    let text = compile_err(concat!(
        "fun f (x: i8, y: out i8)\n",
        "  @assert(x, \"nope\")\n",
        "  y = x\n",
    ));
    assert!(text.contains("`i1` condition"), "{}", text);
}

#[test]
fn an_assertion_message_must_be_a_literal() {
    let text = compile_err(concat!(
        "fun f (x: i8, y: out i8)\n",
        "  @assert(x != 8'd0, x)\n",
        "  y = x\n",
    ));
    assert!(text.contains("string literal"), "{}", text);
}

#[test]
fn an_assertion_keeps_its_cone_alive() {
    // Nothing downstream reads an assertion, so without it being a root the
    // whole cone feeding it would look dead and be stripped.
    let v = compile(concat!(
        "fun f (a: i8, b: i8, y: out i8)\n",
        "  let sum: i8 = a + b\n",
        "  @assert(sum != 8'd0, \"sum must not be zero\")\n",
        "  y = a\n",
    ));
    assert!(v.contains("sum"), "{}", v);
}

#[test]
fn emit_ir_shows_what_the_compiler_decided() {
    use crate::driver::{Emit, compile as compile_with};
    let map = SourceMap::new("t.ddl", REGFILE);
    let ir = compile_with(&map, &EmitOptions::default(), Emit::Ir)
        .expect("compiles");
    assert!(ir.contains("mem   vals : [i32; 32] lutram"), "{}", ir);
    assert!(ir.contains("memread vals["), "{}", ir);
    // No banner: this is for reading, not for checking in.
    assert!(!ir.contains("GENERATED FILE"), "{}", ir);
}

#[test]
fn emit_ast_prints_names_rather_than_pointers() {
    use crate::driver::{Emit, compile as compile_with};
    let map = SourceMap::new("t.ddl", REGFILE);
    let ast = compile_with(&map, &EmitOptions::default(), Emit::Ast)
        .expect("parses");
    assert!(ast.contains("`rf`"), "{}", ast);
    assert!(!ast.contains("byte_ptr"), "{}", ast);
}


#[test]
fn a_bram_in_a_process_with_no_states_has_nowhere_to_put_its_cycle() {
    // Accepting the annotation and emitting an asynchronous read would hand
    // back distributed RAM under a `bram` label. A per-cycle process has no
    // state to spend, so the answer is `lutram` or a process that blocks.
    let text = compile_err(concat!(
        "process rf (addr: buffer in i5, rd: buffer out i32)\n",
        "  var vals: #[impl(bram)] [i32; 32]\n",
        "  let (a, got) = @try_rcv(addr)\n",
        "  let _s = @try_send(rd, vals[a])\n",
    ));
    assert!(text.contains("no state to put it in"), "{}", text);
    assert!(text.contains("use `lutram`"), "{}", text);
}

#[test]
fn a_bram_cannot_be_reset() {
    // The reset a `lutram` gets is a loop over every element. A block RAM
    // written on reset cannot be inferred as one: 256x32 would come out as
    // 8192 flip-flops, which is a silent disaster rather than a loud one.
    let text = compile_err(concat!(
        "process p (req: buffer in i8, resp: buffer out i32)\n",
        "  var table: #[impl(bram)] [i32; 256] = @zeroed()\n",
        "  loop\n",
        "    let a = @rcv(req)\n",
        "    let v = table[a]\n",
        "    @send(resp, v)\n",
    ));
    assert!(text.contains("cannot be reset"), "{}", text);
    assert!(text.contains("flip-flop per bit"), "{}", text);
}


#[test]
fn the_same_source_compiles_to_the_same_bytes() {
    // The SSA joins walk `env.keys()`, so the environment's iteration order
    // decides the order values are emitted in. With a `HashMap` that order is
    // randomised per process and three builds of one source produced three
    // different files -- which makes `--check` impossible to pass and every
    // regeneration a diff. Ten builds here, because a random order can agree
    // with itself twice by luck.
    let first = compile(REGFILE);
    for _ in 0..9 {
        assert_eq!(compile(REGFILE), first, "output is not reproducible");
    }
}


// ---- M7: `let` is a constant, `var` is a variable -------------------------
//
// Until this, `is_mutable` was read in exactly one place in the compiler -- the
// scan that picks registers out of a process body -- so the distinction lived
// only in the reader's head. The first run of the check found a real one:
// `k2g_shift.ddl` declared `let span_mask` and then assigned it on both
// branches of an `if`, which made its initialiser dead code.

#[test]
fn a_let_cannot_be_assigned() {
    let text = compile_err(concat!(
        "fun f (x: i8, y: out i8)\n",
        "  let acc: i8 = 8'd0\n",
        "  acc = x\n",
        "  y = acc\n",
    ));
    assert!(text.contains("is a `let` binding"), "{}", text);
    assert!(text.contains("declare it `var`"), "{}", text);
}

#[test]
fn a_var_can_be_assigned() {
    let v = compile(concat!(
        "fun f (x: i8, y: out i8)\n",
        "  var acc: i8 = 8'd0\n",
        "  acc = x\n",
        "  y = acc\n",
    ));
    assert!(v.contains("assign y = x;"), "{}", v);
}

#[test]
fn an_out_parameter_is_assignable_without_being_a_var() {
    // `out` is written once and read back by the caller, which is a different
    // thing from state that changes.
    let v = compile("fun f (x: i8, y: out i8)\n  y = x\n");
    assert!(v.contains("assign y = x;"), "{}", v);
}

#[test]
fn a_constant_parameter_cannot_be_assigned() {
    let text = compile_err(concat!(
        "process p (n: i8 = 8'd3, src: buffer in i8, o: buffer out i8)\n",
        "  loop\n",
        "    let (x, got) = @try_rcv(src)\n",
        "    n = x\n",
        "    @try_send(o, n)\n",
    ));
    assert!(text.contains("is a constant parameter"), "{}", text);
    assert!(text.contains("buffer in"), "{}", text);
}

#[test]
fn a_received_item_cannot_be_assigned() {
    let text = compile_err(concat!(
        "process p (src: buffer in i8, o: buffer out i8)\n",
        "  loop\n",
        "    let (x, got) = @try_rcv(src)\n",
        "    x = 8'd0\n",
        "    @try_send(o, x)\n",
    ));
    assert!(text.contains("is a `let` binding"), "{}", text);
}

#[test]
fn a_register_is_still_mutable() {
    // The leading `var` scan and the assignability check have to agree.
    let v = compile(concat!(
        "process p (src: buffer in i8, o: buffer out i8)\n",
        "  var c: i8 = 8'd0\n",
        "  loop\n",
        "    let (x, got) = @try_rcv(src)\n",
        "    c = c + x\n",
        "    @try_send(o, c)\n",
    ));
    assert!(v.contains("reg [7:0] c;"), "{}", v);
    assert!(v.contains("c <= "), "{}", v);
}

#[test]
fn a_let_keeps_its_name_where_a_mux_join_would_not() {
    // What the k2g_shift repair bought: a conditional value written as one
    // `let` is a NAMED wire, where the same thing written as an `if` over a
    // pre-initialised binding became an anonymous temporary.
    let v = compile(concat!(
        "fun f (c: i1, y: out i32)\n",
        "  let picked: i32 = if c then 32'd1 else 32'd2\n",
        "  y = picked\n",
    ));
    assert!(v.contains("wire [31:0] picked ="), "{}", v);
}


// ---- M7b: a call can produce more than one value --------------------------

const DIVMOD: &str = concat!(
    "fun divmod (a: i8, b: i8, q: out i8, r: out i8)\n",
    "  q = a / b\n",
    "  r = a % b\n",
);

#[test]
fn a_tuple_binding_takes_one_name_per_output() {
    let v = compile(&format!("{}{}", DIVMOD, concat!(
        "fun f (x: i8, y: i8, s: out i8)\n",
        "  let (quot, rem) = divmod(x, y)\n",
        "  s = quot + rem\n",
    )));
    // The CALLER's names reach the Verilog, not the callee's parameter names.
    assert!(v.contains("wire [7:0] quot = x / y;"), "{}", v);
    assert!(v.contains("wire [7:0] rem = x % y;"), "{}", v);
    assert!(v.contains("assign s = (quot + rem);"), "{}", v);
}

#[test]
fn declaration_order_decides_which_name_gets_which() {
    let v = compile(&format!("{}{}", DIVMOD, concat!(
        "fun f (x: i8, y: i8, s: out i8)\n",
        "  let (first, second) = divmod(x, y)\n",
        "  s = first\n",
    )));
    // `q` is declared first, so `first` is the quotient.
    assert!(v.contains("assign s = x / y;") || v.contains("wire [7:0] first = x / y;"), "{}", v);
}

#[test]
fn binding_the_wrong_number_of_names_says_what_the_outputs_are() {
    let text = compile_err(&format!("{}{}", DIVMOD, concat!(
        "fun f (x: i8, y: i8, s: out i8)\n",
        "  let (a, b, c) = divmod(x, y)\n",
        "  s = a\n",
    )));
    assert!(text.contains("has 2 outputs but 3 name"), "{}", text);
    assert!(text.contains("q, r"), "{}", text);
}

#[test]
fn a_multi_output_function_is_still_refused_in_expression_position() {
    let text = compile_err(&format!("{}{}", DIVMOD, concat!(
        "fun f (x: i8, y: i8, s: out i8)\n",
        "  s = divmod(x, y)\n",
    )));
    assert!(text.contains("cannot be used as an expression"), "{}", text);
    assert!(text.contains("let (a, b) = f(..)"), "{}", text);
}

#[test]
fn a_function_with_no_outputs_produces_nothing_to_bind() {
    let text = compile_err(concat!(
        "fun nothing (a: i8)\n",
        "  let unused: i8 = a\n",
        "fun f (x: i8, s: out i8)\n",
        "  let (a, b) = nothing(x)\n",
        "  s = x\n",
    ));
    assert!(text.contains("has no `out` parameters"), "{}", text);
}

#[test]
fn a_multi_output_call_inside_a_branch_is_muxed() {
    // The results are ordinary bindings, so the SSA join treats them like any
    // other -- but only if both arms bind them.
    let v = compile(&format!("{}{}", DIVMOD, concat!(
        "fun f (c: i1, x: i8, y: i8, s: out i8)\n",
        "  if c then\n",
        "    let (q, r) = divmod(x, y)\n",
        "    s = q + r\n",
        "  else\n",
        "    s = x\n",
    )));
    assert!(v.contains("c ? "), "{}", v);
}

#[test]
fn a_multi_output_call_cannot_recurse() {
    let text = compile_err(concat!(
        "fun loopy (a: i8, p: out i8, q: out i8)\n",
        "  let (x, y) = loopy(a)\n",
        "  p = x\n",
        "  q = y\n",
    ));
    assert!(text.contains("calls itself"), "{}", text);
}

#[cfg_attr(miri, ignore = "reads files; Miri has no Windows path shims")]
#[test]
fn the_verified_alu_and_shifter_can_be_called() {
    // The reason this feature exists: k2g_alu has five `out` parameters and
    // k2g_shift has three, so neither was reachable from DDL until now.
    let k2g = std::path::PathBuf::from("../KAMASUTRA2G/rtl");
    // The generated package lives in the consumer's tree; skip when it is not
    // beside us rather than fail spuriously.
    if !k2g.join("k2g_pkg.ddl").is_file() {
        return;
    }
    // The imports in k2g_alu.ddl and k2g_shift.ddl pull in the types and the
    // package, so naming those two files is the whole dependency list.
    let (imported, load_diags) = crate::source::load_program(
        &["examples/k2g_alu.ddl".into(), "examples/k2g_shift.ddl".into()],
        vec![k2g],
    )
    .expect("the examples are readable");
    assert!(load_diags.is_empty(), "{}", imported.render_all(&load_diags));

    let src = imported.text().to_string()
        + concat!(
            "\nfun both (a: i32, b: i32, t: rdt_e, n: i5, ra: out i32, rs: out i32)\n",
            "  let (arith, ovf, log_r, cmp_r, un) = k2g_alu(a, b, t, t, ARITH_ADD, LOGIC_AND, CMP_EQ, UNARY_NEG)\n",
            "  let (sh, bx, bi) = k2g_shift(a, b, n, SHIFT_LL, 5'd0, 5'd8)\n",
            "  ra = arith\n",
            "  rs = sh\n",
        );
    let v = compile(&src);
    assert!(v.contains("module both ("), "{}", v);
    // Both helpers were inlined into one module: the ALU's carry-extended
    // adder and the shifter's arithmetic right shift are both present, and
    // there is no hierarchy -- one `module both`, no instantiations.
    assert!(v.contains("a33 + b33"), "{}", v);
    assert!(v.contains(">>>"), "{}", v);
    assert!(!v.contains("k2g_alu u_"), "{}", v);
}
