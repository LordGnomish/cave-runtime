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
    for (idx, line) in body.lines().enumerate() {
        // Ignore comments and string literals that *mention* the macro but
        // do not invoke it. Heuristic: strip everything after `//` and skip
        // lines that look like `// ... todo!() ...`.
        let code = match line.split_once("//") {
            Some((before, _)) => before,
            None => line,
        };
        let stripped = code.trim();

        for (kind, needle) in [
            (StubKind::Todo, "todo!("),
            (StubKind::Unimplemented, "unimplemented!("),
            (StubKind::Unreachable, "unreachable!("),
        ] {
            if stripped.contains(needle) {
                // Suppress false positives: any line containing
                // `allow_stub_for_tdd` is acknowledged elsewhere. Also skip
                // doctest/example blocks (`///`).
                if line.trim_start().starts_with("///") || line.trim_start().starts_with("//!") {
                    continue;
                }
                out.push(StubFinding {
                    path: path.to_path_buf(),
                    line: idx + 1,
                    kind,
                    snippet: line.trim().to_string(),
                });
            }
        }
    }
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
}
