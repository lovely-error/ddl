// Source locations and diagnostics.
//
// The parser addresses source text by raw pointer -- an `AlphanumSpan` is a
// `*const u8` and a length, with no offset and no lifetime. Rather than widen
// that type and touch every construction site in lex.rs, this module keeps the
// base pointer of the buffer alongside the text, which is enough to turn any
// pointer the parser hands back into a byte offset and then a line and column.
//
// Multi-file compilation does not change that. Several files become ONE
// buffer, concatenated, and the map records where each one begins; an offset
// is attributed to a file by a search over those starts. So `AlphanumSpan`
// still needs no file id, the parser still sees one contiguous buffer, and a
// diagnostic still names the file and line the author wrote.

use crate::lex::AlphanumSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn new(lo: u32, hi: u32) -> Self {
        Span { lo, hi: if hi < lo { lo } else { hi } }
    }

    pub fn at(offset: u32) -> Self {
        Span { lo: offset, hi: offset }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diag {
    pub severity: Severity,
    pub span: Option<Span>,
    pub msg: String,
    /// Optional second line printed under the snippet, for the "and here is
    /// why" half that width-mismatch and scope errors need.
    pub note: Option<String>,
}

impl Diag {
    pub fn error(span: Span, msg: impl Into<String>) -> Self {
        Diag { severity: Severity::Error, span: Some(span), msg: msg.into(), note: None }
    }

    pub fn error_no_span(msg: impl Into<String>) -> Self {
        Diag { severity: Severity::Error, span: None, msg: msg.into(), note: None }
    }

    pub fn warning(span: Span, msg: impl Into<String>) -> Self {
        Diag { severity: Severity::Warning, span: Some(span), msg: msg.into(), note: None }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// One input file's place in the concatenated buffer.
struct FileInfo {
    path: String,
    /// Byte offset where this file's text begins.
    start: u32,
    /// Index into `line_starts` of this file's first line, so a line number
    /// can be reported relative to the file rather than to the buffer.
    first_line: usize,
}

pub struct SourceMap {
    text: String,
    /// What the author wrote, when that is not what the parser reads.
    ///
    /// Import resolution replaces each `import` line with a comment of exactly
    /// the same length, so the two buffers agree byte for byte on every
    /// offset -- but a snippet quoting the comment would show the reader a
    /// line they never typed. `None` when nothing was rewritten.
    display: Option<String>,
    /// Byte offset of the first character of each line.
    line_starts: Vec<u32>,
    /// In buffer order, never empty.
    files: Vec<FileInfo>,
}

impl SourceMap {
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        Self::from_files(vec![(path.into(), text.into())])
    }

    /// Several files as one buffer, in the order given.
    ///
    /// A file that does not end in a newline gets one, so the next file starts
    /// on its own line: DDL delimits blocks by indentation, and a last line
    /// running into the next file's first would change which block a
    /// declaration belongs to rather than failing to parse.
    pub fn from_files(files: Vec<(String, String)>) -> Self {
        Self::from_rewritten(files.into_iter().map(|(p, t)| (p, t, None)).collect())
    }

    /// As `from_files`, but each file may carry the text the author wrote
    /// alongside the text the parser should read.
    ///
    /// The two must have the same length; a rewrite that moved bytes would put
    /// every span after it on the wrong column, which is worse than the
    /// problem this solves. Mismatched lengths fall back to the parser's text,
    /// so the failure is a plain snippet rather than a wrong one.
    pub fn from_rewritten(files: Vec<(String, String, Option<String>)>) -> Self {
        let files = if files.is_empty() {
            vec![(String::from("<empty>"), String::new(), None)]
        } else {
            files
        };

        let mut text = String::new();
        let mut display = String::new();
        let mut any_rewritten = false;
        let mut line_starts = vec![0u32];
        let mut infos = Vec::with_capacity(files.len());
        for (path, body, original) in files {
            infos.push(FileInfo {
                path,
                start: text.len() as u32,
                first_line: line_starts.len() - 1,
            });
            let base = text.len();
            text.push_str(&body);
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            for ix in base..text.len() {
                if text.as_bytes()[ix] == b'\n' {
                    line_starts.push((ix + 1) as u32);
                }
            }

            match original {
                Some(original) if original.len() == body.len() => {
                    any_rewritten = true;
                    display.push_str(&original);
                }
                _ => display.push_str(&body),
            }
            // Whatever went in, both buffers end at the same offset.
            while display.len() < text.len() {
                display.push('\n');
            }
        }

        SourceMap {
            text,
            display: any_rewritten.then_some(display),
            line_starts,
            files: infos,
        }
    }

    /// The text a snippet should quote.
    fn shown(&self) -> &str {
        self.display.as_deref().unwrap_or(&self.text)
    }

    /// The first input file, which is the one named on the command line.
    pub fn path(&self) -> &str {
        &self.files[0].path
    }

    /// Index of the file that owns a byte offset.
    fn file_at(&self, offset: u32) -> usize {
        match self.files.binary_search_by_key(&offset, |f| f.start) {
            Ok(ix) => ix,
            Err(ix) => ix - 1,
        }
    }

    /// The path a diagnostic at this offset should name.
    pub fn path_at(&self, offset: u32) -> &str {
        &self.files[self.file_at(offset)].path
    }

    /// A span from a file-relative position, which is what a stage running
    /// before the parser has: import resolution knows "line 3 of types.ddl"
    /// and nothing about buffer offsets.
    ///
    /// `None` if no such file or line is in the buffer.
    pub fn span_in_file(&self, path: &str, line_no: u32, col: u32, len: u32) -> Option<Span> {
        let file = self.files.iter().find(|f| f.path == path)?;
        let ix = file.first_line + (line_no as usize).checked_sub(1)?;
        let start = *self.line_starts.get(ix)?;
        let lo = start + col.saturating_sub(1);
        Some(Span::new(lo, lo + len))
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> u32 {
        self.text.len() as u32
    }

    pub fn base_ptr(&self) -> *const u8 {
        self.text.as_bytes().as_ptr()
    }

    pub fn end_ptr(&self) -> *const u8 {
        unsafe { self.base_ptr().add(self.text.len()) }
    }

    /// Byte offset of a pointer the parser handed back. Pointers outside the
    /// buffer clamp to its end rather than panicking -- a diagnostic pointing
    /// at the wrong place is still better than losing the diagnostic.
    pub fn offset_of(&self, ptr: *const u8) -> u32 {
        let base = self.base_ptr() as usize;
        let here = ptr as usize;
        if here < base {
            return 0;
        }
        let off = here - base;
        if off > self.text.len() { self.len() } else { off as u32 }
    }

    pub fn span_of(&self, span: &AlphanumSpan) -> Span {
        let lo = self.offset_of(span.byte_ptr);
        Span::new(lo, lo + span.len)
    }

    pub fn span_at_ptr(&self, ptr: *const u8) -> Span {
        Span::at(self.offset_of(ptr))
    }

    /// Index into `line_starts` of the line containing an offset.
    fn line_ix(&self, offset: u32) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(ix) => ix,
            Err(ix) => ix - 1,
        }
    }

    /// 1-based line and column, the line counted from the start of the file
    /// that owns the offset rather than from the start of the buffer. Column
    /// counts bytes, which equals characters because DDL identifiers and
    /// operators are ASCII.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let line_ix = self.line_ix(offset);
        let first = self.files[self.file_at(offset)].first_line;
        let col = offset - self.line_starts[line_ix];
        ((line_ix - first + 1) as u32, col + 1)
    }

    fn line_text_at(&self, ix: usize) -> &str {
        let shown = self.shown();
        let start = self.line_starts[ix] as usize;
        let end = self
            .line_starts
            .get(ix + 1)
            .map(|e| *e as usize)
            .unwrap_or(shown.len());
        shown[start..end].trim_end_matches(['\n', '\r'])
    }

    /// Renders one diagnostic in the usual caret style:
    ///
    /// ```text
    /// error: `foo` is not defined
    ///  --> shift.ddl:4:11
    ///   |
    /// 4 |   let y = foo + 1
    ///   |           ^^^
    /// ```
    pub fn render(&self, diag: &Diag) -> String {
        let mut out = String::new();
        out.push_str(diag.severity.label());
        out.push_str(": ");
        out.push_str(&diag.msg);

        let span = match diag.span {
            Some(span) => span,
            None => {
                if let Some(note) = &diag.note {
                    out.push_str("\n  note: ");
                    out.push_str(note);
                }
                return out;
            }
        };

        let (line_no, col) = self.line_col(span.lo);
        let line_ix = self.line_ix(span.lo);
        let line = self.line_text_at(line_ix);
        let gutter_w = format!("{}", line_no).len();
        let pad = " ".repeat(gutter_w);

        out.push_str(&format!(
            "\n{} --> {}:{}:{}",
            pad,
            self.path_at(span.lo),
            line_no,
            col
        ));
        out.push_str(&format!("\n{} |", pad));
        out.push_str(&format!("\n{} | {}", line_no, line));

        // Clamp the underline to this line: a span may legitimately run past
        // the end of it (an unterminated construct), and an underline longer
        // than the text it marks reads as a rendering bug.
        let line_end = self.line_starts[line_ix] + line.len() as u32;
        let hi = if span.hi > line_end { line_end } else { span.hi };
        let width = if hi > span.lo { (hi - span.lo) as usize } else { 1 };

        out.push_str(&format!(
            "\n{} | {}{}",
            pad,
            " ".repeat((col - 1) as usize),
            "^".repeat(width)
        ));

        if let Some(note) = &diag.note {
            out.push_str(&format!("\n{} = note: {}", pad, note));
        }
        out
    }

    pub fn render_all(&self, diags: &[Diag]) -> String {
        diags
            .iter()
            .map(|d| self.render(d))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Collects diagnostics for one compilation.
///
/// Passes take `&mut DiagSink` where they used to take `&mut Vec<String>`, so
/// the threading through each pass is unchanged -- only the push sites move
/// from `format!` to a call that carries a location.
pub struct DiagSink<'a> {
    map: &'a SourceMap,
    diags: Vec<Diag>,
}

impl<'a> DiagSink<'a> {
    pub fn new(map: &'a SourceMap) -> Self {
        DiagSink { map, diags: Vec::new() }
    }

    pub fn map(&self) -> &'a SourceMap {
        self.map
    }

    pub fn err_at(&mut self, at: &AlphanumSpan, msg: impl Into<String>) {
        let span = self.map.span_of(at);
        self.diags.push(Diag::error(span, msg));
    }

    pub fn err_span(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::error(span, msg));
    }

    pub fn warn_at(&mut self, at: &AlphanumSpan, msg: impl Into<String>) {
        let span = self.map.span_of(at);
        self.diags.push(Diag::warning(span, msg));
    }

    pub fn push(&mut self, diag: Diag) {
        self.diags.push(diag);
    }

    pub fn has_errors(&self) -> bool {
        self.diags.iter().any(|d| d.severity == Severity::Error)
    }

    pub fn diags(&self) -> &[Diag] {
        &self.diags
    }

    pub fn into_diags(self) -> Vec<Diag> {
        self.diags
    }

    pub fn render(&self) -> String {
        self.map.render_all(&self.diags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> SourceMap {
        SourceMap::new(
            "shift.ddl",
            "fun f (a: i32)\n  let y = foo + 1\n  return y\n",
        )
    }

    #[test]
    fn line_col_is_one_based() {
        let m = map();
        assert_eq!(m.line_col(0), (1, 1));
        // "fun f (a: i32)\n" is 15 bytes, so offset 15 is the start of line 2.
        assert_eq!(m.line_col(15), (2, 1));
        assert_eq!(m.line_col(17), (2, 3));
    }

    #[test]
    fn offsets_round_trip_through_pointers() {
        let m = map();
        let ptr = unsafe { m.base_ptr().add(25) };
        assert_eq!(m.offset_of(ptr), 25);
        // Out-of-range pointers clamp instead of panicking.
        assert_eq!(m.offset_of(m.base_ptr().wrapping_sub(4096)), 0);
        assert_eq!(m.offset_of(m.end_ptr().wrapping_add(4096)), m.len());
    }

    #[test]
    fn render_points_at_the_right_column() {
        let m = map();
        // `foo` begins at column 11 of line 2.
        let lo = 15 + 10;
        let d = Diag::error(Span::new(lo, lo + 3), "`foo` is not defined");
        let text = m.render(&d);

        assert!(text.contains("error: `foo` is not defined"), "{}", text);
        assert!(text.contains("--> shift.ddl:2:11"), "{}", text);
        assert!(text.contains("2 |   let y = foo + 1"), "{}", text);
        // Ten spaces of indent then a three-wide caret run under `foo`.
        assert!(text.contains(&format!("|{}^^^", " ".repeat(11))), "{}", text);
    }

    #[test]
    fn underline_never_runs_past_the_line() {
        let m = map();
        let d = Diag::error(Span::new(15, 9999), "unterminated");
        let text = m.render(&d);
        let carets = text.matches('^').count();
        assert!(carets > 0 && carets <= m.line_text_at(1).len(), "{}", text);
    }

    #[test]
    fn a_span_in_the_second_file_names_the_second_file() {
        // The whole point of concatenating: `cat a.ddl b.ddl` also compiles,
        // but reports every error in b as though it were at the bottom of a.
        let m = SourceMap::from_files(vec![
            ("types.ddl".into(), "enum e: i1
  A
".into()),
            ("shift.ddl".into(), "fun f (a: i32)
  let y = foo + 1
".into()),
        ]);
        let lo = m.text().find("foo").expect("in the buffer") as u32;

        assert_eq!(m.path_at(lo), "shift.ddl");
        assert_eq!(m.line_col(lo), (2, 11));
        assert_eq!(m.path_at(0), "types.ddl");

        let text = m.render(&Diag::error(Span::new(lo, lo + 3), "undefined"));
        assert!(text.contains("--> shift.ddl:2:11"), "{}", text);
        assert!(text.contains("2 |   let y = foo + 1"), "{}", text);
    }

    #[test]
    fn a_file_without_a_trailing_newline_does_not_run_into_the_next() {
        let m = SourceMap::from_files(vec![
            ("a.ddl".into(), "fun f (o: out i1)".into()),
            ("b.ddl".into(), "fun g (o: out i1)
".into()),
        ]);
        assert!(m.text().contains("i1)
fun g"), "{:?}", m.text());
        let lo = m.text().find("fun g").expect("in the buffer") as u32;
        assert_eq!(m.line_col(lo), (1, 1));
        assert_eq!(m.path_at(lo), "b.ddl");
    }

    #[test]
    fn a_diagnostic_without_a_span_still_renders() {
        let m = map();
        let d = Diag::error_no_span("could not read the file").with_note("check the path");
        let text = m.render(&d);
        assert!(text.starts_with("error: could not read the file"));
        assert!(text.contains("note: check the path"));
    }
}
