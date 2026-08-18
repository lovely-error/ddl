#![feature(decl_macro)]
#![feature(str_from_raw_parts)]
#![feature(iter_from_coroutine)]
#![feature(coroutines)]

mod diag;
mod driver;

#[allow(unsafe_op_in_unsafe_fn)]
mod lex;
#[allow(unsafe_op_in_unsafe_fn)]
mod parse;

mod sema;

mod lin;

mod symbols;
mod ty;
mod ir;
mod ir_fsm;
mod ir_match;
mod verilog;

#[cfg(test)]
mod tests_m2;

use std::process::ExitCode;

use diag::SourceMap;
use verilog::EmitOptions;

const USAGE: &str = "\
ddl -- a dataflow description language

USAGE:
    ddl build <input.ddl> [-o <output.v>]
    ddl check <input.ddl>

    build   compile to Verilog-2005; writes to stdout without -o
    check   parse and type-check only, emitting nothing

OPTIONS:
    -o <path>   write the generated Verilog here
    --check     with `build`, verify that <output.v> is up to date and exit 1
                if it is not, without writing. Mirrors `gen_defs --check`.
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
    input: String,
    output: Option<String>,
    check_only: bool,
}

fn parse_build_args(args: &[String]) -> Result<BuildArgs, String> {
    let mut input = None;
    let mut output = None;
    let mut check_only = false;
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
            "--check" => check_only = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown option `{}`", other));
            }
            other => {
                if input.is_some() {
                    return Err("more than one input file given".to_string());
                }
                input = Some(other.to_string());
            }
        }
        ix += 1;
    }
    match input {
        Some(input) => Ok(BuildArgs { input, output, check_only }),
        None => Err("no input file given".to_string()),
    }
}

fn load(path: &str) -> Result<SourceMap, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(SourceMap::new(path, text)),
        Err(e) => Err(format!("cannot read `{}`: {}", path, e)),
    }
}

fn run_build(args: &[String]) -> ExitCode {
    let args = match parse_build_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let map = match load(&args.input) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let opts = EmitOptions {
        regenerate_cmd: match &args.output {
            Some(out) => format!("ddl build {} -o {}", args.input, out),
            None => format!("ddl build {}", args.input),
        },
    };

    let verilog = match driver::compile_to_verilog(&map, &opts) {
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

fn run_check(args: &[String]) -> ExitCode {
    let path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("error: no input file given");
            return ExitCode::FAILURE;
        }
    };
    let map = match load(path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    match driver::compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(_) => ExitCode::SUCCESS,
        Err(diags) => {
            driver::report(&map, &diags);
            ExitCode::FAILURE
        }
    }
}
