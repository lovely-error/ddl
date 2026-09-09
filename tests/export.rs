// Which module an invocation is FOR, and what its boundary looks like.
//
// Two questions, and the interesting failures are all in the first one. A
// root is a module nothing else uses, and "uses" has two halves that are easy
// to conflate: the instances a `graph` builds, and the calls a body inlined. A
// `fun` call leaves no `Instance` behind, so a use graph built from
// `Module.instances` alone reports a called function as a root -- which is the
// `k2g_alu` / `rdt_is_signed` shape, and the reason those edges are recorded
// during lowering rather than derived afterwards.
//
// The second half is that an `import` supplies dependencies, not deliverables.
// Three of the examples carry `uop_nop` and `uop_fault` along and call
// neither; counting those would turn unambiguous files into a question.

use std::path::{Path, PathBuf};

use ddl::diag::SourceMap;
use ddl::driver::compile_to_verilog;
use ddl::ir_export::ExportFlags;
use ddl::source::load_program;
use ddl::verilog::EmitOptions;

fn flags(export: &[&str], bare: &[&str]) -> ExportFlags {
    ExportFlags {
        export: export.iter().map(|s| s.to_string()).collect(),
        bare: bare.iter().map(|s| s.to_string()).collect(),
    }
}

fn build(src: &str, f: ExportFlags) -> Result<String, String> {
    let map = SourceMap::new("t.ddl", src);
    let opts = EmitOptions { export: f, ..EmitOptions::default() };
    compile_to_verilog(&map, &opts).map_err(|d| map.render_all(&d))
}

fn ok(src: &str, f: ExportFlags) -> String {
    build(src, f).unwrap_or_else(|e| panic!("should compile:\n{}", e))
}

fn err(src: &str, f: ExportFlags) -> String {
    match build(src, f) {
        Err(e) => e,
        Ok(v) => panic!("should not compile:\n{}", v),
    }
}

const SEQ: &str = "sequence mul (src: buffer in u16, dst: buffer out u32)\n  let x = @rcv(src)\n  |||\n  @send(dst, {16'd0, x} + {16'd0, x})\n";

// ---- finding the root -----------------------------------------------------

#[test]
fn one_root_needs_no_flag() {
    let v = ok(SEQ, ExportFlags::default());
    // The logic kept its shape and gave up its name.
    assert!(v.contains("module mul_core ("), "{}", v);
    assert!(v.contains("module mul ("), "{}", v);
    assert!(v.contains("mul_core u_mul_core ("), "{}", v);
}

#[test]
fn several_roots_are_diagnosed_by_name() {
    // Not guessed. Which module a file is for is the author's to say, and
    // picking one silently is how a build ships the wrong top level.
    let text = err(
        "fun a (x: u1, o: out u1)\n  o = x\nfun b (x: u1, o: out u1)\n  o = x\n",
        ExportFlags::default(),
    );
    assert!(text.contains("several modules could be exported"), "{}", text);
    assert!(text.contains("`a`"), "{}", text);
    assert!(text.contains("`b`"), "{}", text);
    assert!(text.contains("--export <a>,<b>"), "{}", text);
}

#[test]
fn a_fun_another_fun_calls_is_not_a_root() {
    // The case a use graph built from `Module.instances` gets wrong: the call
    // is inlined, so there is no instance to find. Without the recorded call
    // edge this file would report two roots and refuse to build.
    let v = ok(
        "fun half (x: u8, o: out u8)\n  o = x >> 1'd1\nfun quarter (x: u8, o: out u8)\n  o = half(half(x))\n",
        ExportFlags::default(),
    );
    assert!(v.contains("module half ("), "{}", v);
    assert!(v.contains("module quarter ("), "{}", v);
}

#[test]
fn a_process_that_calls_a_fun_is_the_only_root() {
    let v = ok(
        "fun bump (x: u8, o: out u8)\n  o = x + 8'd1\nprocess p (src: buffer in u8, dst: buffer out u8)\n  loop\n    let a = @rcv(src)\n    @send(dst, bump(a))\n",
        ExportFlags::default(),
    );
    assert!(v.contains("module p_core ("), "{}", v);
    assert!(v.contains("module p ("), "{}", v);
    // The fun it calls is emitted, and is not itself a boundary.
    assert!(v.contains("module bump ("), "{}", v);
    assert!(!v.contains("module bump_core ("), "{}", v);
}

#[test]
fn a_graph_is_the_root_and_what_it_instantiates_is_not() {
    let v = ok(
        &format!("{}graph top (a: buffer in u16, b: buffer out u32)\n  mul(a, b)\n", SEQ),
        ExportFlags::default(),
    );
    assert!(v.contains("module top ("), "{}", v);
    assert!(v.contains("module top_core ("), "{}", v);
    // `mul` keeps its own name: it is used, so it is nobody's boundary.
    assert!(v.contains("module mul ("), "{}", v);
    assert!(!v.contains("module mul_core ("), "{}", v);
}

// ---- the flags ------------------------------------------------------------

#[test]
fn an_unknown_name_lists_the_modules_that_exist() {
    let text = err(SEQ, flags(&["nope"], &[]));
    assert!(text.contains("no module named `nope`"), "{}", text);
    assert!(text.contains("`mul`"), "{}", text);
}

#[test]
fn a_name_given_to_both_flags_is_refused() {
    let text = err(SEQ, flags(&["mul"], &["mul"]));
    assert!(text.contains("given to both --export and --bare-export"), "{}", text);
}

#[test]
fn bare_export_keeps_the_salt_ports() {
    let v = ok(SEQ, flags(&[], &["mul"]));
    // Byte for byte what the compiler emitted before any of this existed: the
    // module keeps its name, its salt ports, and gains no wrapper.
    assert!(v.contains("module mul ("), "{}", v);
    assert!(!v.contains("mul_core"), "{}", v);
    assert!(v.contains("input  [1:0]  src_wsalt"), "{}", v);
    assert!(!v.contains("can_receive"), "{}", v);
}

#[test]
fn several_named_targets_are_all_exported() {
    let src = format!(
        "{}sequence add (src: buffer in u16, dst: buffer out u16)\n  let x = @rcv(src)\n  |||\n  @send(dst, x + 16'd1)\n",
        SEQ
    );
    let v = ok(&src, flags(&["mul", "add"], &[]));
    for name in ["mul", "add"] {
        assert!(v.contains(&format!("module {}_core (", name)), "{}", v);
        assert!(v.contains(&format!("module {} (", name)), "{}", v);
    }
}

#[test]
fn naming_funs_exports_them_unwrapped_and_salt_free() {
    // A `fun` is already a plain combinational module with no salt anywhere,
    // so exporting one means emitting it as it stands.
    let v = ok(
        "fun a (x: u1, o: out u1)\n  o = x\nfun b (x: u1, o: out u1)\n  o = x\n",
        flags(&["a", "b"], &[]),
    );
    assert!(v.contains("module a ("), "{}", v);
    assert!(v.contains("module b ("), "{}", v);
    assert!(!v.contains("_core"), "{}", v);
    assert!(!v.contains("wsalt"), "{}", v);
}

#[test]
fn exporting_something_a_graph_uses_repoints_the_graph_at_the_core() {
    // The wrapper takes the name, so everything that already instantiated it
    // has to follow the logic to `_core` -- otherwise the graph would be
    // wired to the FIFO face through salt ports.
    //
    // Both are named, because exporting `mul` alone would emit `mul` and its
    // dependencies and nothing else; the graph that uses it is not one of
    // those.
    let src = format!("{}graph top (a: buffer in u16, b: buffer out u32)\n  mul(a, b)\n", SEQ);
    let v = ok(&src, flags(&["mul", "top"], &[]));
    assert!(v.contains("mul_core u_mul_core ("), "{}", v);
    assert!(v.contains("mul_core u_mul ("), "{}", v);
    assert!(!v.contains("  mul u_mul ("), "{}", v);
}

#[test]
fn a_target_pulls_in_its_dependencies_and_nothing_else() {
    // The file is the target and what it uses. An unrelated module in it is
    // one a synthesizer elaborates and a reader has to account for.
    let src = "fun half (x: u8, o: out u8)\n  o = x >> 1'd1\nfun quarter (x: u8, o: out u8)\n  o = half(half(x))\nfun unrelated (x: u8, o: out u8)\n  o = ~x\n";
    let v = ok(src, flags(&["quarter"], &[]));
    assert!(v.contains("module quarter ("), "{}", v);
    assert!(v.contains("module half ("), "the dependency is missing:\n{}", v);
    assert!(!v.contains("module unrelated ("), "an unrelated module came along:\n{}", v);
}

#[test]
fn naming_one_of_two_targets_emits_only_that_one() {
    // Two independent modules; asking for one must not ship the other.
    let src = "fun a (x: u1, o: out u1)\n  o = x\nfun b (x: u1, o: out u1)\n  o = ~x\n";
    let only_b = ok(src, flags(&["b"], &[]));
    assert!(only_b.contains("module b ("), "{}", only_b);
    assert!(!only_b.contains("module a ("), "{}", only_b);

    let both = ok(src, flags(&["a", "b"], &[]));
    assert!(both.contains("module a (") && both.contains("module b ("), "{}", both);
    assert_ne!(only_b, both, "naming one target and naming two gave the same file");
}

#[test]
fn a_constant_parameter_survives_onto_the_wrapper() {
    // Constants are folded away before the backend runs, so the `// Built
    // with:` header is the only place the number still appears. A wrapper
    // that dropped it would be a module whose dimensions are written nowhere.
    let v = ok(
        "sequence p (k: u8 = 8'd3, src: buffer in u8, dst: buffer out u8)\n  let x = @rcv(src)\n  |||\n  @send(dst, x + k)\n",
        ExportFlags::default(),
    );
    assert_eq!(v.matches("// Built with:").count(), 2, "{}", v);
    assert_eq!(v.matches("k : u8 = 8'd3").count(), 2, "{}", v);
}

// ---- candidates come from the command line --------------------------------

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-export").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        Scratch { dir }
    }

    fn write(&self, name: &str, body: &str) -> String {
        let path = self.dir.join(name);
        std::fs::write(&path, body).expect("scratch file");
        path.display().to_string()
    }
}

#[test]
fn an_unused_imported_declaration_is_not_a_candidate_and_is_not_emitted() {
    // The `uop_nop` shape. `lib.ddl` supplies a helper that `top.ddl` never
    // calls. It must not make the build ask which of the two it is for, and
    // it must not end up in the file either: an import is a place to look for
    // dependencies, not a list of things to ship.
    let s = Scratch::new("import-unused");
    s.write("lib.ddl", "fun helper (x: u8, o: out u8)\n  o = x\n");
    let top = s.write(
        "top.ddl",
        "import \"lib.ddl\"\nsequence only (src: buffer in u8, dst: buffer out u8)\n  let x = @rcv(src)\n  |||\n  @send(dst, x)\n",
    );

    let (map, load_diags) = load_program(&[top], Vec::new()).expect("readable");
    assert!(load_diags.is_empty(), "{}", map.render_all(&load_diags));
    let v = compile_to_verilog(&map, &EmitOptions::default())
        .unwrap_or_else(|d| panic!("should compile:\n{}", map.render_all(&d)));

    assert!(v.contains("module only ("), "{}", v);
    assert!(v.contains("module only_core ("), "{}", v);
    assert!(!v.contains("module helper ("), "an unused import was emitted:\n{}", v);
}

#[test]
fn an_imported_declaration_that_is_used_is_emitted() {
    // The other half: a dependency reached through an import belongs in the
    // file, and is still not a candidate for being the target.
    let s = Scratch::new("import-used");
    s.write("lib.ddl", "fun helper (x: u8, o: out u8)\n  o = x + 8'd1\n");
    let top = s.write(
        "top.ddl",
        "import \"lib.ddl\"\nsequence only (src: buffer in u8, dst: buffer out u8)\n  let x = @rcv(src)\n  |||\n  @send(dst, helper(x))\n",
    );

    let (map, load_diags) = load_program(&[top], Vec::new()).expect("readable");
    assert!(load_diags.is_empty(), "{}", map.render_all(&load_diags));
    let v = compile_to_verilog(&map, &EmitOptions::default())
        .unwrap_or_else(|d| panic!("should compile:\n{}", map.render_all(&d)));

    assert!(v.contains("module helper ("), "the dependency is missing:\n{}", v);
    assert!(v.contains("module only ("), "{}", v);
}

#[test]
fn every_file_named_on_the_command_line_is_a_candidate() {
    // The other side of the same rule: two files ASKED for, so two roots, and
    // the compiler will not choose between them.
    let s = Scratch::new("two-roots");
    let a = s.write("a.ddl", "fun one (x: u8, o: out u8)\n  o = x\n");
    let b = s.write("b.ddl", "fun two (x: u8, o: out u8)\n  o = x\n");

    let (map, _) = load_program(&[a, b], Vec::new()).expect("readable");
    let text = match compile_to_verilog(&map, &EmitOptions::default()) {
        Err(d) => map.render_all(&d),
        Ok(v) => panic!("should not compile:\n{}", v),
    };
    assert!(text.contains("several modules could be exported"), "{}", text);
    assert!(text.contains("`one`"), "{}", text);
    assert!(text.contains("`two`"), "{}", text);
}

#[test]
fn checking_does_not_need_an_export_chosen() {
    // `check` ran the whole of the Verilog path, so it asked which module the
    // file was FOR -- a question about producing an artifact. A file with
    // several roots could not be CHECKED until one was picked, and the
    // `--bare-export` the error suggested was parsed and then ignored, so
    // following the advice produced the same error again.
    let src = concat!(
        "fun a (x: u8, o: out u8)\n",
        "  o = x\n",
        "fun b (x: u8, o: out u8)\n",
        "  o = x\n",
    );
    let map = SourceMap::new("t.ddl", src);

    // Building is still entitled to ask.
    let built = compile_to_verilog(&map, &ddl::verilog::EmitOptions::default());
    assert!(built.is_err(), "a build with two roots must ask which one");

    // Checking is not.
    ddl::driver::check(&map, &ddl::verilog::EmitOptions::default())
        .expect("checking should not require an export");
}

#[test]
fn checking_still_reports_errors_in_the_source() {
    // The endpoint must not have become a no-op on the way to ignoring
    // exports: it still lowers every declaration.
    let map = SourceMap::new("t.ddl", "fun f (a: u8, hi: u3, o: out u8)\n  o = a[hi..0]\n");
    let diags = ddl::driver::check(&map, &ddl::verilog::EmitOptions::default())
        .expect_err("a bad range is still an error when checking");
    let text = map.render_all(&diags);
    assert!(text.contains("must be a constant known at compile time"), "{}", text);
}
