// Compilation driver: source text in, diagnostics or declarations out.
//
// This exists so that every stage has one place to report from. Before it,
// `parse_top_level` returned a bare `*const u8` and the only caller rendered
// it as `panic!("{}", *err as char)` -- the entire user-facing parse error was
// one character.

use std::collections::HashMap;

use crate::diag::{Diag, DiagSink, Severity, SourceMap, Span};
use crate::ir::{lower_function, lower_process};
use crate::lex::{find_tab, parse_top_level, TopLevelDecl};
use crate::parse::{
    FunctionDecl, anumspan_to_str, resolve_precedence_for_enum, resolve_precedence_for_function,
    resolve_precedence_for_process, resolve_precedence_for_sequence,
    resolve_precedence_for_struct,
};
use crate::symbols;
use crate::verilog::{EmitOptions, emit_banner, emit_module};

pub struct Parsed {
    pub decls: Vec<TopLevelDecl>,
}

/// Parses one source file.
///
/// The `SourceMap` must outlive the returned declarations: the AST holds raw
/// pointers into its text. That is enforced here by the borrow on `map`, which
/// is the closest thing to a lifetime the pointer-based AST can currently
/// carry.
pub fn parse_source(map: &SourceMap) -> Result<Parsed, Vec<Diag>> {
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
        Ok(decls) => Ok(Parsed { decls }),
        Err(at) => {
            let span = map.span_at_ptr(at);
            let offset = map.offset_of(at);
            let msg = if offset >= map.len() {
                "unexpected end of input".to_string()
            } else {
                let rest = &map.text()[offset as usize..];
                let word: String = rest
                    .chars()
                    .take_while(|c| !c.is_whitespace())
                    .take(24)
                    .collect();
                if word.is_empty() {
                    "unexpected input here".to_string()
                } else {
                    format!("unexpected `{}`", word)
                }
            };
            Err(vec![Diag::error(span, msg).with_note(
                "expected a top-level `process`, `sequence`, `fun`, `struct` or `enum` declaration",
            )])
        }
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
}

pub fn compile_to_verilog(map: &SourceMap, opts: &EmitOptions) -> Result<String, Vec<Diag>> {
    compile(map, opts, Emit::Verilog)
}

pub fn compile(map: &SourceMap, opts: &EmitOptions, emit: Emit) -> Result<String, Vec<Diag>> {
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
    let bodies: HashMap<String, &FunctionDecl> = funcs
        .iter()
        .map(|f| (anumspan_to_str(&f.name).to_string(), f))
        .collect();

    let dumping_ir = emit == Emit::Ir;
    let mut out = if dumping_ir { String::new() } else { emit_banner(opts) };
    let mut emitted = 0usize;
    let mut render = |out: &mut String, module: &crate::ir::Module| {
        out.push('\n');
        if dumping_ir {
            out.push_str(&crate::ir::render_module(module));
        } else {
            out.push_str(&emit_module(module, opts));
        }
    };
    for func in &funcs {
        if let Some(module) = lower_function(map, &syms, &bodies, func, &mut sink) {
            render(&mut out, &module);
            emitted += 1;
        }
    }
    for seq in &seqs {
        if let Some(module) = crate::ir_pipe::lower_sequence(map, &syms, &bodies, seq, &mut sink) {
            render(&mut out, &module);
            emitted += 1;
        }
    }
    for proc in &procs {
        if let Some(module) = lower_process(map, &syms, &bodies, proc, &mut sink) {
            render(&mut out, &module);
            emitted += 1;
        }
    }

    if sink.has_errors() {
        return Err(sink.into_diags());
    }
    if emitted == 0 {
        return Err(vec![Diag::error_no_span("nothing to emit: no `fun`, `process` or `sequence` declarations found")]);
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
            "process Good (a: stream in i1)\n",
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
        let src = "process Name (a: stream in i1)\n\treturn\n";
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
            "process Name (a: stream in i1)\n",
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
            "fun adder (a: i8, b: i8, sum: out i8)\n",
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
        let v = compile("fun f (a: i1, o: out i1)\n  o = a\n");
        assert!(v.contains("GENERATED FILE -- DO NOT EDIT BY HAND"), "{}", v);
        assert!(v.contains("Regenerate with:"), "{}", v);
    }

    // The three constructs GowinSynthesis dies on with an empty log. None of
    // them can come out of this backend, and these assertions are what keeps
    // that true.
    #[test]
    fn never_emits_clog2_or_width_casts_or_calls() {
        let v = compile(concat!(
            "fun widths (a: i8, b: i32, o: out i32, p: out s40)\n",
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
            "fun sh (v: i32, amt: i5, o: out s32)\n",
            "  o = @signed(v) >> amt\n",
        ));
        assert!(v.contains("wire signed [31:0]"), "{}", v);
        // Arithmetic shift, chosen by the left operand's signedness.
        assert!(v.contains(">>>"), "{}", v);
    }

    #[test]
    fn unsigned_shift_stays_logical() {
        let v = compile(concat!(
            "fun sh (v: i32, amt: i5, o: out i32)\n",
            "  o = v >> amt\n",
        ));
        assert!(v.contains(">>"), "{}", v);
        assert!(!v.contains(">>>"), "{}", v);
    }

    #[test]
    fn if_else_becomes_a_mux() {
        let v = compile(concat!(
            "fun pick (c: i1, a: i8, b: i8, o: out i8)\n",
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
            "fun bad (a: i32, b: i5, o: out i32)\n",
            "  o = a + b\n",
        ));
        assert!(text.contains("width mismatch"), "{}", text);
        assert!(text.contains("@zext(x, 32)"), "{}", text);
    }

    #[test]
    fn an_unassigned_output_is_an_error() {
        let text = compile_err("fun bad (a: i8, o: out i8)\n  let x = a\n");
        assert!(text.contains("`o` is never assigned"), "{}", text);
    }

    #[test]
    fn assigning_on_only_one_branch_is_an_error() {
        // Combinational logic has no memory, so this would be a latch.
        let text = compile_err(concat!(
            "fun bad (c: i1, a: i8, o: out i8)\n",
            "  if c then\n",
            "    o = a\n",
        ));
        assert!(text.contains("only one branch"), "{}", text);
    }

    #[test]
    fn unsized_literals_adopt_the_other_operand_width() {
        let v = compile(concat!(
            "fun inc (a: i32, o: out i32)\n",
            "  o = a - 1\n",
        ));
        assert!(v.contains("32'd1"), "{}", v);
    }

    #[test]
    fn a_literal_that_does_not_fit_is_rejected() {
        let text = compile_err("fun bad (a: i8, o: out i8)\n  o = a + 300\n");
        assert!(text.contains("width mismatch") || text.contains("does not fit"), "{}", text);
    }

    #[test]
    fn multiplication_widens_to_the_full_product() {
        let v = compile(concat!(
            "fun mul (a: i16, b: i16, o: out i32)\n",
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
            "fun bits (a: i32, o: out i8, p: out i32)\n",
            "  o = a[15..8]\n",
            "  p = @concat(a[15..0], a[31..16])\n",
        ));
        assert!(v.contains("[15:8]"), "{}", v);
        assert!(v.contains("{"), "{}", v);
    }

    #[test]
    fn a_computed_index_becomes_a_part_select() {
        let v = compile(concat!(
            "fun pick (a: i32, i: i5, o: out i1)\n",
            "  o = a[i]\n",
        ));
        assert!(v.contains("+: 1"), "{}", v);
    }

    #[test]
    fn a_stream_pipe_says_what_is_missing_rather_than_failing_silently() {
        // `buffer` pipes work; `stream` overwrites its oldest value and needs
        // different generated logic.
        let text = compile_err("process P (a: stream in i1, o: out i1)\n  o = 1'b0\n");
        assert!(text.contains("`stream` pipes are not supported yet"), "{}", text);
    }
}
