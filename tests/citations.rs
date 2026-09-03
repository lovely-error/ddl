// `desc.md:N` citations, checked against desc.md.
//
// This codebase points at the design document by line number, in comments, in
// diagnostics and in the examples. Nothing consulted those numbers, so they
// drifted: a sweep found seventeen of twenty-six pointing somewhere else, and
// the two most-cited ones -- for "a process reaches its own state and nothing
// else" -- landed on a blank line twenty-three. A citation that resolves to
// nothing is worse than no citation, because a reader who follows it concludes
// the claim was invented.
//
// (Written out in words there rather than in the `desc.md:N` form, because
// this file scans itself and an illustration would fail as a citation.)
//
// The same argument as tests/editor.rs: a reference nothing verifies goes
// stale in silence, and the symptom is not a build failure.
//
// Two checks, and the second is the one with teeth:
//
//   * the cited line exists and is not blank. Catches the whole class the
//     sweep found most of -- desc.md grew at the top and every number below
//     the insertion slid.
//   * where the citing text quotes desc.md directly -- `desc.md:37, "may stop
//     (reach terminal state)"` -- the cited line must contain the quote. That
//     catches a slide of one or two lines, which the blank check cannot.
//
// What it CANNOT check is a citation that lands on a plausible-looking wrong
// line and quotes nothing. `desc.md:44` for "a sequence must not contain
// memory" is correct today and would still pass if desc.md were reordered
// around it. Quoting the line you cite is what makes a citation checkable, and
// is worth doing for that reason.

/// Citations that name a compiler source file and a line, checked the same way.
///
/// The documents point INTO the compiler as well as at the design: port-k2g.md
/// argues from two places in the graph lowering and one in the IR. Nothing
/// consulted those either, and they had drifted exactly as the desc.md ones
/// did -- one past the end of a file that had since shrunk, one onto a bare
/// closing brace.
///
/// The check is weaker here than for desc.md, because a citation into code
/// lands on a plausible-looking line far more often than one into prose: a
/// closing brace is not blank. What it still catches is the class the desc.md
/// sweep found most of -- a file grew or shrank above the citation and every
/// number below it slid.
///
/// (Written out in words rather than in the form it scans for, because this
/// file scans itself and an illustration would fail as a citation. The same
/// reason the test above gives.)
#[test]
fn every_source_citation_points_at_something() {
    for path in citing_files() {
        let text = read(&path);
        for (ix, line) in text.lines().enumerate() {
            for cite in source_citations(line) {
                let (file, n) = cite;
                let cited = match std::fs::read_to_string(&file) {
                    Ok(t) => t,
                    Err(_) => panic!(
                        "{}:{} cites {}, which is not a file",
                        path,
                        ix + 1,
                        file
                    ),
                };
                let lines: Vec<&str> = cited.lines().collect();
                assert!(
                    n >= 1 && n <= lines.len(),
                    "{}:{} cites {}:{}, which has {} lines",
                    path,
                    ix + 1,
                    file,
                    n,
                    lines.len()
                );
                assert!(
                    !lines[n - 1].trim().is_empty(),
                    "{}:{} cites {}:{}, which is blank",
                    path,
                    ix + 1,
                    file,
                    n
                );
            }
        }
    }
}

/// Every such citation in one line, as `(path, line)`.
///
/// A range cites its first line; that the rest exist follows from the file
/// being at least that long, which checking the first already establishes.
fn source_citations(line: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("src/") {
        let tail = &rest[at..];
        rest = &rest[at + 4..];
        let Some(colon) = tail.find(".rs:") else {
            continue;
        };
        let file = &tail[..colon + 3];
        let is_a_path = file[4..].chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
        if !is_a_path {
            continue;
        }
        let digits: String = tail[colon + 4..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(n) = digits.parse::<usize>() {
            out.push((file.to_string(), n));
        }
    }
    out
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

/// Every file that might cite the design document.
fn citing_files() -> Vec<String> {
    let mut out = Vec::new();
    for dir in ["src", "tests", "examples", "."] {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let keep = path
                .extension()
                .is_some_and(|e| e == "rs" || e == "ddl" || e == "md" || e == "sh");
            if keep && path.is_file() {
                let p = path.to_string_lossy().replace('\\', "/");
                // desc.md cites nothing and is what everything else cites.
                if !p.ends_with("desc.md") {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

/// Comment markers removed and whitespace collapsed, so a citation and the
/// quote after it can be matched across a wrapped comment.
fn flatten(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let line = line.trim_start();
        let line = line
            .strip_prefix("///")
            .or_else(|| line.strip_prefix("//"))
            .or_else(|| line.strip_prefix("--"))
            .unwrap_or(line);
        out.push_str(line.trim());
        out.push(' ');
    }
    out
}

/// One `desc.md:N` or `desc.md:N-M`, and what follows it.
struct Citation {
    line: usize,
    through: usize,
    /// The text desc.md is quoted as saying, when the citation quotes it.
    quote: Option<String>,
}

fn citations_in(flat: &str) -> Vec<Citation> {
    let mut found = Vec::new();
    let bytes = flat.as_bytes();
    let mut at = 0;
    while let Some(hit) = flat[at..].find("desc.md:") {
        let start = at + hit + "desc.md:".len();
        at = start;

        let digits = |from: usize| {
            let mut end = from;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end == from {
                None
            } else {
                Some((flat[from..end].parse::<usize>().expect("digits"), end))
            }
        };

        let (line, mut cursor) = match digits(start) {
            Some(v) => v,
            None => continue,
        };
        // `desc.md:53-58` cites a run.
        let mut through = line;
        if bytes.get(cursor) == Some(&b'-')
            && let Some((end_line, end_at)) = digits(cursor + 1)
        {
            through = end_line;
            cursor = end_at;
        }

        // A quote counts only when it follows the citation directly, with
        // nothing between but `'s`, punctuation and space. Anything wordier is
        // prose about the line rather than a copy of it, and anything further
        // away risks picking up an unrelated string literal.
        let rest = &flat[cursor..];
        let gap: String = rest
            .chars()
            .take_while(|c| *c != '"')
            .take(6)
            .collect();
        let gap_is_punctuation = gap.chars().all(|c| " ,:'s".contains(c));
        let quote = if gap_is_punctuation
            && let Some(open) = rest.find('"')
            && open <= gap.len()
            && let Some(close) = rest[open + 1..].find('"')
        {
            let q = &rest[open + 1..open + 1 + close];
            (q.len() >= 12).then(|| q.to_string())
        } else {
            None
        };

        found.push(Citation { line, through, quote });
    }
    found
}

#[test]
fn every_citation_points_at_something() {
    let desc: Vec<&str> = {
        let leaked: &'static str = Box::leak(read("desc.md").into_boxed_str());
        leaked.lines().collect()
    };

    let mut checked = 0usize;
    let mut quoted = 0usize;
    for path in citing_files() {
        let flat = flatten(&read(&path));
        for c in citations_in(&flat) {
            checked += 1;
            assert!(
                c.line >= 1 && c.through <= desc.len(),
                "{} cites desc.md:{}, which has {} lines",
                path,
                c.line,
                desc.len()
            );
            let cited: Vec<&str> = desc[c.line - 1..c.through].to_vec();
            assert!(
                cited.iter().any(|l| !l.trim().is_empty()),
                "{} cites desc.md:{}, which is blank",
                path,
                c.line
            );

            let Some(q) = c.quote else { continue };
            quoted += 1;
            let body = cited.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
            let want = q.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                body.contains(&want),
                "{} quotes desc.md:{} as saying {:?}, but it says {:?}",
                path,
                c.line,
                want,
                body
            );
        }
    }

    // Anti-vacuous. A rename or a moved directory that made `citing_files`
    // return nothing would otherwise pass silently, which is the exact failure
    // this file exists to prevent.
    assert!(checked >= 20, "only found {} citations; the sweep is not running", checked);
    assert!(quoted >= 4, "only {} citations quote the line they cite", quoted);
}
