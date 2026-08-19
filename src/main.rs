// The `ddl` command line: argument parsing, file IO, and exit codes.
//
// The compiler itself is in the library; this file is what turns its result
// into something a shell can read.

use std::process::ExitCode;

use std::path::PathBuf;

use ddl::diag::{Diag, SourceMap};
use ddl::driver::{self, Emit};
use ddl::source;
use ddl::verilog::EmitOptions;

const USAGE: &str = "\
ddl -- a dataflow description language

USAGE:
    ddl build <input.ddl>... [-o <output.v>]
    ddl check <input.ddl>...

    build   compile to Verilog-2005; writes to stdout without -o
    check   parse and type-check only, emitting nothing

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
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprint!("{}", USAGE);
        return ExitCode::FAILURE;
    }

    match args[0].as_str() {
        "build" => run_build(&args[1..]),
        "check" => run_check(&args[1..]),
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
}

fn parse_build_args(args: &[String]) -> Result<BuildArgs, String> {
    let mut inputs = Vec::new();
    let mut include = Vec::new();
    let mut output = None;
    let mut check_only = false;
    let mut emit = Emit::Verilog;
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
    Ok(BuildArgs { inputs, include, output, check_only, emit })
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

    let opts = EmitOptions { regenerate_cmd: regenerate_cmd(&args) };

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
    match std::fs::write(out_path, &verilog) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: cannot write `{}`: {}", out_path, e);
            ExitCode::FAILURE
        }
    }
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
    match driver::compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(_) => ExitCode::SUCCESS,
        Err(diags) => {
            driver::report(&map, &diags);
            ExitCode::FAILURE
        }
    }
}
