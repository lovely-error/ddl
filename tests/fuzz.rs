// Fuzzing the compiler against inputs nobody would write.
//
// The parser addresses source text by raw pointer: 147 of them in lex.rs, all
// doing arithmetic over a buffer, with `parse_top_level` the only thing
// between that arithmetic and whatever a file happens to contain. A wrong
// bound there is not a wrong answer, it is a read past the end -- and the
// thing that makes it hard to find by hand is that valid input never goes
// near it.
//
// THE PROPERTY: every input is either compiled or diagnosed. Not "compiles",
// not "produces a good error" -- just that the compiler returns. A panic is a
// bug, a read past the end is a worse one, and a hang is a bug too.
//
// No dependency and no external tool. `cargo-fuzz` wants libFuzzer, which is
// shaky on windows-msvc, and a fuzzer that only runs on one machine finds
// nothing on the others. This runs wherever `cargo test` does.
//
// DETERMINISTIC, so a failure is reproducible:
//
//     DDL_FUZZ_SEED=12345 cargo test --test fuzz
//     DDL_FUZZ_ITERS=1000000 cargo test --test fuzz --release
//
// Without a seed it uses a fixed one, so the suite tests the same inputs every
// run and a regression is not a coin flip. The long soak is the env var.

use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

use ddl::diag::SourceMap;
use ddl::driver::compile_to_verilog;
use ddl::verilog::EmitOptions;

/// xorshift64*. A PRNG rather than `rand`, because a fuzzer that needs a
/// dependency to run is a fuzzer that does not run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }

    /// `pick` on a slice of `&str` gives `&&str`; this saves the deref at
    /// every call site.
    fn word<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.next().is_multiple_of(one_in)
    }
}

/// The tokens a DDL program is made of, so mutation produces something that
/// reaches the parser rather than bouncing off the first byte.
const WORDS: &[&str] = &[
    "fun", "process", "sequence", "graph", "struct", "enum", "let", "var", "loop", "break",
    "for", "in", "if", "then", "else", "match", "import", "return", "buffer", "out",
    "inout", "@rcv", "@send", "@try_rcv", "@try_send", "@zext", "@sext", "@trunc", "@cast",
    "@concat", "@zeroed", "@assert", "@unreachable", "u1", "u8", "u32", "i32", "|||", "=>", "..",
    "==", "+=", "<<=", "#[impl(lutram)]", "#[impl(bram)]", "(", ")", "[", "]", ",", ":", "=",
    "+", "-", "*", "/", "%", "&", "|", "^", "~", "<", ">", "!", ".", "_", "\n", "  ", "    ",
    "8'd1", "32'hFF", "1'b0", "0", "999999999999999999999", "a", "x", "src", "dst", "-- c",
];

/// Programs that compile, as mutation seeds. Mutating something valid gets far
/// deeper than random bytes ever do -- past the parser, into lowering, which is
/// where most of the compiler is.
fn corpus() -> Vec<String> {
    #[cfg_attr(miri, allow(unused_mut))]
    let mut seeds: Vec<String> = vec![
        "fun f (a: u8, o: out u8)\n  o = a\n".into(),
        "sequence s (src: buffer in u16, dst: buffer out u16)\n  let a = @rcv(src)\n  |||\n  @send(dst, a)\n".into(),
        "process p (src: buffer in u32, dst: buffer out u32)\n  loop\n    let a = @rcv(src)\n    @send(dst, a)\n".into(),
        "enum e\n  A\n  B(u8)\nfun f (x: e, o: out u8)\n  var v: u8 = @zeroed()\n  match x\n    .A =>\n      v = 8'd0\n    .B d =>\n      v = d\n  o = v\n".into(),
        "struct s\n  a: u8\n  b: u8\nfun f (x: s, o: out u8)\n  o = x.a\n".into(),
        "fun f (o: out u8)\n  var acc: u8 = @zeroed()\n  for i in 0..4\n    acc = acc + 8'd1\n  o = acc\n".into(),
    ];
    // The examples too, when they are beside us -- they are the largest real
    // programs there are, and the k2g_* ones need a package we may not have.
    //
    // Not under Miri, which has no `GetFullPathNameW` and aborts rather than
    // handing back an error. The built-in seeds above cover every declaration
    // kind, so a Miri run loses breadth of corpus and none of the coverage
    // that matters -- and the point of a Miri run is the pointer arithmetic,
    // which does not care how realistic the input was.
    #[cfg(not(miri))]
    if let Ok(dir) = std::fs::read_dir("examples") {
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "ddl")
                && let Ok(text) = std::fs::read_to_string(&path) {
                    seeds.push(text);
                }
        }
    }
    seeds
}

/// One input. Four generators, because they reach different places: raw bytes
/// exercise the lexer's bounds, token soup gets into the parser, mutation gets
/// into lowering, and splicing produces the half-valid shapes a person
/// actually types.
fn generate(rng: &mut Rng, corpus: &[String]) -> String {
    match rng.below(4) {
        0 => {
            // Raw bytes, ASCII-weighted but not exclusively: a comment may hold
            // anything, and the column arithmetic assumes one byte per column.
            let len = rng.below(400);
            let mut s = String::with_capacity(len);
            for _ in 0..len {
                if rng.chance(20) {
                    s.push(*rng.pick(&['é', '←', '\u{1F600}', '\u{0}', '\t', '\r']));
                } else {
                    s.push((0x20 + rng.below(0x5f) as u8) as char);
                }
                if rng.chance(8) {
                    s.push('\n');
                }
            }
            s
        }
        1 => {
            // Token soup: everything the grammar knows, in an order it does not.
            let n = rng.below(120);
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(rng.word(WORDS));
                if rng.chance(3) {
                    s.push(' ');
                }
            }
            s
        }
        2 => {
            let seed = rng.pick(corpus).clone();
            mutate(rng, seed)
        }
        _ => {
            let a = rng.pick(corpus).clone();
            let b = rng.pick(corpus).clone();
            let cut_a = rng.below(a.len().max(1)).min(a.len());
            let cut_b = rng.below(b.len().max(1)).min(b.len());
            let mut s = String::new();
            s.push_str(floor_char(&a, cut_a));
            s.push_str(ceil_char(&b, cut_b));
            s
        }
    }
}

/// Up to eight edits to a program that compiles.
fn mutate(rng: &mut Rng, mut s: String) -> String {
    for _ in 0..1 + rng.below(8) {
        if s.is_empty() {
            break;
        }
        match rng.below(6) {
            // Truncate -- the shape that finds a reader running past the end.
            0 => {
                let at = rng.below(s.len());
                s = floor_char(&s, at).to_string();
            }
            // Splice a token in.
            1 => {
                let at = rng.below(s.len());
                let cut = floor_char(&s, at).len();
                s.insert_str(cut, rng.word(WORDS));
            }
            // Drop a line, which is how indentation gets torn.
            2 => {
                let lines: Vec<&str> = s.lines().collect();
                if !lines.is_empty() {
                    let drop = rng.below(lines.len());
                    s = lines
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != drop)
                        .map(|(_, l)| *l)
                        .collect::<Vec<_>>()
                        .join("\n");
                }
            }
            // Re-indent a line by a random amount.
            3 => {
                let lines: Vec<String> = s.lines().map(str::to_string).collect();
                if !lines.is_empty() {
                    let at = rng.below(lines.len());
                    let pad = " ".repeat(rng.below(9));
                    let mut out = lines;
                    out[at] = format!("{}{}", pad, out[at].trim_start());
                    s = out.join("\n");
                }
            }
            // Duplicate a line.
            4 => {
                let lines: Vec<String> = s.lines().map(str::to_string).collect();
                if !lines.is_empty() {
                    let at = rng.below(lines.len());
                    let mut out = lines.clone();
                    out.insert(at, lines[at].clone());
                    s = out.join("\n");
                }
            }
            // Flip one byte, staying inside ASCII so the result is still UTF-8.
            _ => {
                let bytes = s.as_bytes();
                let at = rng.below(bytes.len());
                if bytes[at].is_ascii_graphic() {
                    let mut b = bytes.to_vec();
                    b[at] = 0x21 + (rng.below(0x5e) as u8);
                    s = String::from_utf8(b).unwrap_or(s);
                }
            }
        }
    }
    s
}

/// The largest char boundary at or below `at`, so slicing never splits a
/// character. The compiler must survive multi-byte input; the HARNESS must not
/// be the thing that panics on it.
fn floor_char(s: &str, mut at: usize) -> &str {
    at = at.min(s.len());
    while at > 0 && !s.is_char_boundary(at) {
        at -= 1;
    }
    &s[..at]
}

fn ceil_char(s: &str, mut at: usize) -> &str {
    at = at.min(s.len());
    while at < s.len() && !s.is_char_boundary(at) {
        at += 1;
    }
    &s[at..]
}

/// Compiles one input and answers whether it returned at all.
///
/// `catch_unwind` so the input can be printed: a bare panic in a fuzz loop
/// tells you a bug exists and not which of ten thousand inputs caused it.
fn survives(src: &str) -> Result<(), String> {
    survives_counting(src, &mut 0, &mut 0)
}

/// As `survives`, and tallies which answer came back.
///
/// The tally is the anti-vacuous guard. A fuzzer whose every input is rejected
/// by the first byte of the parser tests the parser's rejection path and
/// nothing else -- it would pass forever while the lowering it is supposed to
/// be exercising went untouched.
fn survives_counting(src: &str, compiled: &mut u64, diagnosed: &mut u64) -> Result<(), String> {
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        let map = SourceMap::new("fuzz.ddl", src);
        // Both answers are fine. The property is that there IS an answer.
        compile_to_verilog(&map, &EmitOptions::default()).is_ok()
    }));
    if let Ok(true) = &outcome {
        *compiled += 1;
    } else if let Ok(false) = &outcome {
        *diagnosed += 1;
    }
    match outcome {
        Ok(_) => Ok(()),
        Err(payload) => {
            let what = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "a panic with no message".to_string());
            Err(what)
        }
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[test]
fn the_compiler_answers_or_diagnoses_but_never_panics() {
    let seed = env_u64("DDL_FUZZ_SEED", 0x5DD1_F0FF);
    // Miri interprets rather than executes, at roughly a hundredth of the
    // speed. Fewer cases, but every one of them checked for undefined
    // behaviour rather than merely for not crashing -- which is the whole
    // reason to run it: a read one past the end usually does not crash.
    let default_iters = if cfg!(miri) { 150 } else { 20_000 };
    let iters = env_u64("DDL_FUZZ_ITERS", default_iters);

    let corpus = corpus();
    assert!(corpus.len() >= 6, "the built-in seeds are missing");

    // Panic messages from the cases that are SUPPOSED to be caught would
    // otherwise bury the one that matters.
    let previous = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));

    let mut rng = Rng(seed);
    let mut failures: Vec<(u64, String, String)> = Vec::new();
    let (mut compiled, mut diagnosed) = (0u64, 0u64);
    let started = Instant::now();
    let budget = Duration::from_secs(env_u64("DDL_FUZZ_SECS", 60));

    for i in 0..iters {
        // The per-case seed, so one failing input is reproducible on its own.
        let case_seed = rng.0;
        let src = generate(&mut rng, &corpus);
        if let Err(what) = survives_counting(&src, &mut compiled, &mut diagnosed) {
            failures.push((case_seed, what, src));
            if failures.len() >= 5 {
                break;
            }
        }
        if i % 256 == 0 && started.elapsed() > budget {
            break;
        }
    }

    panic::set_hook(previous);

    // Visible with --nocapture, so a soak run can be judged rather than
    // trusted: an iteration count says nothing about how deep any of them got.
    println!(
        "fuzz: {} compiled, {} diagnosed, {} cases in {:?}",
        compiled,
        diagnosed,
        compiled + diagnosed,
        started.elapsed()
    );

    // A fuzzer that never reaches lowering is testing one `if` in the lexer.
    assert!(
        compiled > 0,
        "no generated input compiled, so nothing past the parser was exercised          ({} were diagnosed)",
        diagnosed
    );
    assert!(diagnosed > 0, "every input compiled, which cannot be right");

    if !failures.is_empty() {
        let mut report = format!("{} input(s) made the compiler panic\n", failures.len());
        for (case_seed, what, src) in &failures {
            report.push_str(&format!(
                "\n---- case seed {:#x} ----\n{}\n---- source ({} bytes) ----\n{}\n",
                case_seed,
                what,
                src.len(),
                src
            ));
        }
        panic!("{}", report);
    }
}

/// A hang is a bug too, and `catch_unwind` says nothing about one.
///
/// Separate from the loop above so the timeout is meaningful: this runs a
/// batch on its own thread and fails if the batch does not finish, rather
/// than letting CI sit on it until something else gives up.
///
/// Not under Miri, where it measures the interpreter rather than the compiler.
/// Miri runs this code roughly two orders of magnitude slower than native, so
/// a two-thousand-case batch cannot finish inside any wall-clock limit that
/// would mean anything natively -- the test failed there from the day it was
/// written, which is why `cargo miri test --test fuzz` never came back clean.
/// The two tests beside it are the ones Miri is FOR: they check the parser's
/// pointer arithmetic for undefined behaviour, which is a property no amount
/// of native running can rule out.
#[cfg_attr(miri, ignore = "a wall-clock timeout measures Miri, not the compiler")]
#[test]
fn no_input_makes_the_compiler_stop_answering() {
    let seed = env_u64("DDL_FUZZ_SEED", 0x11A26);
    let iters = env_u64("DDL_FUZZ_HANG_ITERS", 2_000);
    let limit = Duration::from_secs(env_u64("DDL_FUZZ_HANG_SECS", 120));

    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(|_| {}));
        let corpus = corpus();
        let mut rng = Rng(seed);
        for _ in 0..iters {
            let src = generate(&mut rng, &corpus);
            let _ = survives(&src);
        }
        panic::set_hook(previous);
        let _ = tx.send(());
    });

    match rx.recv_timeout(limit) {
        Ok(()) => {
            worker.join().expect("the worker finished");
        }
        Err(_) => panic!(
            "the fuzz batch did not finish in {:?}; an input is looping or is \
             pathologically slow. Re-run with DDL_FUZZ_SEED={:#x} and a smaller \
             DDL_FUZZ_HANG_ITERS to bisect it",
            limit, seed
        ),
    }
}

/// The inputs that have broken it before.
///
/// A fuzzer finds a bug once; this is what stops it coming back. Each entry
/// stays even after the fix, because the fix is what it is testing.
#[test]
fn inputs_that_broke_it_before_still_do_not() {
    let cases: &[(&str, &str)] = &[
        ("empty", ""),
        ("one newline", "\n"),
        ("only spaces", "     "),
        ("a lone keyword", "fun"),
        ("a header and nothing else", "fun f (a: u8, o: out u8)"),
        ("unterminated parameter list", "fun f (a: u8"),
        ("multi-byte in a comment", "-- \u{1F600}\nfun f (a: u8, o: out u8)\n  o = a\n"),
        ("multi-byte identifier", "fun \u{00e9} (a: u8, o: out u8)\n  o = a\n"),
        ("a NUL byte", "fun f (a: u8, o: out u8)\n  o = a\n\u{0}"),
        ("CR alone", "fun f (a: u8, o: out u8)\r  o = a\r"),
        ("no trailing newline", "fun f (a: u8, o: out u8)\n  o = a"),
        ("deep nesting", &"(".repeat(200)),
        // The one the fuzzer found. Nesting past what the parser follows used
        // to overflow the stack, which is the process dying with no
        // diagnostic -- `catch_unwind` cannot see it, so the harness above
        // would have reported nothing at all.
        ("balanced parens past the limit", &format!(
            "fun f (a: u8, o: out u8)
  o = {}a{}
",
            "(".repeat(5000),
            ")".repeat(5000),
        )),
        ("unbalanced open parens", &format!(
            "fun f (a: u8, o: out u8)
  o = {}a
",
            "(".repeat(5000),
        )),
        ("indented blocks past the limit", &{
            let mut s = String::from("fun f (c: u1, a: u8, o: out u8)
");
            for i in 0..300 {
                s.push_str(&" ".repeat(i + 1));
                s.push_str("if c then
");
            }
            s.push_str(&" ".repeat(301));
            s.push_str("o = a
");
            s
        }),
        ("a huge width", "fun f (a: u99999999, o: out u8)\n  o = a\n"),
        ("a huge literal", "fun f (o: out u8)\n  o = 999999999999999999999999999999\n"),
        ("indentation only", "fun f (a: u8, o: out u8)\n                    \n"),
    ];
    for (name, src) in cases {
        if let Err(what) = survives(src) {
            panic!("`{}` panicked: {}\n---- source ----\n{}", name, what, src);
        }
    }
}
