// The `ddl` command line: argument parsing, file IO, and exit codes.
//
// The compiler itself is in the library; this file is what turns its result
// into something a shell can read.

use std::process::ExitCode;

use std::path::{Path, PathBuf};

use ddl::diag::{Diag, SourceMap};
use ddl::driver::{self, Emit};
use ddl::source;
use ddl::verilog::EmitOptions;

const USAGE: &str = "\
ddl -- a dataflow description language

USAGE:
    ddl build <input.ddl>... [-o <output.v>]
    ddl check <input.ddl>...
    ddl fmt   <input.ddl>... [--check]

    build   compile to Verilog-2005; writes to stdout without -o
    check   parse and type-check only, emitting nothing
    fmt     tidy whitespace in place; --check reports without writing.
            Indentation is never touched -- only the parser knows which
            deeper lines are blocks, and it discards that

Several inputs compile as one program, as does one input that names others
with `import \"path.ddl\"`. There are no namespaces: every declaration is
visible to every other, whichever file it is in.

OPTIONS:
    -o <path>       write the output here
    -I <dir>        also look here when resolving an import
    --check         with `build`, verify that <output.v> is up to date and exit
                    1 if it is not, without writing. Mirrors
                    `gen_defs --check`.
    --emit=<what>   what to produce: `v` (default) the Verilog-2005, `ir` the
                    lowered modules before the backend folds anything, `ast`
                    the declarations after precedence resolution, or `dot`
                    Graphviz of what each `graph` connects to what.
                    `--check` applies to `v` only.
    --export <a>,<b>
                    these modules present the FIFO interface a person wires up:
                    `p_can_receive`/`p_receive_en`/`p_data_write_in` on a pipe
                    they consume, `p_has_data`/`p_drop_item`/`p_data_read_out`
                    on one they produce. The module's own logic keeps its shape
                    under the name `<name>_core`, and the name it was declared
                    with becomes a wrapper holding the adapters.
                    Without this the compiler picks the one module nothing else
                    instantiates or calls, among the files named here. If
                    several qualify it names them and stops, rather than
                    choosing which module the build is for.
    --bare-export <a>,<b>
                    these keep the raw salt ports instead: two gray-code
                    pointers and a packed pair of entries, exactly as the
                    lowering produces them. For someone who would rather speak
                    the protocol directly than through a FIFO.
    --lvt-bram      build a `bram` that has more than one write port out of
                    one-write blocks: one bank per write port, replicated per
                    read port, and a live value table saying which bank holds
                    the newest value. Without it the ports are emitted as
                    written, which an ASIC memory compiler answers with a real
                    multi-write cell -- an FPGA has none, and infers no RAM at
                    all. Costs the table in flip-flops, so it is a choice.
";

/// Whether two paths name the same file.
///
/// `canonicalize` resolves `.`, `..`, symlinks and (on Windows) case, so
/// `-o SRC.DDL` is caught as well as `-o ./src.ddl`. It only works on files
/// that exist; an output path that does not exist yet cannot be an input, so
/// falling back to a plain comparison loses nothing.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Writes `text` to `path` without a moment where the file is half-written.
///
/// A plain `fs::write` truncates first, so an interrupted or failing write
/// leaves a partial file that still looks like an artifact -- and for a
/// generated file, one that a later `--check` compares against. The temporary
/// goes in the SAME directory so the rename stays on one filesystem, which is
/// what makes it atomic.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;

    let directory = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let temporary = directory.join(format!(".{}.{}.tmp", name, std::process::id()));

    let outcome = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        // Flushed and closed before the rename, so what lands is whole.
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if outcome.is_ok() {
        return outcome;
    }
    let _ = std::fs::remove_file(&temporary);

    // Not every destination is a file that can be renamed onto: `-o /dev/null`
    // (`nul` on Windows) is a device, and the rename fails with "cannot move
    // to a different disk drive". Discarding output that way is a normal thing
    // to ask for, so the direct write stays available -- it is only the
    // atomicity that is unavailable there, and a device has nothing to lose.
    std::fs::write(path, text)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprint!("{}", USAGE);
        return ExitCode::FAILURE;
    }

    match args[0].as_str() {
        "build" => run_build(&args[1..]),
        "check" => run_check(&args[1..]),
        "fmt" => run_fmt(&args[1..]),
        "-h" | "--help" | "help" => {
            print!("{}", USAGE);
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("unknown command `{}`", other);
            eprint!("{}", USAGE);
            ExitCode::FAILURE
        }
    }
}

struct BuildArgs {
    inputs: Vec<String>,
    include: Vec<PathBuf>,
    output: Option<String>,
    check_only: bool,
    emit: Emit,
    lvt_bram: bool,
    export: ddl::ir_export::ExportFlags,
}

fn parse_build_args(args: &[String]) -> Result<BuildArgs, String> {
    let mut inputs = Vec::new();
    let mut include = Vec::new();
    let mut output = None;
    let mut check_only = false;
    let mut emit = Emit::Verilog;
    let mut lvt_bram = false;
    let mut export = ddl::ir_export::ExportFlags::default();
    let mut ix = 0;
    while ix < args.len() {
        match args[ix].as_str() {
            "-o" => {
                ix += 1;
                match args.get(ix) {
                    Some(p) => output = Some(p.clone()),
                    None => return Err("-o needs a path".to_string()),
                }
            }
            "-I" => {
                ix += 1;
                match args.get(ix) {
                    Some(p) => include.push(PathBuf::from(p)),
                    None => return Err("-I needs a directory".to_string()),
                }
            }
            "--check" => check_only = true,
            "--export" | "--bare-export" => {
                let flag = args[ix].clone();
                ix += 1;
                match args.get(ix) {
                    Some(list) => {
                        let names = split_names(list, &flag)?;
                        if flag == "--export" {
                            export.export.extend(names);
                        } else {
                            export.bare.extend(names);
                        }
                    }
                    None => return Err(format!("{} needs a module name", flag)),
                }
            }
            "--lvt-bram" => lvt_bram = true,
            other if other.starts_with("--emit=") => {
                emit = match &other["--emit=".len()..] {
                    "v" | "verilog" => Emit::Verilog,
                    "ir" => Emit::Ir,
                    "ast" => Emit::Ast,
                    "dot" => Emit::Dot,
                    what => {
                        return Err(format!(
                            "unknown --emit target `{}`; expected `v`, `ir`, `ast` or `dot`",
                            what
                        ));
                    }
                };
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option `{}`", other));
            }
            other => inputs.push(other.to_string()),
        }
        ix += 1;
    }
    if inputs.is_empty() {
        return Err("no input file given".to_string());
    }
    Ok(BuildArgs { inputs, include, output, check_only, emit, lvt_bram, export })
}

/// Splits `a,b,c` into names, refusing an empty one.
///
/// An empty entry is almost always a stray comma, and silently dropping it
/// would export something other than what was asked for.
fn split_names(list: &str, flag: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for part in list.split(',') {
        let name = part.trim();
        if name.is_empty() {
            return Err(format!("{} has an empty name in `{}`", flag, list));
        }
        out.push(name.to_string());
    }
    Ok(out)
}

/// Loads the inputs and everything they import.
///
/// Unresolved imports come back as diagnostics rather than as an `Err`,
/// because they have a location worth rendering; only a root that cannot be
/// read at all fails before there is a `SourceMap` to render against.
fn load(inputs: &[String], include: &[PathBuf]) -> Result<(SourceMap, Vec<Diag>), String> {
    source::load_program(inputs, include.to_vec())
}

fn run_build(args: &[String]) -> ExitCode {
    let args = match parse_build_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let (map, load_diags) = match load(&args.inputs, &args.include) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    if !load_diags.is_empty() {
        driver::report(&map, &load_diags);
        return ExitCode::FAILURE;
    }

    let opts = EmitOptions {
        regenerate_cmd: regenerate_cmd(&args),
        lvt_bram: args.lvt_bram,
        export: args.export.clone(),
    };

    // `--check` compares against a checked-in generated file, and only the
    // Verilog is ever checked in. Failing here beats silently comparing an IR
    // dump against a .v and reporting it stale forever.
    if args.check_only && args.emit != Emit::Verilog {
        eprintln!("error: --check applies to `--emit=v` only");
        return ExitCode::FAILURE;
    }

    let verilog = match driver::compile(&map, &opts, args.emit) {
        Ok(v) => v,
        Err(diags) => {
            driver::report(&map, &diags);
            return ExitCode::FAILURE;
        }
    };

    let out_path = match &args.output {
        None => {
            if args.check_only {
                eprintln!("error: --check needs -o <path> to compare against");
                return ExitCode::FAILURE;
            }
            print!("{}", verilog);
            return ExitCode::SUCCESS;
        }
        Some(p) => p,
    };

    // Line endings are normalised before comparing: .gitattributes normalises
    // to LF but a fresh checkout on Windows can still land CRLF, which would
    // otherwise make --check fail on an identical file.
    let current = std::fs::read_to_string(out_path).ok().map(|s| s.replace("\r\n", "\n"));
    let unchanged = current.as_deref() == Some(verilog.as_str());

    if args.check_only {
        if unchanged {
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "error: `{}` is out of date; regenerate with `{}`",
            out_path, opts.regenerate_cmd
        );
        return ExitCode::FAILURE;
    }

    if unchanged {
        return ExitCode::SUCCESS;
    }
    // The source has already been read by this point, so `-o` naming one of
    // the inputs would replace it with Verilog and succeed: the program is
    // gone and the build reports success. Roots and imports both.
    let destination = Path::new(out_path);
    if let Some(clobbered) = map.paths().find(|p| same_file(Path::new(p), destination)) {
        eprintln!(
            "error: `{}` is an input to this build; writing the output there would replace it",
            clobbered
        );
        return ExitCode::FAILURE;
    }
    match write_atomically(destination, &verilog) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: cannot write `{}`: {}", out_path, e);
            ExitCode::FAILURE
        }
    }
}

/// `ddl fmt` -- whitespace hygiene, in place.
///
/// Each file is formatted on its own. There is no `-I` and no import
/// resolution: formatting is per-file, and the verification blanks import
/// lines the way the parser does.
fn run_fmt(args: &[String]) -> ExitCode {
    let mut check_only = false;
    let mut inputs: Vec<String> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--check" => check_only = true,
            other if other.starts_with('-') => {
                eprintln!("error: unknown option `{}`", other);
                return ExitCode::FAILURE;
            }
            other => inputs.push(other.to_string()),
        }
    }
    if inputs.is_empty() {
        eprintln!("error: no input file given");
        return ExitCode::FAILURE;
    }

    let mut needs_formatting = 0usize;
    let mut failed = false;
    for path in &inputs {
        // Read raw. Normalising the line endings before comparing would make a
        // CRLF file report as already formatted while still being CRLF on
        // disk -- the formatter would have quietly decided it was fine.
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: cannot read `{}`: {}", path, e);
                failed = true;
                continue;
            }
        };
        match ddl::fmt::format_source(&text) {
            ddl::fmt::Outcome::Unchanged => {}
            ddl::fmt::Outcome::Changed(formatted) => {
                needs_formatting += 1;
                if check_only {
                    eprintln!("{}: needs formatting", path);
                } else if let Err(e) = write_atomically(Path::new(path), &formatted) {
                    eprintln!("error: cannot write `{}`: {}", path, e);
                    failed = true;
                }
            }
            // Declining is not a failure of the file. Reported so it is not
            // silent, and does not set the exit code, so `--check` in a hook
            // does not fail a build over a file the formatter chose to leave.
            ddl::fmt::Outcome::Refused(why) => {
                eprintln!("{}: left alone, because {}", path, why);
            }
        }
    }

    if failed {
        return ExitCode::FAILURE;
    }
    if check_only && needs_formatting > 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// The command that reproduces this build, for the banner in the generated
/// file. It has to be runnable as printed, so every input and every `-I` is in
/// it -- a banner naming one of three inputs is worse than no banner.
fn regenerate_cmd(args: &BuildArgs) -> String {
    let mut cmd = String::from("ddl build");
    for input in &args.inputs {
        cmd.push(' ');
        cmd.push_str(input);
    }
    for dir in &args.include {
        cmd.push_str(" -I ");
        cmd.push_str(&dir.display().to_string());
    }
    if let Some(out) = &args.output {
        cmd.push_str(" -o ");
        cmd.push_str(out);
    }
    // The export selection changes which module the file presents, so a
    // banner that left it out would name a command that does not reproduce
    // the file it heads.
    for (flag, names) in [("--export", &args.export.export), ("--bare-export", &args.export.bare)] {
        if names.is_empty() {
            continue;
        }
        cmd.push(' ');
        cmd.push_str(flag);
        cmd.push(' ');
        cmd.push_str(&names.join(","));
    }
    cmd
}

fn run_check(args: &[String]) -> ExitCode {
    let args = match parse_build_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    // Accepting an option and then ignoring it is worse than refusing it: the
    // old `check` took `--bare-export`, dropped it, and repeated the very
    // message that had asked for it.
    let irrelevant: &[(&str, bool)] = &[
        ("-o", args.output.is_some()),
        ("--export", !args.export.export.is_empty()),
        ("--bare-export", !args.export.bare.is_empty()),
        ("--emit", args.emit != Emit::Verilog),
    ];
    for (flag, given) in irrelevant {
        if *given {
            eprintln!(
                "error: `{}` does not apply to `check`, which reports on the source rather than producing a file",
                flag
            );
            return ExitCode::FAILURE;
        }
    }
    let (map, load_diags) = match load(&args.inputs, &args.include) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    if !load_diags.is_empty() {
        driver::report(&map, &load_diags);
        return ExitCode::FAILURE;
    }
    // Only what checking can act on. `--lvt-bram` changes how memories lower,
    // so it changes what there is to check; an output path and an export
    // selection are about producing a file, and this produces none.
    let opts = EmitOptions { lvt_bram: args.lvt_bram, ..EmitOptions::default() };
    match driver::check(&map, &opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(diags) => {
            driver::report(&map, &diags);
            ExitCode::FAILURE
        }
    }
}
