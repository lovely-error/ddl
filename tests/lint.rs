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

/// `$VERILATOR`, else `verilator` on PATH.
fn verilator() -> OsString {
    std::env::var_os("VERILATOR").unwrap_or_else(|| OsString::from("verilator"))
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

#[cfg_attr(miri, ignore = "runs a subprocess")]
#[test]
fn the_generated_verilog_passes_a_linter() {
    let bin = verilator();

    // Probing with `--version` separates "no Verilator here" from "Verilator
    // objected", which are the two outcomes a reader most needs kept apart.
    let version = match Command::new(&bin).arg("--version").output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => {
            eprintln!(
                "SKIP: no Verilator ({}). Install it and re-run; see the header of tests/lint.rs \
                 for the two environment variables Windows needs.",
                bin.to_string_lossy()
            );
            return;
        }
    };

    let files = generated_verilog();
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
        // A data directory Verilator cannot find is a broken install, not a
        // broken netlist. Reporting it as a lint failure would send a reader
        // looking for a compiler bug that is not there.
        if text.contains("Cannot find verilated_std") {
            eprintln!(
                "SKIP: {} cannot find its data directory. Set VERILATOR_ROOT; see the header of \
                 tests/lint.rs.",
                bin.to_string_lossy()
            );
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
