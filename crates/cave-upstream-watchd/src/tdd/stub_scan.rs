//! Stub scanner — counts `todo!()` / `unimplemented!()` / `unreachable!()`
//! macro invocations in the implementation files of a branch.
//!
//! Charter Golden Rule §1 says **no stubs in production code**. The scan
//! is whole-file (it does not diff against base) on purpose: stubs that
//! were already present before the branch are still stubs that the
//! Charter gate should refuse to ship.

use std::path::{Path, PathBuf};

use crate::tdd::classifier::{classify_file, FileKind};
use crate::tdd::git_inspector::{GitError, GitInspector};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StubKind {
    Todo,
    Unimplemented,
    Unreachable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StubFinding {
    pub path: PathBuf,
    pub line: usize,
    pub kind: StubKind,
    pub snippet: String,
}

/// Scan the impl files touched on `branch` (relative to `base`) for stub
/// macros, reading the *post-branch* content of each file.
///
/// `inspector` is used in two ways:
///   1. enumerate commits in the range to find which files were touched;
///   2. read each touched file at `branch` tip.
pub fn scan_stubs<I: GitInspector + ?Sized>(
    inspector: &I,
    base: &str,
    branch: &str,
) -> Result<Vec<StubFinding>, GitError> {
    let commits = inspector.commits_between(base, branch)?;
    let mut touched: Vec<PathBuf> = Vec::new();
    for c in &commits {
        for f in &c.files {
            if classify_file(&f.path) == FileKind::Impl && !touched.contains(&f.path) {
                touched.push(f.path.clone());
            }
        }
    }

    let mut findings = Vec::new();
    for path in touched {
        let body = match inspector.read_at_commit(branch, &path)? {
            Some(b) => b,
            None => continue, // file deleted by the time we look — skip
        };
        scan_body(&path, &body, &mut findings);
    }
    Ok(findings)
}

/// Scan a single file's contents on disk. Public so the pre-commit hook can
/// reuse the parser without going through git.
pub fn scan_path<P: AsRef<Path>>(path: P) -> std::io::Result<Vec<StubFinding>> {
    let path = path.as_ref();
    let body = std::fs::read_to_string(path)?;
    let mut findings = Vec::new();
    scan_body(path, &body, &mut findings);
    Ok(findings)
}

fn scan_body(path: &Path, body: &str, out: &mut Vec<StubFinding>) {
    // Mask string-literal contents over the whole body first so the
    // `in_str` state is tracked across newlines (Rust permits multi-line
    // strings; per-line masking would let line 2 of a string literal leak
    // its `todo!()` into the scan). The masker replaces every in-string
    // byte with a space; the original `body` is still used to print the
    // human-readable snippet.
    let masked_body = strip_string_literals(body);
    let original_lines: Vec<&str> = body.lines().collect();

    for (idx, masked_line) in masked_body.lines().enumerate() {
        // Drop trailing `// ...` comments + doc-comments. (Block comments
        // `/* ... */` are not stripped — the false-positive rate is
        // negligible because block-comment usage in production Rust is
        // rare and the gate's safe failure mode is over-report anyway.)
        let code = match masked_line.split_once("//") {
            Some((before, _)) => before,
            None => masked_line,
        };
        let original_line = original_lines.get(idx).copied().unwrap_or("");
        if original_line.trim_start().starts_with("///")
            || original_line.trim_start().starts_with("//!")
        {
            continue;
        }
        let stripped = code.trim();

        for (kind, needle) in [
            (StubKind::Todo, "todo!("),
            (StubKind::Unimplemented, "unimplemented!("),
            (StubKind::Unreachable, "unreachable!("),
        ] {
            if stripped.contains(needle) {
                out.push(StubFinding {
                    path: path.to_path_buf(),
                    line: idx + 1,
                    kind,
                    snippet: original_line.trim().to_string(),
                });
            }
        }
    }
}

/// Replace the contents of every string literal — regular `"..."` and raw
/// `r"..."` / `r#"..."#` / `r##"..."##` (any number of hashes) — with
/// spaces (length-preserving, newlines kept) so a subsequent `contains`
/// check sees only non-string code. Regular strings handle `\"` and `\\`
/// escapes; raw strings honour their hash-delimiter rules per the Rust
/// reference.
pub(crate) fn strip_string_literals(src: &str) -> String {
    // Two-state machine over `char_indices`:
    //   State::Code     — outside any string
    //   State::Str { .. } — inside a regular `"..."` literal (escape-aware)
    //   State::Raw { hashes } — inside a raw `r#...".."#...` literal
    //
    // Substitution rule: a char that is *inside* a string is replaced with
    // ` ` (newline preserved); the same-byte-count substitution is enough
    // because the scanner only looks for ASCII needles and never reports
    // column ranges.
    enum State {
        Code,
        Str,
        Raw(usize), // hash count
    }

    let mut out = String::with_capacity(src.len());
    let mut state = State::Code;
    let mut iter = src.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        match state {
            State::Code => {
                // Look ahead for raw-string opener: `r`/`br` + N×`#` + `"`.
                let is_raw_prefix = c == 'r'
                    || (c == 'b' && {
                        let rest = &src[i + c.len_utf8()..];
                        rest.starts_with('r')
                    });
                if is_raw_prefix {
                    let prefix_len = if c == 'b' { 2 } else { 1 };
                    let rest = &src[i + prefix_len..];
                    let hashes = rest.bytes().take_while(|b| *b == b'#').count();
                    let after_hashes = &rest[hashes..];
                    if after_hashes.starts_with('"') {
                        // Commit: mask `r` (or `br`) + hashes + opening `"`
                        let total = prefix_len + hashes + 1;
                        out.push_str(&" ".repeat(total));
                        // Advance the char iterator past those `total` bytes
                        // (all ASCII so byte-count == char-count).
                        for _ in 0..(total - c.len_utf8()) {
                            iter.next();
                        }
                        state = State::Raw(hashes);
                        continue;
                    }
                }
                if c == '"' {
                    out.push(' ');
                    state = State::Str;
                } else {
                    out.push(c);
                }
            }
            State::Str => {
                if c == '\\' {
                    out.push(' ');
                    if let Some((_, next)) = iter.next() {
                        if next == '\n' {
                            out.push('\n');
                        } else {
                            out.push(' ');
                        }
                    }
                } else if c == '"' {
                    out.push(' ');
                    state = State::Code;
                } else if c == '\n' {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            State::Raw(hashes) => {
                if c == '"' {
                    // Closing iff followed by exactly `hashes` `#` chars.
                    let rest = &src[i + 1..];
                    if rest.bytes().take(hashes).filter(|b| *b == b'#').count() == hashes {
                        out.push(' ');
                        for _ in 0..hashes {
                            out.push(' ');
                            iter.next();
                        }
                        state = State::Code;
                        continue;
                    }
                    out.push(' ');
                } else if c == '\n' {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_todo_macro() {
        let body = "fn foo() { todo!(\"later\") }\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, StubKind::Todo);
        assert_eq!(out[0].line, 1);
    }

    #[test]
    fn finds_unimplemented_and_unreachable() {
        let body = "
fn a() { unimplemented!() }
fn b(x: i32) -> i32 {
    match x {
        _ => unreachable!(\"hot path\"),
    }
}
";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, StubKind::Unimplemented);
        assert_eq!(out[1].kind, StubKind::Unreachable);
    }

    #[test]
    fn ignores_comment_mentions() {
        let body = "// remember: todo!() is bad\nfn ok() {}\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn ignores_doc_comments() {
        let body = "/// We use todo!() as a placeholder in docs\nfn ok() {}\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn ignores_trailing_comment_with_macro_name() {
        let body = "fn ok() {} // todo!() one day\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn finds_macro_with_args() {
        let body = "fn x() { todo!(\"msg: {} {}\", 1, 2); }\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert_eq!(out.len(), 1);
    }

    // ── string-literal false-positive suppression ────────────────────

    #[test]
    fn quoted_macro_name_in_string_literal_is_not_flagged() {
        // The scanner's own lookup table contains `"todo!("` as a literal.
        // That must not register as a real stub call.
        let body = r#"
let needles = [("Todo", "todo!("), ("Unimpl", "unimplemented!("), ("Unr", "unreachable!(")];
"#;
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty(), "false positives: {out:?}");
    }

    #[test]
    fn eprintln_with_quoted_macro_mention_not_flagged() {
        let body = "fn warn() { eprintln!(\"don't use todo!() here\"); }\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn real_macro_call_still_flagged_when_quoted_mentions_present_elsewhere() {
        let body = "fn x() { eprintln!(\"checks todo!()\"); todo!() }\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, StubKind::Todo);
    }

    #[test]
    fn multiline_string_literal_masks_inner_macro_calls() {
        // Rust permits raw multi-line strings. A whole-body masker must
        // track `in_str` across newlines or the inner lines leak.
        let body = r#"let s = "
fn fake() { todo!() }
fn other() { unimplemented!() }
"; let x = 1;
"#;
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty(), "leaked from multi-line string: {out:?}");
    }

    #[test]
    fn escaped_quote_inside_string_does_not_close_string() {
        // `"a\"todo!("` is one literal containing `a"todo!(` — must not
        // be flagged as a real call. The escape-aware peeler keeps the
        // `in_str` state true past the `\"` so `todo!(` stays masked.
        let body = "let s = \"escaped \\\"todo!(\\\" inside\"; let y = 1;\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn strip_string_literals_basic() {
        assert_eq!(strip_string_literals(r#"a + "hello" + b"#), r#"a +         + b"#);
    }

    #[test]
    fn strip_string_literals_preserves_outside() {
        let s = strip_string_literals(r#"foo("x") + bar"#);
        assert!(s.contains("foo"));
        assert!(s.contains("bar"));
        assert!(!s.contains("x"));
    }

    // ── raw-string handling ──────────────────────────────────────────

    #[test]
    fn raw_string_single_hash_masks_inner_macro() {
        // r#"..."# with macro calls inside — common Rust test-fixture
        // shape. The masker must hide them so the gate does not
        // false-positive on its own / nearby test source files.
        let body = "let src = r#\"\npub fn x() { unimplemented!() }\npub fn y() { todo!() }\n\"#;\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty(), "raw-string leak: {out:?}");
    }

    #[test]
    fn raw_string_double_hash_masks_inner_macro() {
        let body = "let s = r##\"todo!()\"##;\nlet y = 1;\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn raw_string_no_hash_masks_inner_macro() {
        let body = "let s = r\"todo!()\";\nlet y = 1;\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn byte_raw_string_masks_inner_macro() {
        let body = "let s = br#\"todo!()\"#;\nlet y = 1;\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn raw_string_does_not_consume_lone_r_identifier() {
        // `let r = todo!()` starts with `r` but `r` is followed by `=`,
        // not `"`. Must NOT enter raw-string mode; the real macro call
        // MUST be flagged.
        let body = "let r = todo!(\"yes\");\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn real_macro_after_raw_string_still_flagged() {
        let body = "let s = r#\"todo!()\"#; todo!();\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
    }

    #[test]
    fn raw_string_with_unbalanced_hashes_inside_is_preserved() {
        // Inner `#` doesn't terminate; only `"##` does for double-hash.
        let body = "let s = r##\"foo \"#\" todo!()\"##;\nlet y = 1;\n";
        let mut out = Vec::new();
        scan_body(Path::new("x.rs"), body, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }
}
