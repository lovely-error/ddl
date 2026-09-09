// Compilation driver: source text in, diagnostics or declarations out.
//
// This exists so that every stage has one place to report from. Before it,
// `parse_top_level` returned a bare `*const u8` and the only caller rendered
// it as `panic!("{}", *err as char)` -- the entire user-facing parse error was
// one character.

use std::collections::HashMap;

use crate::diag::{Diag, DiagSink, Severity, SourceMap, Span};
use crate::ir::{lower_function, lower_process};
use crate::lex::{AlphanumSpan, find_tab, parse_top_level, TopLevelDecl};
use crate::parse::{
    FunctionDecl, anumspan_to_str, resolve_precedence_for_enum, resolve_precedence_for_function,
    resolve_precedence_for_graph, resolve_precedence_for_process, resolve_precedence_for_sequence,
    resolve_precedence_for_struct,
};
use crate::symbols;
use crate::verilog::{EmitOptions, emit_banner, emit_modules};

/// One file's declarations and their source-map context for diagnostics.
/// Identifier and string text is owned by the AST. Extracting declarations
/// from this wrapper is safe even after the source map has been dropped;
/// source-location metadata is meaningful only with its original map.
pub struct Parsed<'a> {
    pub decls: Vec<TopLevelDecl>,
    _buffer: std::marker::PhantomData<&'a SourceMap>,
}

/// Parses one source file.
///
/// Parsing borrows the source while scanning. Returned identifier and string
/// spans own their text, while retaining source locations for diagnostics.
pub fn parse_source<'a>(map: &'a SourceMap) -> Result<Parsed<'a>, Vec<Diag>> {
    let base = map.base_ptr();
    let end = map.end_ptr();

    // Checked before parsing rather than during. The parser counts only spaces
    // for indentation, so a tab does not fail -- it silently changes which
    // block a statement belongs to, which is far worse than an error.
    if let Some(tab) = find_tab(base, end) {
        let span = map.span_at_ptr(tab);
        return Err(vec![Diag::error(span, "tab character in source")
            .with_note("DDL blocks are delimited by indentation, which counts spaces only")]);
    }

    match unsafe { parse_top_level(base, map.len()) } {
        Ok(decls) => Ok(Parsed { decls, _buffer: std::marker::PhantomData }),
        Err(err) => {
            // Depth exhaustion is not a syntax error at the token it stopped
            // on: the source is nested past what the parser will follow, and
            // saying `unexpected \`(\`` about the ninety-seventh one sends the
            // reader looking at a parenthesis that is no different from the
            // ninety-sixth.
            if crate::lex::nesting_overflowed() {
                let span = map.span_at_ptr(err.at);
                return Err(vec![
                    Diag::error(span, "this nests deeper than the parser will follow")
                        .with_note(format!(
                            "the limit is {} levels of brackets or indented blocks; past that the parser runs out of stack, which is a crash rather than a diagnostic",
                            crate::lex::NESTING_LIMIT
                        )),
                ]);
            }
            // A parser that knew what was wrong said so. Only used when it
            // sits at or past where the parse gave up, so a diagnosis left
            // behind by a speculative parse some other parser then succeeded
            // at is never what gets reported.
            if let Some(d) = crate::lex::taken_diagnosis()
                && d.at as usize >= err.at as usize
            {
                let diag = Diag::error(map.span_at_ptr(d.at), d.message);
                return Err(vec![match d.note {
                    Some(note) => diag.with_note(note),
                    None => diag,
                }]);
            }

            let span = map.span_at_ptr(err.at);
            let offset = map.offset_of(err.at);
            let word: String = if offset >= map.len() {
                String::new()
            } else {
                map.text()[offset as usize..]
                    .chars()
                    .take_while(|c| !c.is_whitespace())
                    .take(24)
                    .collect()
            };

            // Inside a declaration the keyword is not the problem -- the line
            // is -- so the message names the line and the note says what a
            // body of that kind holds. Reporting `unexpected \`fun\`` at the
            // declaration, which is what this did, blames the one line that
            // parsed.
            let (msg, note) = match err.inside {
                Some(kind) => (
                    if word.is_empty() {
                        format!("this does not belong in {} body", a_kind(kind))
                    } else {
                        format!("`{}` does not belong in {} body", word, a_kind(kind))
                    },
                    body_hint(kind).to_string(),
                ),
                None => (
                    if offset >= map.len() {
                        "unexpected end of input".to_string()
                    } else if word.is_empty() {
                        "unexpected input here".to_string()
                    } else {
                        format!("unexpected `{}`", word)
                    },
                    "expected a top-level `process`, `sequence`, `graph`, `fun`, `struct` or `enum` declaration".to_string(),
                ),
            };
            Err(vec![Diag::error(span, msg).with_note(note)])
        }
    }
}

/// `a \`graph\``, `an \`enum\`` -- the article that reads correctly.
fn a_kind(kind: &str) -> String {
    let article = if kind.starts_with('e') { "an" } else { "a" };
    format!("{} `{}`", article, kind)
}

/// What each kind of declaration holds, for the note under a body error.
fn body_hint(kind: &str) -> &'static str {
    match kind {
        "graph" => "a graph body declares a pipe with `let <name>: buffer <T>`, or instantiates one with `Name(a, b)`",
        "enum" => "an enum body names one variant per line, with either `(<payload type>)` or `= <discriminant>` after the name, and never both",
        "struct" => "a struct body names one field per line, as `<name>: <type>`",
        "sequence" => "a sequence body holds statements and `|||` stage cuts",
        _ => "a body holds one statement per line, indented past the declaration",
    }
}

/// Source in, Verilog-2005 out.
///
/// The whole vertical slice: parse, resolve precedence, type and lower each
/// combinational `fun`, emit. Declarations that are not functions are skipped
/// with a diagnostic rather than silently ignored -- `process` and `sequence`
/// need a state machine and a pipeline scheduler respectively.
/// How far to run, and what to print.
///
/// The two early exits are for reading, not for feeding to another tool: they
/// answer "did the parser see what I wrote" and "what did the compiler decide"
/// without having to infer either from the Verilog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emit {
    /// Declarations after precedence resolution.
    Ast,
    /// Lowered modules, before the backend folds anything.
    Ir,
    /// The generated Verilog-2005.
    Verilog,
    /// Graphviz of the `graph` declarations: what is connected to what.
    Dot,
    /// Everything the other modes check, and nothing they produce.
    ///
    /// `ddl check` used to run the whole of `Emit::Verilog`, which made it ask
    /// which module the file was FOR -- a question about producing an artifact,
    /// not about whether the source is correct. A file with several roots
    /// therefore failed to CHECK until an export was chosen, and the
    /// `--bare-export` the message suggested was parsed and then dropped on
    /// the floor, so taking the advice changed nothing.
    Check,
}

pub fn compile_to_verilog(map: &SourceMap, opts: &EmitOptions) -> Result<String, Vec<Diag>> {
    compile(map, opts, Emit::Verilog)
}

/// Checks a program without choosing an export or rendering anything.
///
/// Every declaration is parsed, resolved and lowered, so this reports what a
/// build would report about the source itself -- and nothing about which of
/// several roots the output should present, which is a question only a build
/// has to answer.
pub fn check(map: &SourceMap, opts: &EmitOptions) -> Result<(), Vec<Diag>> {
    compile(map, opts, Emit::Check).map(|_| ())
}

/// The stack the compiler runs on.
///
/// Recursive descent puts source nesting on the stack, and a debug build
/// spends about eight times as much of it per level as a release build does --
/// so the depth a program compiles at would otherwise depend on which binary
/// compiled it. Fifty levels of parentheses was enough to kill the debug
/// binary, which is not a diagnostic but a dead process.
///
/// A thread with a known stack makes the limit a property of the language
/// rather than of the build profile. `lex::NESTING_LIMIT` is then a rule
/// somebody chose, and it is enforced identically everywhere.
const COMPILER_STACK: usize = 64 * 1024 * 1024;

/// Lowers one declaration, and refuses to lose it quietly.
///
/// A lowering that returns `None` is claiming it already explained itself. When
/// it has not, the declaration disappears from the output while the build still
/// reports success -- `ddl build` writing a Verilog file that is missing one of
/// the functions it was given, exit code 0. A range whose ends would not fold
/// did exactly that. The reason belongs at the mistake, so this does not try to
/// produce one; it makes the silence itself an error, so no future `None` on
/// this path can cost a declaration.
fn lowered<T>(
    sink: &mut DiagSink,
    what: &str,
    name: &AlphanumSpan,
    lower: impl FnOnce(&mut DiagSink) -> Option<T>,
) -> Option<T> {
    let before = sink.error_count();
    let lowered = lower(sink);
    if lowered.is_none() && sink.error_count() == before {
        let span = sink.span_of(name);
        sink.push(
            Diag::error(
                span,
                format!(
                    "internal error: {} `{}` failed to compile without reporting why; this is a bug in the compiler",
                    what,
                    anumspan_to_str(name)
                ),
            )
            .as_internal(),
        );
    }
    lowered
}

pub fn compile(map: &SourceMap, opts: &EmitOptions, emit: Emit) -> Result<String, Vec<Diag>> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(COMPILER_STACK)
            .name("ddl-compile".to_string())
            .spawn_scoped(scope, || compile_on_this_stack(map, opts, emit))
            .expect("a thread for the compiler")
            .join()
            .unwrap_or_else(|_| {
                Err(vec![Diag::internal_error(
                    "the compiler panicked; this is a bug in the compiler",
                )])
            })
    })
}

fn compile_on_this_stack(
    map: &SourceMap,
    opts: &EmitOptions,
    emit: Emit,
) -> Result<String, Vec<Diag>> {
    let parsed = parse_source(map)?;
    let base = map.base_ptr();
    let mut sink = DiagSink::new(map);

    // Pass 1: resolve precedence for every declaration.
    //
    // Split by kind first, because the symbol table has to exist before any
    // function body is lowered -- that is what makes declarations mutually
    // visible regardless of the order they appear in the file.
    let mut enums = Vec::new();
    let mut structs = Vec::new();
    let mut funcs = Vec::new();
    let mut procs = Vec::new();
    let mut seqs = Vec::new();
    let mut graphs = Vec::new();
    let mut externs = Vec::new();

    for decl in &parsed.decls {
        match decl {
            TopLevelDecl::FunctionStmt(f) => {
                match unsafe { resolve_precedence_for_function(base, f) } {
                    Ok(r) => funcs.push(r),
                    Err(_) => sink.err_at(&f.name, "could not resolve this function"),
                }
            }
            TopLevelDecl::EnumDecl(e) => {
                match unsafe { resolve_precedence_for_enum(base, e) } {
                    Ok(r) => enums.push(r),
                    Err(_) => sink.err_at(&e.name, "could not resolve this enum"),
                }
            }
            TopLevelDecl::ExternDecl(e) => {
                match unsafe { crate::parse::resolve_precedence_for_extern(base, e) } {
                    Ok(r) => externs.push(r),
                    Err(_) => sink.err_at(&e.name, "could not resolve this extern"),
                }
            }
            TopLevelDecl::StructDecl(st) => {
                match unsafe { resolve_precedence_for_struct(base, st) } {
                    Ok(r) => structs.push(r),
                    Err(_) => sink.err_at(&st.name, "could not resolve this struct"),
                }
            }
            TopLevelDecl::ProcessStmt(p) => {
                match unsafe { resolve_precedence_for_process(base, p) } {
                    Ok(r) => procs.push(r),
                    Err(_) => sink.err_at(&p.name, "could not resolve this process"),
                }
            }
            TopLevelDecl::SequenceDecl(sq) => {
                match unsafe { resolve_precedence_for_sequence(base, sq) } {
                    Ok(r) => seqs.push(r),
                    Err(_) => sink.err_at(&sq.name, "could not resolve this sequence"),
                }
            }
            TopLevelDecl::GraphDecl(g) => {
                match unsafe { resolve_precedence_for_graph(base, g) } {
                    Ok(r) => graphs.push(r),
                    Err(_) => sink.err_at(&g.name, "could not resolve this graph"),
                }
            }
        }
    }
    if sink.has_errors() {
        return Err(sink.into_diags());
    }

    if emit == Emit::Ast {
        let mut out = String::new();
        for e in &enums {
            out.push_str(&format!("{:#?}\n", e));
        }
        for s in &structs {
            out.push_str(&format!("{:#?}\n", s));
        }
        for f in &funcs {
            out.push_str(&format!("{:#?}\n", f));
        }
        for s in &seqs {
            out.push_str(&format!("{:#?}\n", s));
        }
        for p in &procs {
            out.push_str(&format!("{:#?}\n", p));
        }
        return Ok(out);
    }

    // Pass 2: the symbol table.
    let syms = symbols::build(&enums, &structs, &funcs, &mut sink);
    if sink.has_errors() {
        return Err(sink.into_diags());
    }

    // Pass 3: lower and emit. Bodies are indexed by name so a call can be
    // inlined without searching.
    let mut module_origins = std::collections::HashSet::new();
    for name in funcs
        .iter()
        .map(|d| &d.name)
        .chain(seqs.iter().map(|d| &d.name))
        .chain(procs.iter().map(|d| &d.name))
        .chain(graphs.iter().map(|d| &d.name))
        .chain(externs.iter().map(|d| &d.name))
    {
        if !module_origins.insert(anumspan_to_str(name)) {
            sink.err_at(
                name,
                format!(
                    "module `{}` is declared more than once",
                    anumspan_to_str(name)
                ),
            );
        }
    }
    if sink.has_errors() {
        return Err(sink.into_diags());
    }
    let bodies: HashMap<String, &FunctionDecl> = funcs
        .iter()
        .map(|f| (anumspan_to_str(&f.name).to_string(), f))
        .collect();

    let dumping_ir = emit == Emit::Ir;
    let drawing = emit == Emit::Dot;
    let mut out = if dumping_ir || drawing { String::new() } else { emit_banner(opts) };
    let mut emitted = 0usize;
    // `--emit=dot` needs every module at once -- a graph's picture is drawn
    // from what it instantiates -- so they are collected rather than rendered
    // one at a time.
    let mut drawn: Vec<crate::ir::Module> = Vec::new();
    // Collected across graphs and again across export wrappers: one module per
    // shape, however many boundaries ask for it.
    let mut adapts: Vec<crate::ir_adapt::AdaptUse> = Vec::new();
    let mut render = |out: &mut String, module: &crate::ir::Module| {
        if drawing || !dumping_ir {
            drawn.push(module.clone());
            return;
        }
        out.push('\n');
        out.push_str(&crate::ir::render_module(module));
    };
    for func in &funcs {
        if let Some(module) = lowered(&mut sink, "fun", &func.name, |sink| {
            lower_function(map, &syms, &bodies, func, sink)
        }) {
            render(&mut out, &module);
            emitted += 1;
        }
    }
    for seq in &seqs {
        if let Some(module) = lowered(&mut sink, "sequence", &seq.name, |sink| {
            crate::ir_pipe::lower_sequence(map, &syms, &bodies, seq, sink)
        }) {
            render(&mut out, &module);
            emitted += 1;
        }
    }
    for proc in &procs {
        if let Some(module) = lowered(&mut sink, "process", &proc.name, |sink| {
            lower_process(map, &syms, &bodies, proc, sink)
        }) {
            render(&mut out, &module);
            emitted += 1;
        }
    }

    // Graphs last, and after their contents: a graph is the only declaration
    // that emits an instantiation, so everything it names has to have been
    // emitted above it for the file to be readable top to bottom.
    // Built whether or not the file has a graph. A signature is what a graph
    // connects to AND what the export pass reads a boundary off, and an
    // `extern` that is never instantiated is still worth checking -- before
    // this was hoisted, a malformed one in a graph-less file was silently
    // accepted.
    let mut sigs: std::collections::BTreeMap<String, crate::ir_graph::BlockSig> =
        std::collections::BTreeMap::new();
    {
        // An `extern` contributes a signature and no module. That is the whole
        // of the feature: a graph can instantiate something DDL did not
        // compile, and the connections are still checked, because what a graph
        // can see of anything it instantiates is its pipe interface and
        // nothing else.
        for ext in &externs {
            let name = anumspan_to_str(&ext.name).to_string();
            crate::ir_graph::check_extern(map, ext, &syms, &mut sink);
            sigs.insert(
                name.clone(),
                crate::ir_graph::signature_of(&name, "extern", &ext.args, &syms),
            );
        }
        for seq in &seqs {
            let name = anumspan_to_str(&seq.name).to_string();
            sigs.insert(name.clone(), crate::ir_graph::signature_of(&name, "sequence", &seq.args, &syms));
        }
        for proc in &procs {
            let name = anumspan_to_str(&proc.name).to_string();
            sigs.insert(name.clone(), crate::ir_graph::signature_of(&name, "process", &proc.args, &syms));
        }
        for graph in &graphs {
            let name = anumspan_to_str(&graph.name).to_string();
            sigs.insert(name.clone(), crate::ir_graph::signature_of(&name, "graph", &graph.args, &syms));
        }
    }

    if !graphs.is_empty() {
        // Before any of them is lowered: a cycle is a property of the set, and
        // lowering one declaration can only see the one declaration.
        crate::ir_graph::check_graph_cycles(map, &graphs, &mut sink);
        if sink.has_errors() {
            return Err(sink.into_diags());
        }
        // Combinators first, in the file: a graph instantiates them, and the
        // rule for this file is that everything a graph names has been emitted
        // above it so the whole thing reads top to bottom.
        let mut combs: Vec<crate::ir_graph::CombUse> = Vec::new();
        let mut lowered = Vec::new();
        for graph in &graphs {
            if let Some(module) =
                crate::ir_graph::lower_graph(map, &syms, &sigs, graph, &mut combs, &mut adapts, &mut sink)
            {
                lowered.push(module);
            }
        }
        for use_ in &adapts {
            if let Some(module) =
                crate::ir_adapt::build(map, &syms, use_.kind, &use_.ty, &mut sink)
            {
                render(&mut out, &module);
                emitted += 1;
            }
        }
        for use_ in &combs {
            if let Some(module) =
                crate::ir_comb::build(map, &syms, use_.kind, use_.fan, &use_.ty, &mut sink)
            {
                render(&mut out, &module);
                emitted += 1;
            }
        }
        for module in &lowered {
            render(&mut out, module);
            emitted += 1;
        }
    }

    if sink.has_errors() {
        return Err(sink.into_diags());
    }
    if drawing {
        return Ok(crate::dot::render(&drawn));
    }
    // Everything above has run: every declaration is lowered and every
    // diagnostic it had is in the sink. What is left below is choosing and
    // rendering an artifact, which checking has no opinion about.
    if emit == Emit::Check {
        return if sink.has_errors() { Err(sink.into_diags()) } else { Ok(String::new()) };
    }
    if emitted == 0 {
        return Err(vec![Diag::error_no_span(
            "nothing to emit: no `fun`, `process`, `sequence` or `graph` declarations found",
        )]);
    }
    if !dumping_ir {
        // Which module is this invocation FOR? Only the declarations the
        // author named can answer: an `import` supplies dependencies, and one
        // of those being unused does not make it the deliverable.
        let mut candidates: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for name in funcs
            .iter()
            .map(|d| &d.name)
            .chain(seqs.iter().map(|d| &d.name))
            .chain(procs.iter().map(|d| &d.name))
            .chain(graphs.iter().map(|d| &d.name))
        {
            if map.is_root_at(map.span_of(name).lo) {
                candidates.insert(anumspan_to_str(name).to_string());
            }
        }

        let exports = crate::ir_export::resolve(&drawn, &candidates, &opts.export)
            .map_err(|d| vec![d])?;

        // The file is the targets and what they use. A module that has nothing
        // to do with what was asked for is one a synthesizer would elaborate
        // and a reader would have to account for.
        let keep = crate::ir_export::closure(&drawn, &exports.keep);
        drawn.retain(|m| keep.contains(&m.name));

        let wrappers_at = drawn.len();
        drawn = crate::ir_export::apply(drawn, &exports.wrap, &sigs, &mut adapts);

        // Wrapping asks for adapters of its own, and they have to appear
        // above the wrapper that names them.
        let have: std::collections::BTreeSet<String> =
            drawn.iter().map(|m| m.name.clone()).collect();
        // `adapts` collected a use for every pipe of every `extern` in the
        // whole compilation, and the retain above has just thrown some of
        // those graphs away. Rebuilding the list wholesale put their adapters
        // back: `--export wanted` emitted `ddl_salt_to_wport_7` for an extern
        // only an unexported graph ever named, with no instantiation anywhere
        // in the file. Nothing catches that downstream either -- an
        // unreferenced module is Verilator's MULTITOP, and examples/lint.vlt
        // waives it for a reason of its own.
        //
        // So an adapter is built when something KEPT asks for it, which after
        // wrapping is exactly the instances the surviving modules name.
        let referenced: std::collections::BTreeSet<&str> = drawn
            .iter()
            .flat_map(|m| m.instances.iter().map(|i| i.module.as_str()))
            .collect();
        let mut fresh = Vec::new();
        for use_ in &adapts {
            let name = crate::ir_adapt::module_name(use_.kind, &use_.ty);
            if have.contains(&name) || !referenced.contains(name.as_str()) {
                continue;
            }
            if let Some(module) =
                crate::ir_adapt::build(map, &syms, use_.kind, &use_.ty, &mut sink)
            {
                fresh.push(module);
            }
        }
        if sink.has_errors() {
            return Err(sink.into_diags());
        }
        drawn.splice(wrappers_at..wrappers_at, fresh);

        let verilog =
            emit_modules(&drawn, opts).map_err(|message| vec![Diag::error_no_span(message)])?;
        out.push('\n');
        out.push_str(&verilog);
    }
    Ok(out)
}

/// Renders diagnostics and reports whether any of them were fatal.
pub fn report(map: &SourceMap, diags: &[Diag]) -> bool {
    if !diags.is_empty() {
        eprintln!("{}", map.render_all(diags));
    }
    diags.iter().any(|d| d.severity == Severity::Error)
}

/// A span covering nothing, for diagnostics that have no better location yet.
pub fn nowhere() -> Span {
    Span::at(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parse_error_reports_a_real_location() {
        let src = concat!(
            "process Good (a: buffer in u1)\n",
            "  return\n",
            "\n",
            "gibberish Bad\n",
        );
        let map = SourceMap::new("t.ddl", src);
        let diags = parse_source(&map).err().expect("should fail to parse");
        let text = map.render_all(&diags);

        assert!(text.contains("t.ddl:4:1"), "{}", text);
        assert!(text.contains("unexpected `gibberish`"), "{}", text);
        assert!(text.contains("gibberish Bad"), "{}", text);
    }

    #[test]
    fn tabs_are_rejected_with_an_explanation() {
        let src = "process Name (a: buffer in u1)\n\treturn\n";
        let map = SourceMap::new("t.ddl", src);
        let diags = parse_source(&map).err().expect("tabs should be rejected");
        let text = map.render_all(&diags);

        assert!(text.contains("tab character in source"), "{}", text);
        assert!(text.contains("t.ddl:2:1"), "{}", text);
        assert!(text.contains("spaces only"), "{}", text);
    }

    #[test]
    fn a_clean_source_parses() {
        let src = concat!(
            "-- a comment, which desc.md uses everywhere\n",
            "process Name (a: buffer in u1)\n",
            "  let x = a\n",
            "  return\n",
        );
        let map = SourceMap::new("t.ddl", src);
        let parsed = parse_source(&map).expect("should parse");
        assert_eq!(parsed.decls.len(), 1);
    }
}

#[cfg(test)]
mod emit_tests {
    use super::*;

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

    #[test]
    fn a_trivial_function_becomes_a_module() {
        let v = compile(concat!(
            "fun adder (a: u8, b: u8, sum: out u8)\n",
            "  sum = a + b\n",
        ));
        assert!(v.contains("module adder ("), "{}", v);
        assert!(v.contains("input  [7:0] a,"), "{}", v);
        assert!(v.contains("output [7:0] sum"), "{}", v);
        assert!(v.contains("a + b"), "{}", v);
        assert!(v.contains("assign sum ="), "{}", v);
        assert!(v.contains("endmodule"), "{}", v);
    }

    #[test]
    fn the_banner_says_how_to_regenerate() {
        let v = compile("fun f (a: u1, o: out u1)\n  o = a\n");
        assert!(v.contains("GENERATED FILE -- DO NOT EDIT BY HAND"), "{}", v);
        assert!(v.contains("Regenerate with:"), "{}", v);
    }

    // The three constructs GowinSynthesis dies on with an empty log. None of
    // them can come out of this backend, and these assertions are what keeps
    // that true.
    #[test]
    fn never_emits_clog2_or_width_casts_or_calls() {
        let v = compile(concat!(
            "fun widths (a: u8, b: u32, o: out u32, p: out i40)\n",
            "  o = @zext(a, 32) + b\n",
            "  p = @sext(@signed(b), 40)\n",
        ));
        // The banner names all three constructs, so check the code only.
        let code: String = v
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!code.contains("$clog2"), "{}", v);
        // A width cast is `N'(expr)`; a sized literal `N'd5` is fine.
        assert!(!code.contains("'("), "{}", v);
        // No functions at all, so none can appear in a continuous assign.
        assert!(!code.contains("function"), "{}", v);
        // Extension is a concatenation, as k2g_cdc_fifo.sv:91 writes by hand.
        assert!(v.contains("{{24{1'b0}}"), "{}", v);
    }

    #[test]
    fn signedness_lives_on_the_declaration() {
        let v = compile(concat!(
            "fun sh (v: u32, amt: u5, o: out i32)\n",
            "  o = @signed(v) >> amt\n",
        ));
        assert!(v.contains("wire signed [31:0]"), "{}", v);
        // Arithmetic shift, chosen by the left operand's signedness.
        assert!(v.contains(">>>"), "{}", v);
    }

    #[test]
    fn unsigned_shift_stays_logical() {
        let v = compile(concat!(
            "fun sh (v: u32, amt: u5, o: out u32)\n",
            "  o = v >> amt\n",
        ));
        assert!(v.contains(">>"), "{}", v);
        assert!(!v.contains(">>>"), "{}", v);
    }

    #[test]
    fn if_else_becomes_a_mux() {
        let v = compile(concat!(
            "fun pick (c: u1, a: u8, b: u8, o: out u8)\n",
            "  if c then\n",
            "    o = a\n",
            "  else\n",
            "    o = b\n",
        ));
        assert!(v.contains(" ? ") && v.contains(" : "), "{}", v);
    }

    #[test]
    fn mixed_widths_are_rejected_with_the_cast_to_write() {
        let text = compile_err(concat!(
            "fun bad (a: u32, b: u5, o: out u32)\n",
            "  o = a + b\n",
        ));
        assert!(text.contains("width mismatch"), "{}", text);
        assert!(text.contains("@zext(x, 32)"), "{}", text);
    }

    #[test]
    fn an_unassigned_output_is_an_error() {
        let text = compile_err("fun bad (a: u8, o: out u8)\n  let x = a\n");
        assert!(text.contains("`o` is never assigned"), "{}", text);
    }

    #[test]
    fn assigning_on_only_one_branch_is_an_error() {
        // Combinational logic has no memory, so this would be a latch.
        let text = compile_err(concat!(
            "fun bad (c: u1, a: u8, o: out u8)\n",
            "  if c then\n",
            "    o = a\n",
        ));
        assert!(text.contains("only one branch"), "{}", text);
    }

    #[test]
    fn unsized_literals_adopt_the_other_operand_width() {
        let v = compile(concat!(
            "fun inc (a: u32, o: out u32)\n",
            "  o = a - 1\n",
        ));
        assert!(v.contains("32'd1"), "{}", v);
    }

    #[test]
    fn a_literal_that_does_not_fit_is_rejected() {
        let text = compile_err("fun bad (a: u8, o: out u8)\n  o = a + 300\n");
        assert!(text.contains("width mismatch") || text.contains("does not fit"), "{}", text);
    }

    #[test]
    fn multiplication_widens_to_the_full_product() {
        let v = compile(concat!(
            "fun mul (a: u16, b: u16, o: out u32)\n",
            "  o = a * b\n",
        ));
        assert!(v.contains("output [31:0] o"), "{}", v);
        // Both operands are widened to the product width first, so no bits are
        // lost -- Verilog would otherwise truncate to the operand width.
        assert_eq!(v.matches("{{16{1'b0}}").count(), 2, "{}", v);
        assert!(v.contains(" * "), "{}", v);
    }

    #[test]
    fn bit_slices_and_concat() {
        let v = compile(concat!(
            "fun bits (a: u32, o: out u8, p: out u32)\n",
            "  o = a[15..8]\n",
            "  p = {a[15..0], a[31..16]}\n",
        ));
        assert!(v.contains("[15:8]"), "{}", v);
        assert!(v.contains("{"), "{}", v);
    }

    #[test]
    fn a_computed_index_becomes_a_part_select() {
        let v = compile(concat!(
            "fun pick (a: u32, i: u5, o: out u1)\n",
            "  o = a[i]\n",
        ));
        assert!(v.contains("+: 1"), "{}", v);
    }
}
