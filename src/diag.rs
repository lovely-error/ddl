// Source locations and diagnostics.
//
// The parser addresses source text by raw pointer -- an `AlphanumSpan` is a
// `*const u8` and a length, with no offset and no lifetime. Rather than widen
// that type and touch every construction site in lex.rs, this module keeps the
// base pointer of the file alongside the text, which is enough to turn any
// pointer the parser hands back into a byte offset and then a line and column.
//
// That works because there is exactly one source buffer in flight at a time.
// When multi-file compilation arrives, `AlphanumSpan` grows a file id and the
// arithmetic here moves behind it; nothing else in the compiler changes.

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

pub struct SourceMap {
    path: String,
    text: String,
    /// Byte offset of the first character of each line.
    line_starts: Vec<u32>,
}

impl SourceMap {
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0u32];
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push((ix + 1) as u32);
            }
        }
        SourceMap { path: path.into(), text, line_starts }
    }

    pub fn path(&self) -> &str {
        &self.path
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

    /// 1-based line and column. Column counts bytes, which equals characters
    /// because DDL identifiers and operators are ASCII.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let line_ix = match self.line_starts.binary_search(&offset) {
            Ok(ix) => ix,
            Err(ix) => ix - 1,
        };
        let col = offset - self.line_starts[line_ix];
        ((line_ix + 1) as u32, col + 1)
    }

    fn line_text(&self, line_no: u32) -> &str {
        let ix = (line_no - 1) as usize;
        let start = self.line_starts[ix] as usize;
        let end = self
            .line_starts
            .get(ix + 1)
            .map(|e| *e as usize)
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
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
        let line = self.line_text(line_no);
        let gutter_w = format!("{}", line_no).len();
        let pad = " ".repeat(gutter_w);

        out.push_str(&format!("\n{} --> {}:{}:{}", pad, self.path, line_no, col));
        out.push_str(&format!("\n{} |", pad));
        out.push_str(&format!("\n{} | {}", line_no, line));

        // Clamp the underline to this line: a span may legitimately run past
        // the end of it (an unterminated construct), and an underline longer
        // than the text it marks reads as a rendering bug.
        let line_end = self.line_starts[(line_no - 1) as usize] + line.len() as u32;
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
/// the threading already present in sema.rs is unchanged -- only the push
/// sites move from `format!` to a call that carries a location.
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
        assert!(carets > 0 && carets <= m.line_text(2).len(), "{}", text);
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
