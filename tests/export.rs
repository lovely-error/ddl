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
        crossings: Vec::new(),
    }
}

/// The same, plus `--async-export`-style crossings, as `<pipe>=<domain>`.
fn crossed(export: &[&str], on: &str, pipes: &[&str]) -> ExportFlags {
    let mut f = flags(export, &[]);
    for spec in pipes {
        f.crossings
            .push(ddl::ir_export::parse_crossing("--async-export", &format!("{}.{}", on, spec), false).unwrap());
    }
    f
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


// ---------------------------------------------------------------------------
// Clock-domain crossings. The load-bearing property is the first test: asking
// for none must change nothing at all, because that is what makes the feature
// free for the designs -- nearly all of them -- that have one clock.

const TWO_PIPES: &str = "\
sequence filt (rx: buffer in u16, tx: buffer out u16)
  let v = @rcv(rx)
  |||
  @send(tx, v + 1)
";

#[test]
fn without_a_crossing_flag_the_output_is_unchanged() {
    let plain = ok(TWO_PIPES, flags(&["filt"], &[]));
    assert!(!plain.contains("ddl_cdc"), "a build that asked for no crossing emitted one");
    assert!(!plain.contains("_clk,"), "a build that asked for no crossing added a clock port");
    assert!(!plain.contains("CLOCK-DOMAIN CROSSINGS"));
}

#[test]
fn a_crossed_pipe_gains_one_clock_port_and_a_fifo() {
    let v = ok(TWO_PIPES, crossed(&["filt"], "filt", &["rx=io"]));
    assert!(v.contains("input         io_clk"), "{}", v);
    assert!(v.contains("module ddl_cdc_in_16x8"), "{}", v);
    assert!(v.contains("ddl_cdc_in_16x8 u_rx_cdc"), "{}", v);
    // The library modules go out verbatim, once.
    assert_eq!(v.matches("module ddl_cdc_fifo").count(), 1, "{}", v);
    assert_eq!(v.matches("module ddl_rst_cross").count(), 1, "{}", v);
    // The uncrossed pipe is untouched: no shell, no second clock.
    assert!(!v.contains("u_tx_cdc"), "{}", v);
}

#[test]
fn two_pipes_in_one_domain_share_one_clock_port() {
    let v = ok(TWO_PIPES, crossed(&["filt"], "filt", &["rx=io", "tx=io"]));
    assert_eq!(v.matches("io_clk,").count(), 1, "one port, not one per pipe:\n{}", v);
    assert!(v.contains("u_rx_cdc") && v.contains("u_tx_cdc"), "{}", v);
}

#[test]
fn two_pipes_in_two_domains_get_two_clock_ports() {
    let v = ok(TWO_PIPES, crossed(&["filt"], "filt", &["rx=a", "tx=b"]));
    assert!(v.contains("input         a_clk"), "{}", v);
    assert!(v.contains("input         b_clk"), "{}", v);
}

#[test]
fn each_direction_gets_the_shell_that_faces_the_right_way() {
    let v = ok(TWO_PIPES, crossed(&["filt"], "filt", &["rx=io", "tx=io"]));
    // `rx` is a `buffer in`: the outside writes, so data flows into the core.
    assert!(v.contains("module ddl_cdc_in_16x8"), "{}", v);
    // `tx` is a `buffer out`: the other way.
    assert!(v.contains("module ddl_cdc_out_16x8"), "{}", v);
}

#[test]
fn depth_is_per_boundary_and_names_the_module() {
    let v = ok(TWO_PIPES, crossed(&["filt"], "filt", &["rx=io:4", "tx=io:16"]));
    assert!(v.contains("module ddl_cdc_in_16x4"), "{}", v);
    assert!(v.contains("module ddl_cdc_out_16x16"), "{}", v);
    // Depth reaches the FIFO as an address width, not as a count.
    assert!(v.contains(".AW(2)"), "{}", v);
    assert!(v.contains(".AW(4)"), "{}", v);
}

#[test]
fn the_banner_says_what_the_timing_tool_must_be_told() {
    // Not through `ok`, which does not set the banner's crossing list -- this
    // is the path `main.rs` takes, so it is built the same way here.
    let f = crossed(&["filt"], "filt", &["rx=io"]);
    let map = SourceMap::new("t.ddl", TWO_PIPES);
    let opts = EmitOptions {
        crossings: f.crossings.iter().map(|c| (format!("{}.{}", c.owner, c.pipe), c.domain.clone())).collect(),
        export: f,
        ..EmitOptions::default()
    };
    let v = compile_to_verilog(&map, &opts).expect("should compile");
    assert!(v.contains("CLOCK-DOMAIN CROSSINGS"), "{}", v);
    assert!(v.contains("set_clock_groups -asynchronous"), "{}", v);
    assert!(v.contains("[get_clocks io_clk]"), "{}", v);
}

#[test]
fn a_crossing_on_something_that_is_not_wrapped_is_refused() {
    let f = crossed(&["filt"], "nope", &["rx=io"]);
    let err = build(TWO_PIPES, f).expect_err("should be refused");
    assert!(err.contains("not being wrapped"), "{}", err);
}

#[test]
fn a_crossing_on_a_pipe_that_does_not_exist_is_refused_and_lists_the_real_ones() {
    let f = crossed(&["filt"], "filt", &["nope=io"]);
    let err = build(TWO_PIPES, f).expect_err("should be refused");
    assert!(err.contains("no pipe called `nope`"), "{}", err);
    assert!(err.contains("`rx`") && err.contains("`tx`"), "{}", err);
}

#[test]
fn a_depth_that_the_fifo_cannot_use_is_refused_at_the_flag() {
    for bad in ["filt.rx:5", "filt.rx:2", "filt.rx:0"] {
        let e = ddl::ir_export::parse_crossing("--async-export", bad, false)
            .expect_err("should be refused");
        assert!(e.contains("power of two") || e.contains("at least 4"), "{}: {}", bad, e);
    }
    // And the shapes that are fine.
    for good in ["filt.rx:4", "filt.rx:8", "filt.rx:64"] {
        assert!(ddl::ir_export::parse_crossing("--async-export", good, false).is_ok(), "{}", good);
    }
}

#[test]
fn a_domain_defaults_to_the_pipes_own_name() {
    let c = ddl::ir_export::parse_crossing("--async-export", "filt.rx", false).unwrap();
    assert_eq!(c.domain, "rx");
    assert_eq!(c.depth, ddl::ir_export::DEFAULT_CDC_DEPTH);
}


const WITH_EXTERN: &str = "\
extern psram (req: buffer in u32, rsp: buffer out u32)

sequence drive (i: buffer in u32, o: buffer out u32)
  let v = @rcv(i)
  |||
  @send(o, v)

graph top (src: buffer in u32, dst: buffer out u32)
  let a: buffer u32
  let b: buffer u32
  drive(src, a)
  psram(a, b)
  drive(b, dst)
";

fn ext_crossed(pipes: &[&str]) -> ExportFlags {
    let mut f = flags(&["top"], &[]);
    for spec in pipes {
        f.crossings.push(
            ddl::ir_export::parse_crossing("--async-extern", &format!("top.u_psram.{}", spec), true)
                .unwrap(),
        );
    }
    f
}

#[test]
fn a_crossed_extern_keeps_its_own_clk_and_gains_the_faces_clock() {
    let v = ok(WITH_EXTERN, ext_crossed(&["req=mem"]));
    // Its core clock is untouched -- that was the whole point of naming the
    // face's clock on the ABI the face already uses.
    assert!(v.contains(".clk               (clk)"), "{}", v);
    assert!(v.contains(".req_clk           (mem_clk)"), "{}", v);
    assert!(v.contains(".req_rst_n         (mem_rst_n)"), "{}", v);
    assert!(v.contains("module ddl_cdc_to_ext_32x8"), "{}", v);
}

#[test]
fn an_extern_domain_reaches_the_wrapper_rather_than_dangling() {
    // The clock port is generated deep in `lower_graph`, on the graph itself.
    // A wrapper built afterwards has to carry it out, or it is an input nothing
    // drives -- which Verilog elaborates perfectly happily.
    let v = ok(WITH_EXTERN, ext_crossed(&["req=mem"]));
    let wrapper = v.split("module top (").nth(1).expect("a wrapper");
    let header = wrapper.split(");").next().unwrap();
    assert!(header.contains("mem_clk"), "not on the wrapper:\n{}", header);
    assert!(header.contains("mem_rst_n"), "not on the wrapper:\n{}", header);
    assert!(v.contains(".mem_clk   (mem_clk)") || v.contains(".mem_clk (mem_clk)"),
            "not passed to the core:\n{}", v);
}

#[test]
fn two_extern_pipes_can_sit_on_two_different_clocks() {
    let v = ok(WITH_EXTERN, ext_crossed(&["req=a", "rsp=b"]));
    assert!(v.contains(".req_clk           (a_clk)"), "{}", v);
    assert!(v.contains(".rsp_clk           (b_clk)"), "{}", v);
    // One in each direction.
    assert!(v.contains("module ddl_cdc_to_ext_32x8"), "{}", v);
    assert!(v.contains("module ddl_cdc_from_ext_32x8"), "{}", v);
}

#[test]
fn an_untouched_extern_is_exactly_as_it_was() {
    let v = ok(WITH_EXTERN, flags(&["top"], &[]));
    assert!(!v.contains("ddl_cdc"), "{}", v);
    assert!(!v.contains("_clk           ("), "no face clock should appear:\n{}", v);
}
