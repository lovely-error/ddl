// Finding the files a compilation is made of.
//
// DDL has no namespaces and no separate compilation: every declaration in the
// program is visible to every other one, and the symbol table is built from
// all of them at once. `import` therefore says nothing about visibility. It
// says only "this file is part of the program too", which is exactly the job
// that was being done by
//
//     cat k2g_pkg.ddl k2g_types.ddl k2g_shift.ddl > src.ddl
//
// in examples/verify.sh. That worked, and cost two things worth having: a
// diagnostic could only ever name the concatenation, and a module could not
// say for itself what it depends on.
//
// So an import is resolved here, before parsing, and the files are handed to
// the `SourceMap` in dependency order. The import line itself is replaced by a
// comment of exactly the same length rather than removed -- the parser never
// sees it, and every byte offset after it is still the offset the author would
// count. The file as written goes along too, so a diagnostic on an import line
// quotes the import instead of the comment standing in for it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::diag::{Diag, SourceMap};

/// Where a file's text came from, and what it looks like after import lines
/// have been blanked out.
struct Loaded {
    path: String,
    text: String,
    /// The file as it is on disk. Kept so a diagnostic on an import line
    /// quotes the import rather than the comment it was rewritten into.
    original: String,
}

/// An import that could not be resolved, kept until the `SourceMap` exists so
/// the diagnostic can point at the line that asked for it.
struct BadImport {
    /// Path of the file containing the import line.
    in_file: String,
    line_no: u32,
    /// 1-based column of the quoted path.
    col: u32,
    len: u32,
    msg: String,
}

pub struct Loader {
    /// Extra directories to resolve an import against, after the directory of
    /// the importing file. `-I` on the command line.
    search: Vec<PathBuf>,
    seen: HashSet<PathBuf>,
    files: Vec<Loaded>,
    bad: Vec<BadImport>,
}

impl Loader {
    pub fn new(search: Vec<PathBuf>) -> Self {
        Loader { search, seen: HashSet::new(), files: Vec::new(), bad: Vec::new() }
    }

    /// Reads `path` and everything it imports, transitively.
    ///
    /// Call once per file named on the command line; a file reached twice --
    /// by two importers, or by a cycle -- is included once, at the position of
    /// its first appearance.
    pub fn add_root(&mut self, path: &Path) -> Result<(), String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.load(path, text);
                Ok(())
            }
            Err(e) => Err(format!("cannot read `{}`: {}", path.display(), e)),
        }
    }

    fn load(&mut self, path: &Path, text: String) {
        if !self.seen.insert(key(path)) {
            return;
        }
        // Reserve this file's slot before descending, then fill it in: what an
        // import means is "this comes first", and a file that imports another
        // has to appear after it.
        let slot = self.files.len();
        self.files.push(Loaded {
            path: display(path),
            text: String::new(),
            original: text.clone(),
        });

        let (blanked, imports) = strip_imports(&text);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for imp in imports {
            let target = match &imp.target {
                Ok(t) => t.clone(),
                Err(why) => {
                    self.bad.push(BadImport {
                        in_file: display(path),
                        line_no: imp.line_no,
                        col: imp.col,
                        len: imp.len,
                        msg: why.to_string(),
                    });
                    continue;
                }
            };
            match self.resolve(&dir, &target) {
                Some(found) => match std::fs::read_to_string(&found) {
                    Ok(body) => self.load(&found, body),
                    Err(e) => self.bad.push(BadImport {
                        in_file: display(path),
                        line_no: imp.line_no,
                        col: imp.col,
                        len: imp.len,
                        msg: format!("cannot read `{}`: {}", found.display(), e),
                    }),
                },
                None => self.bad.push(BadImport {
                    in_file: display(path),
                    line_no: imp.line_no,
                    col: imp.col,
                    len: imp.len,
                    msg: format!("cannot find `{}`", target),
                }),
            }
        }

        // Imported files were pushed after the slot; move this file's text
        // back behind them by rotating the slot to the end of what it pulled
        // in. Cheap, and it keeps `files` in the order a reader would expect.
        let end = self.files.len();
        self.files[slot].text = blanked;
        self.files[slot..end].rotate_left(1);
    }

    fn resolve(&self, importer_dir: &Path, target: &str) -> Option<PathBuf> {
        let direct = importer_dir.join(target);
        if direct.is_file() {
            return Some(direct);
        }
        for dir in &self.search {
            let candidate = dir.join(target);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// The buffer, plus any import that could not be resolved.
    ///
    /// Both are returned: an unresolved import is an error, but the files that
    /// did load still carry the spans the diagnostic needs.
    pub fn finish(self) -> (SourceMap, Vec<Diag>) {
        let map = SourceMap::from_rewritten(
            self.files
                .into_iter()
                .map(|f| (f.path, f.text, Some(f.original)))
                .collect(),
        );
        let diags = self
            .bad
            .iter()
            .map(|b| {
                let note = "an import is resolved against the importing file's directory, \
                            then against each `-I` directory";
                // No span rather than a made-up one. `Span::at(0)` renders as
                // line 1 of the first file, which is a place a reader will go
                // and look at, and every byte of it will be innocent. This
                // should be unreachable -- the file and the line both come
                // from our own scan of it -- and if it ever is reached, saying
                // nothing is the honest answer.
                match map.span_in_file(&b.in_file, b.line_no, b.col, b.len) {
                    Some(span) => Diag::error(span, b.msg.clone()).with_note(note),
                    None => Diag::error_no_span(format!("{}: {}", b.in_file, b.msg))
                        .with_note(note),
                }
            })
            .collect();
        (map, diags)
    }
}

/// Loads `roots` and their imports into one buffer.
pub fn load_program(roots: &[String], search: Vec<PathBuf>) -> Result<(SourceMap, Vec<Diag>), String> {
    let mut loader = Loader::new(search);
    for root in roots {
        loader.add_root(Path::new(root))?;
    }
    Ok(loader.finish())
}

/// A line that begins with the `import` keyword: either the path it names, or
/// why it is not an import after all.
struct Import {
    target: Result<String, &'static str>,
    line_no: u32,
    col: u32,
    len: u32,
}

/// The text as the PARSER sees it: import lines blanked, nothing else changed.
///
/// For anything that has to parse one file on its own -- `ddl fmt` verifying
/// it did not change the syntax tree, say. Without this the parser meets a
/// line beginning `import` and has no such keyword, so a file that compiles
/// perfectly well reads as broken.
pub fn as_the_parser_sees_it(text: &str) -> String {
    strip_imports(text).0
}

/// Replaces every top-level `import "path"` line with a comment of the same
/// length, and reports what was imported.
///
/// Same length is the whole trick: every byte after the line keeps the offset
/// it had in the file the author edited, so spans need no adjustment and the
/// `SourceMap` needs to know nothing about imports.
fn strip_imports(text: &str) -> (String, Vec<Import>) {
    let mut out = String::with_capacity(text.len());
    let mut imports = Vec::new();

    for (ix, line) in text.split_inclusive('\n').enumerate() {
        let body = line.trim_end_matches(['\n', '\r']);
        let tail = &line[body.len()..];
        // Only at the left margin: indentation means the line belongs to a
        // declaration's block, where `import` is not a keyword at all.
        let at_margin = !body.starts_with(' ');
        match parse_import(body) {
            Ok((target, at)) if at_margin => {
                imports.push(Import {
                    target: Ok(target),
                    line_no: (ix + 1) as u32,
                    col: (at + 1) as u32,
                    len: (body.len() - at) as u32,
                });
                blank(&mut out, body.len());
                out.push_str(tail);
            }
            Err(Some(why)) if at_margin => {
                imports.push(Import {
                    target: Err(why),
                    line_no: (ix + 1) as u32,
                    col: 1,
                    len: body.len() as u32,
                });
                blank(&mut out, body.len());
                out.push_str(tail);
            }
            _ => out.push_str(line),
        }
    }
    (out, imports)
}

/// A comment exactly `len` bytes wide, which is what makes blanking a line
/// invisible to every offset after it.
fn blank(out: &mut String, len: usize) {
    out.push_str("--");
    for _ in 2..len {
        out.push(' ');
    }
}

/// `import "some/path.ddl"`, optionally followed by a comment, returning the
/// path and the column of its opening quote.
///
/// `Err` is for a line that opens with the keyword and then is not one: that
/// is a typo in an import, and saying so beats letting the parser report
/// `unexpected \`import\`` about a keyword it has never heard of.
fn parse_import(line: &str) -> Result<(String, usize), Option<&'static str>> {
    let rest = match line.strip_prefix("import") {
        Some(rest) if rest.starts_with(' ') => rest,
        // `imported` is an ordinary identifier, and there is no bare `import`
        // form to mistype.
        _ => return Err(None),
    };
    let quote = match rest.find('"') {
        Some(at) if rest[..at].chars().all(|c| c == ' ') => at,
        _ => return Err(Some("expected a quoted path, as in `import \"types.ddl\"`")),
    };
    let after = &rest[quote + 1..];
    let close = match after.find('"') {
        Some(at) => at,
        None => return Err(Some("this path is missing its closing quote")),
    };
    let tail = after[close + 1..].trim();
    if !tail.is_empty() && !tail.starts_with("--") {
        return Err(Some("an import line holds one path and nothing else but a comment"));
    }
    Ok((after[..close].to_string(), "import".len() + quote))
}

fn key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn display(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_import_line_becomes_a_comment_of_the_same_length() {
        let src = "import \"types.ddl\"\nfun f (o: out u1)\n  o = 1'd1\n";
        let (out, imports) = strip_imports(src);

        assert_eq!(out.len(), src.len(), "{:?}", out);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target.as_deref(), Ok("types.ddl"));
        assert_eq!(imports[0].line_no, 1);
        // Every later line starts where it started before.
        assert_eq!(out.find("fun f"), src.find("fun f"));
        assert!(out.starts_with("--"), "{:?}", out);
        assert!(out.lines().next().unwrap().trim_end() == "--", "{:?}", out);
    }

    #[test]
    fn an_indented_import_is_left_alone() {
        // Inside a block, `import` is an ordinary identifier and blanking the
        // line would silently delete code.
        let src = "process p (a: buffer in u1)\n  import \"x.ddl\"\n";
        let (out, imports) = strip_imports(src);
        assert_eq!(out, src);
        assert!(imports.is_empty());
    }

    #[test]
    fn a_trailing_comment_is_allowed() {
        let src = "import \"k2g_pkg.ddl\"   -- generated from the emulator\n";
        let (out, imports) = strip_imports(src);
        assert_eq!(out.len(), src.len());
        assert_eq!(imports[0].target.as_deref(), Ok("k2g_pkg.ddl"));
    }

    #[test]
    fn a_word_that_merely_starts_with_import_is_not_one() {
        for line in ["importfoo \"a.ddl\"\n", "imported = 1\n"] {
            let (out, imports) = strip_imports(line);
            assert_eq!(out, line, "{:?}", line);
            assert!(imports.is_empty(), "{:?}", line);
        }
    }

    #[test]
    fn a_mistyped_import_says_so_rather_than_reaching_the_parser() {
        // Otherwise the parser reports ``unexpected `import` `` about a
        // keyword it does not have, which sends the reader looking for the
        // wrong bug entirely.
        for line in ["import a.ddl\n", "import \"a.ddl\n", "import \"a.ddl\" extra\n"] {
            let (out, imports) = strip_imports(line);
            assert_eq!(out.len(), line.len(), "{:?}", line);
            assert_eq!(imports.len(), 1, "{:?}", line);
            assert!(imports[0].target.is_err(), "{:?}", line);
        }
    }

    #[test]
    fn crlf_survives_blanking() {
        let src = "import \"t.ddl\"\r\nfun f (o: out u1)\r\n";
        let (out, imports) = strip_imports(src);
        assert_eq!(out.len(), src.len());
        assert_eq!(imports.len(), 1);
        assert!(out.contains("\r\n"), "{:?}", out);
    }
}
