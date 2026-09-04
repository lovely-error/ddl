// `ddl fmt`.
//
// A formatter is worth more in this language than in most, because a DDL block
// is delimited by indentation: getting the whitespace wrong does not raise a
// syntax error, it changes which block a statement belongs to. That is also
// what makes writing one delicate -- the thing a formatter normally does most
// freely is the one thing here that can change the program.
//
// SO IT IS TEXT, NOT AN AST ROUND TRIP. The parser drops comments entirely --
// `skip_trivia` skips them and no node holds one -- so re-emitting from the
// AST would produce a file with every comment deleted. examples/tagged.ddl is
// about forty percent comment. That is not a formatter, that is a shredder.
//
// AND IT DOES NOT RE-INDENT. That was tried and thrown away, which is worth
// recording because it looks like the obvious thing for a formatter to do.
// Levels can be recovered textually -- a stack of the indentation depths seen
// so far -- and doing that reformatted four of the nine examples, legally: the
// syntax tree came out identical every time. It was still wrong. What it
// changed was the continuation lines of wrapped parameter lists,
//
//     fun k2g_shift (
//         value: u32,          -- shift operand, and the BINS destination
//
// pulling them from four spaces to two, because a deeper line is
// indistinguishable from a nested block without knowing what construct it is
// inside. Four spaces there is exactly what separates a wrapped signature from
// a body statement at two. The parse survived and the reason to read it did
// not.
//
// Only the parser knows which deeper lines are blocks, and the parser throws
// that away as trivia. So indentation is left entirely alone and this does
// hygiene: the changes that cannot mean anything.
//
// IT STILL PROVES IT CHANGED NOTHING. The file is parsed before and after and
// the syntax trees compared, and a difference means the original is left
// exactly as it was. Cheap, and the one guarantee that makes a formatter for
// this language safe to run unattended.

use crate::diag::SourceMap;
use crate::driver::{Emit, compile};
use crate::verilog::EmitOptions;

/// What formatting a file did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Already formatted; nothing to write.
    Unchanged,
    /// Formatted, and the syntax tree is unchanged.
    Changed(String),
    /// Formatting it would have changed what it means, or could not be shown
    /// not to, so it was left alone.
    ///
    /// Usually not an error in the file. A tab, or a file that does not parse
    /// on its own, are both reasons to decline rather than reasons to
    /// complain.
    Refused(String),
}

/// Formats source text, or explains why it will not.
pub fn format_source(text: &str) -> Outcome {
    let formatted = match tidy(text) {
        Some(f) => f,
        None => {
            return Outcome::Refused(
                "it contains a tab, and the width to replace it with is a guess".to_string(),
            );
        }
    };

    if formatted == text {
        return Outcome::Unchanged;
    }

    // The proof. A formatter for an indentation-delimited language that cannot
    // show it preserved the program is a formatter nobody should run.
    match (syntax_of(text), syntax_of(&formatted)) {
        (Some(before), Some(after)) if before == after => Outcome::Changed(formatted),
        (Some(_), Some(_)) => Outcome::Refused(
            "formatting it would have changed what it parses to".to_string(),
        ),
        // A file that does not parse cannot be shown to be unharmed, so it is
        // not touched. `ddl check` is the tool for finding out why.
        _ => Outcome::Refused("it does not parse".to_string()),
    }
}

/// The syntax tree, as text, or `None` if it does not parse.
///
/// `--emit=ast` prints identifiers rather than addresses and is deterministic,
/// which is what makes it usable as an equality test.
fn syntax_of(text: &str) -> Option<String> {
    // Blank the imports first, exactly as a real compilation does. A file that
    // names another one parses fine in a build and not at all on its own, and
    // a formatter that declined to touch every file with an `import` in it
    // would decline to touch most of them.
    let seen = crate::source::as_the_parser_sees_it(text);
    let map = SourceMap::new("fmt.ddl", seen);
    compile(&map, &EmitOptions::default(), Emit::Ast).ok()
}

/// Whitespace hygiene: the changes that cannot alter a program.
///
/// Trailing whitespace, line endings, runs of blank lines, and the final
/// newline. Indentation is not touched -- see the note at the top of the file
/// for the version that did touch it and why it is gone.
fn tidy(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0usize;

    for raw in text.lines() {
        let line = raw.trim_end();

        if line.is_empty() {
            // A run of blank lines collapses. More than one is never
            // information, and unlike indentation a blank line cannot change
            // which block anything belongs to.
            blank_run += 1;
            if blank_run == 1 {
                out.push('\n');
            }
            continue;
        }
        blank_run = 0;

        // The compiler rejects tabs outright: it counts spaces for
        // indentation, so a tab silently changes which block a line is in.
        // Fixing one means choosing a width, and choosing wrong moves the
        // line. This declines rather than guesses.
        if line.contains('\t') {
            return None;
        }

        out.push_str(line);
        out.push('\n');
    }

    // Exactly one trailing newline. A file with none runs its last line into
    // whatever follows it in the buffer, which for a multi-file build is the
    // next file.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation_is_left_exactly_as_it_was() {
        // The version that canonicalised it reformatted four of the nine
        // examples, legally -- the syntax tree was identical every time -- and
        // pulled the continuation lines of every wrapped parameter list from
        // four spaces to two. Only the parser knows which deeper lines are
        // blocks, and it throws that away.
        let src = concat!(
            "fun k2g_shift (
",
            "    value: u32,
",
            "    amount: u5,
",
            "    result: out u32)
",
            "  result = value
",
        );
        assert_eq!(format_source(src), Outcome::Unchanged);
    }

    #[test]
    fn nesting_is_not_rewritten_either() {
        let src = concat!(
            "fun f (c: u1, a: u8, o: out u8)
",
            "    if c then
",
            "        o = a
",
            "    else
",
            "        o = a
",
        );
        assert_eq!(format_source(src), Outcome::Unchanged);
    }

    #[test]
    fn comments_survive() {
        // The whole reason this is not an AST round trip: the parser drops
        // every one of them, so re-emitting from the tree would delete them.
        let src = "-- a note   \nfun f (a: u8, o: out u8)\n    -- another  \n    o = a\n";
        match format_source(src) {
            Outcome::Changed(got) => {
                assert!(got.contains("-- a note"), "{}", got);
                assert!(got.contains("-- another"), "{}", got);
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn trailing_whitespace_goes() {
        let got = format_source("fun f (a: u8, o: out u8)   \n  o = a\t\n");
        // The tab makes it decline; that is the point of the next test.
        assert!(matches!(got, Outcome::Refused(_)));

        let got = format_source("fun f (a: u8, o: out u8)   \n  o = a  \n");
        assert_eq!(got, Outcome::Changed("fun f (a: u8, o: out u8)\n  o = a\n".to_string()));
    }

    #[test]
    fn a_tab_makes_it_decline_rather_than_guess() {
        // Converting one means choosing a width, and choosing wrong moves the
        // line into a different block.
        let got = format_source("fun f (a: u8, o: out u8)\n\to = a\n");
        assert!(matches!(got, Outcome::Refused(_)), "{:?}", got);
    }

    #[test]
    fn a_run_of_blank_lines_collapses() {
        let got = format_source("fun f (a: u8, o: out u8)\n  o = a\n\n\n\n");
        assert_eq!(got, Outcome::Changed("fun f (a: u8, o: out u8)\n  o = a\n".to_string()));
    }

    #[test]
    fn a_missing_trailing_newline_is_added() {
        let got = format_source("fun f (a: u8, o: out u8)\n  o = a");
        assert_eq!(got, Outcome::Changed("fun f (a: u8, o: out u8)\n  o = a\n".to_string()));
    }

    #[test]
    fn an_already_formatted_file_is_left_alone() {
        let src = "fun f (a: u8, o: out u8)\n  o = a\n";
        assert_eq!(format_source(src), Outcome::Unchanged);
    }

    #[test]
    fn a_file_that_does_not_parse_is_not_touched() {
        // It cannot be shown to be unharmed, so it is not harmed. The trailing
        // whitespace is what gives it something to change, so the refusal is
        // the verification declining rather than there being nothing to do.
        let got = format_source("fun f (a: u8, o: out u8)   \n  ??? broken\n");
        assert!(matches!(got, Outcome::Refused(_)), "{:?}", got);
    }
}
