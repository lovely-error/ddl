// Verilator lint over every Verilog the compiler emits.
//
// The other tests in this repository check the output by asserting that
// substrings appear in it, and examples/verify.sh proves nine modules
// bit-equivalent to hand-written SystemVerilog. Between those two there is a
// gap: a substring assertion says the text changed, never that it is wrong,
// and verify.sh needs Questa, Gowin and an external tree, so it runs on one
// machine and rarely.
//
// A linter is an oracle that needs no reference. It reads the emitted Verilog
// as a synthesis tool would and objects to width mismatches, inferred latches,
// undriven and multiply-driven nets, combinational loops, and nets flopped on
// both clock edges -- a class of defect that no amount of `v.contains(...)`
// can see, because the assertion would have to know what to look for.
//
// The output is clean today at `-Wall`, which is what makes this worth wiring
// up now: every warning that is not waived in examples/lint.vlt is something
// the compiler started doing and did not use to.
//
// ---- running it elsewhere -------------------------------------------------
//
// Nothing here knows a path on any particular machine. Verilator is found on
// PATH, or wherever `$VERILATOR` says, and when it is absent the test says so
// and passes -- the same bargain `the_examples_still_compile` makes with the
// K2G tree, because a check that cannot run must not be a check that fails.
//
//   Linux/macOS   the package manager's `verilator`; nothing to configure.
//   Windows       MSYS2's `mingw-w64-x86_64-verilator`. Its `verilator` is a
//                 Perl wrapper that Git Bash's Perl cannot load, and the
//                 binary beside it is compiled with an MSYS2-internal data
//                 path, so both need saying:
//
//                   VERILATOR=/c/msys64/mingw64/bin/verilator_bin.exe
//                   VERILATOR_ROOT=/c/msys64/mingw64/share/verilator
//
//                 Those are two environment variables on the machine that has
//                 it, not two facts in this repository.
//   WSL           an ordinary Linux install; neither variable applies.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the waivers live. Repo-owned on purpose: the rule set has to be the
/// same everywhere, and a waiver argued for in a file a reader can open is
/// worth more than a flag buried in a command line.
const WAIVERS: &str = "examples/lint.vlt";

/// Stubs for the modules DDL did not compile. Passed with every file rather
/// than only with the one that needs it: which example instantiates an
/// `extern` is not something this test should have to know.
const EXTERNS: &str = "examples/externs.v";

/// The Verilator this was calibrated against.
///
/// `-Wall` is not a fixed set -- a later Verilator can add a rule and object
/// to output that has not changed. That is worth being told about rather than
/// insulated from, so the version is reported on failure instead of pinned.
const CALIBRATED_AGAINST: &str = "5.046";

/// The oldest Verilator that understands every rule the waivers name.
///
/// `MODMISSING` arrived in 5.038. Anything older rejects the WAIVER FILE and
/// never reads a line of the generated Verilog, so it cannot judge this
/// repository at all -- which is a different situation from a lint failure and
/// is reported as one. Ubuntu ships 5.020, which is how CI found this.
const NEEDS_AT_LEAST: &str = "5.038";

/// `$VERILATOR`, else `verilator` on PATH.
fn verilator() -> OsString {
    std::env::var_os("VERILATOR").unwrap_or_else(|| OsString::from("verilator"))
}

/// Whether a missing or broken Verilator is a failure rather than a skip.
///
/// Locally, skipping is right: not everybody has Verilator, and a suite that
/// goes red because a tool is absent teaches people to ignore red. In CI it is
/// the opposite -- the job exists to run the linter, so a skip there is the
/// check quietly not happening. The test passed, cargo swallowed the `SKIP` it
/// wrote to stderr, and the workflow went green having linted nothing.
///
/// CI sets `DDL_REQUIRE_VERILATOR=1`, which is what keeps passed, skipped and
/// failed distinguishable where it matters.
fn verilator_is_required() -> bool {
    std::env::var("DDL_REQUIRE_VERILATOR").is_ok_and(|v| v != "0" && !v.is_empty())
}

/// Every `.v` the compiler wrote, which is every `.v` here except the stubs.
///
/// The `.sv` files are hand-written references for examples/verify.sh and are
/// not this compiler's output, so they are not this compiler's to answer for.
fn generated_verilog() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir("examples")
        .expect("examples/ is beside Cargo.toml")
        .map(|e| e.expect("readable entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "v"))
        .filter(|p| p != Path::new(EXTERNS))
        .collect();
    // Deterministic, so a failure names the same file on every machine.
    out.sort();
    out
}

/// The `--lvt-bram` builds, written where they can be linted with the rest.
///
/// They are not checked-in `.v` files because each is a second way to build a
/// source that already has one, and two generated outputs from one source is
/// one of them going stale. But this is the newest emission in the backend --
/// bank arrays, per-read replicas, a table and the wires that select from it
/// -- so leaving it out would mean the least-exercised path is the one nothing
/// reads.
///
/// BOTH constructs, because they come out a different shape: a process muxes
/// every read onto one port, so its banks have one replica where a pipeline's
/// have one per read.
fn lvt_variants() -> Vec<PathBuf> {
    use ddl::driver::compile_to_verilog;
    use ddl::verilog::EmitOptions;

    let mut out = Vec::new();
    for name in ["rf_lvt", "rf_lvt_proc"] {
        let src = format!("examples/{}.ddl", name);
        let Ok((map, load_diags)) =
            ddl::source::load_program(std::slice::from_ref(&src), Vec::new())
        else {
            continue;
        };
        if !load_diags.is_empty() {
            continue;
        }
        let opts = EmitOptions {
            regenerate_cmd: format!("ddl build --lvt-bram {}", src),
            crossings: Vec::new(),
            lvt_bram: true,
            ..EmitOptions::default()
        };
        let Ok(text) = compile_to_verilog(&map, &opts) else {
            continue;
        };
        let dir = PathBuf::from("target/lint");
        if std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        let path = dir.join(format!("{}_lvt.v", name));
        if std::fs::write(&path, text).is_ok() {
            out.push(path);
        }
    }
    out
}

#[cfg_attr(miri, ignore = "runs a subprocess")]
#[test]
fn the_generated_verilog_passes_a_linter() {
    let bin = verilator();

    // Probing with `--version` separates "no Verilator here" from "Verilator
    // objected", which are the two outcomes a reader most needs kept apart.
    let version = match Command::new(&bin).arg("--version").output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => {
            let how = format!(
                "no Verilator ({}). Install it and re-run; see the header of tests/lint.rs for the two environment variables Windows needs.",
                bin.to_string_lossy()
            );
            assert!(!verilator_is_required(), "DDL_REQUIRE_VERILATOR is set, but {}", how);
            eprintln!("SKIP: {}", how);
            return;
        }
    };

    let mut files = generated_verilog();
    // Generated rather than found: see `lvt_variants`.
    files.extend(lvt_variants());
    let mut linted = 0;
    let mut failures: Vec<String> = Vec::new();

    for file in &files {
        let out = Command::new(&bin)
            .args(["--lint-only", "-Wall", WAIVERS, EXTERNS])
            .arg(file)
            .output()
            .expect("Verilator answered --version, so it runs");
        if out.status.success() {
            linted += 1;
            continue;
        }

        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // A waiver naming a rule this Verilator has never heard of is a
        // toolchain mismatch, not a defect in the netlist: the run stopped on
        // examples/lint.vlt and never looked at the file being linted. Every
        // file produces the identical message, so reporting them as lint
        // failures buries the single fact that matters -- the version.
        if text.contains("Unknown error code") {
            let how = format!(
                "{} does not understand a rule {} waives, so it stopped on the waivers and linted nothing. These need Verilator {} or newer; this is {}.",
                bin.to_string_lossy(),
                WAIVERS,
                NEEDS_AT_LEAST,
                version
            );
            assert!(!verilator_is_required(), "DDL_REQUIRE_VERILATOR is set, but {}", how);
            eprintln!("SKIP: {}", how);
            return;
        }
        // A data directory Verilator cannot find is a broken install, not a
        // broken netlist. Reporting it as a lint failure would send a reader
        // looking for a compiler bug that is not there.
        if text.contains("Cannot find verilated_std") {
            let how = format!(
                "{} cannot find its data directory. Set VERILATOR_ROOT; see the header of tests/lint.rs.",
                bin.to_string_lossy()
            );
            assert!(!verilator_is_required(), "DDL_REQUIRE_VERILATOR is set, but {}", how);
            eprintln!("SKIP: {}", how);
            return;
        }
        failures.push(format!("--- {}\n{}", file.display(), text.trim()));
    }

    assert!(
        failures.is_empty(),
        "{} objected to generated Verilog.\n\n{}\n\
         This ran {}, and the waivers in {} were written against {}. A rule that \
         version added is a real difference worth reading before it is waived.",
        bin.to_string_lossy(),
        failures.join("\n\n"),
        version,
        WAIVERS,
        CALIBRATED_AGAINST,
    );

    // An anti-vacuous guard, the same one examples/verify.sh applies to its
    // cell counts: a run that linted nothing proves nothing, and would pass.
    assert!(
        linted >= 9,
        "only {} files linted; the standalone examples alone are nine",
        linted
    );
}

/// The regenerate command a generated file carries in its own banner.
///
/// Returned as the argument list after `ddl build`, so this test drives the
/// compiler the same way the banner tells a reader to.
fn regenerate_args(text: &str) -> Option<Vec<String>> {
    let line = text.lines().find_map(|l| l.trim_start_matches("//").trim().strip_prefix("Regenerate with: ddl build "))?;
    Some(line.split_whitespace().map(|s| s.to_string()).collect())
}

#[test]
fn the_checked_in_verilog_is_what_this_compiler_produces() {
    // Most of what the linter reads is checked in, so linting it says what the
    // compiler USED to emit unless something ties the two together. Nothing
    // did: `examples/*.v` could drift arbitrarily far from the compiler in
    // this commit and every test would still pass, including the lint.
    //
    // Each file names its own regenerate command in its banner, so that is
    // what gets run -- which also keeps the banners honest.
    let mut checked = 0;
    for path in generated_verilog() {
        let text = std::fs::read_to_string(&path).expect("a generated file is readable");
        let Some(args) = regenerate_args(&text) else {
            panic!("{} has no `Regenerate with:` banner", path.display());
        };

        let mut inputs = Vec::new();
        let mut include = Vec::new();
        let mut lvt_bram = false;
        let mut export = ddl::ir_export::ExportFlags::default();
        let mut it = args.iter().peekable();
        let mut skip_output = false;
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "-o" => {
                    it.next();
                    skip_output = true;
                }
                "-I" => include.push(PathBuf::from(it.next().expect("-I takes a path"))),
                "--lvt-bram" => lvt_bram = true,
                "--export" => export.export = it.next().expect("--export takes names")
                    .split(',').map(|s| s.to_string()).collect(),
                "--bare-export" => export.bare = it.next().expect("--bare-export takes names")
                    .split(',').map(|s| s.to_string()).collect(),
                other => inputs.push(other.to_string()),
            }
        }
        assert!(skip_output, "{}: banner has no -o", path.display());

        // A file whose sources are not in this repository is not this
        // repository's to answer for; `generated_verilog` already drops the
        // hand-written stubs, and an unresolvable import is the K2G tree.
        let Ok((map, load_diags)) = ddl::source::load_program(&inputs, include) else {
            continue;
        };
        if !load_diags.is_empty() {
            continue;
        }
        let opts = ddl::verilog::EmitOptions {
            regenerate_cmd: format!("ddl build {}", args.join(" ")),
            crossings: Vec::new(),
            lvt_bram,
            export,
        };
        let Ok(fresh) = ddl::driver::compile_to_verilog(&map, &opts) else {
            continue;
        };
        assert_eq!(
            fresh.replace("\r\n", "\n"),
            text.replace("\r\n", "\n"),
            "{} is out of date; regenerate with `ddl build {}`",
            path.display(),
            args.join(" ")
        );
        checked += 1;
    }
    assert!(checked > 0, "no generated Verilog was checked, so this proves nothing");
    eprintln!("freshness: {} generated file(s) match the compiler", checked);
}
