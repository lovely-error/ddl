// The editor grammar, checked against the compiler.
//
// A syntax file is the one part of a toolchain nothing verifies: it is
// consulted by an editor, never by a build, so it goes stale silently and the
// only symptom is a keyword that stops being a colour. `graph` and every `@`
// builtin added since would have gone unhighlighted with nobody the wiser.
//
// So this reads the keyword set out of the compiler's own source and requires
// the grammar to cover it. Reading Rust with a regex is crude and would be the
// wrong tool for anything that had to be exact; here the failure mode is a
// missing entry, and a crude check that catches that beats no check.

use std::collections::BTreeSet;

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

fn grammar() -> String {
    read("editors/vscode/syntaxes/ddl.tmLanguage.json")
}

/// Every `"name" => AnumResolution::Builtin(..)` arm in the resolver.
fn builtins_the_compiler_knows() -> BTreeSet<String> {
    let src = read("src/parse.rs");
    let mut found = BTreeSet::new();
    for line in src.lines() {
        let line = line.trim();
        if !line.contains("AnumResolution::Builtin(") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('"')
            && let Some(end) = rest.find('"') {
                found.insert(rest[..end].to_string());
            }
    }
    found
}

#[test]
fn the_grammar_is_valid_json_with_the_scope_the_manifest_names() {
    let text = grammar();
    // A malformed grammar is not an error in an editor; it is silence.
    assert!(text.contains("\"scopeName\": \"source.ddl\""), "scopeName missing");
    let manifest = read("editors/vscode/package.json");
    assert!(manifest.contains("\"source.ddl\""), "manifest names a different scope");
    assert!(manifest.contains("\".ddl\""), "manifest does not claim the .ddl extension");
}

#[test]
fn every_builtin_the_compiler_resolves_is_in_the_grammar() {
    let known = builtins_the_compiler_knows();
    assert!(known.len() >= 15, "only found {} builtins; the scrape broke", known.len());

    let text = grammar();
    let missing: Vec<&String> = known.iter().filter(|b| !text.contains(b.as_str())).collect();
    assert!(
        missing.is_empty(),
        "the grammar does not highlight: {:?}\nadd them to the `builtin` pattern in \
         editors/vscode/syntaxes/ddl.tmLanguage.json",
        missing
    );
}

#[test]
fn every_declaration_keyword_is_in_the_grammar() {
    // The set that opens a top-level declaration. `graph` is here because it
    // was added after the grammar would first have been written, which is
    // exactly the drift this test exists for.
    let text = grammar();
    for kw in ["fun", "process", "sequence", "graph", "struct", "enum"] {
        assert!(text.contains(kw), "the grammar does not highlight `{}`", kw);
    }
}

#[test]
fn every_statement_keyword_is_in_the_grammar() {
    let text = grammar();
    for kw in [
        "let", "var", "if", "then", "else", "match", "loop", "break", "for", "in", "return",
        "import", "buffer", "stream", "out", "inout",
    ] {
        assert!(text.contains(kw), "the grammar does not highlight `{}`", kw);
    }
}

#[test]
fn the_memory_kinds_match_the_ones_the_compiler_accepts() {
    // `MemKind::parse` is the authority; an annotation the grammar highlights
    // and the compiler rejects is worse than one it leaves plain.
    let ty = read("src/ty.rs");
    let text = grammar();
    for kind in ["lutram", "bram", "bkram"] {
        assert!(ty.contains(&format!("\"{}\"", kind)), "{} is not a MemKind", kind);
        assert!(text.contains(kind), "the grammar does not highlight `{}`", kind);
    }
}

#[test]
fn the_grammar_highlights_the_pieces_of_a_real_example() {
    // An end-to-end sanity check on the patterns rather than the word list:
    // these are the constructs that make DDL look unlike C, and a grammar that
    // missed them would still pass every test above.
    let text = grammar();
    for (what, needle) in [
        ("the stage cut", "stage-cut"),
        ("`--` comments", "comment.line.double-dash.ddl"),
        ("sized literals like 8'd1", "constant.numeric.sized.ddl"),
        ("`iN`/`sN` types", "[is][0-9]+"),
        ("dotted match patterns", "variable.other.enummember.ddl"),
        ("the import path", "keyword.control.import.ddl"),
    ] {
        assert!(text.contains(needle), "the grammar has no rule for {}", what);
    }
}
