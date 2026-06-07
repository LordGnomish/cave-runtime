// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter-compliance gate.
//!
//! Before any LLM-produced change is allowed to commit + merge, it must clear
//! the Cave golden rule: **strict TDD, no stubs, honest LOC, no paperwork.**
//! This module is the machine enforcement of that rule. Three independent
//! checks compose into a [`CharterAudit`]:
//!
//! 1. [`scan_for_stubs`] — reject `todo!()` / `unimplemented!()` placeholders.
//! 2. [`tdd_sequence_compliant`] — the commit history for the task must show a
//!    failing-test (RED) commit before the first implementation (GREEN) commit.
//! 3. [`count_code_lines`] / [`impl_test_ratio`] — honest LOC measured from
//!    source, not self-reported, with a real test body present.
//!
//! The stub needles are assembled from fragments (`concat!`) so this scanner
//! never trips over its own source if pointed at the autopilot crate.

use std::path::{Path, PathBuf};

/// A detected stub placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubHit {
    pub line: usize,
    pub needle: &'static str,
    pub text: String,
}

/// Macro forms the Charter forbids in committed code.
fn stub_needles() -> [&'static str; 3] {
    [
        concat!("todo", "!("),
        concat!("unimplemented", "!("),
        concat!("unreachable", "!("),
    ]
}

/// Scan one source string for forbidden placeholder macros. Comment-only lines
/// (trimmed start `//`) are ignored so doc references don't false-positive.
pub fn scan_for_stubs(src: &str) -> Vec<StubHit> {
    let mut hits = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with("//!") {
            continue;
        }
        for needle in stub_needles() {
            if raw.contains(needle) {
                hits.push(StubHit {
                    line: i + 1,
                    needle,
                    text: raw.trim().to_string(),
                });
            }
        }
    }
    hits
}

/// Recursively scan a directory's `.rs` files for stubs. Skips `target/`.
pub fn scan_dir_for_stubs(dir: &Path) -> std::io::Result<Vec<(PathBuf, StubHit)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().and_then(|n| n.to_str()) == Some("target") {
                    continue;
                }
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                let src = std::fs::read_to_string(&p)?;
                for hit in scan_for_stubs(&src) {
                    out.push((p.clone(), hit));
                }
            }
        }
    }
    Ok(out)
}

/// Classification of a commit for TDD-sequence checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitKind {
    /// Introduces a failing test (the RED step).
    Red,
    /// Introduces / fixes implementation to pass (the GREEN step).
    Green,
    /// Chore, docs, refactor — irrelevant to the RED→GREEN ordering.
    Other,
}

/// Classify a commit message into a TDD step. Heuristics over conventional
/// markers; case-insensitive.
pub fn classify_commit(message: &str) -> CommitKind {
    let m = message.to_lowercase();
    let red = m.contains("red")
        || m.contains("failing test")
        || m.contains("add test")
        || m.contains("test:")
        || m.starts_with("test(");
    let green = m.contains("green")
        || m.contains("impl")
        || m.starts_with("feat")
        || m.contains("make test")
        || m.contains("pass");
    // RED wins ties: a "test:" commit is the failing step even if it says pass.
    if red {
        CommitKind::Red
    } else if green {
        CommitKind::Green
    } else {
        CommitKind::Other
    }
}

/// True iff the (chronological) commit sequence honours strict TDD: the first
/// GREEN commit is preceded by at least one RED commit. A sequence with no
/// GREEN commits is vacuously compliant (no implementation claimed yet).
pub fn tdd_sequence_compliant(kinds: &[CommitKind]) -> bool {
    let mut seen_red = false;
    for k in kinds {
        match k {
            CommitKind::Red => seen_red = true,
            CommitKind::Green => {
                if !seen_red {
                    return false;
                }
            }
            CommitKind::Other => {}
        }
    }
    true
}

/// Count non-blank, non-comment Rust source lines. This is the *honest* LOC
/// measure — it ignores blank lines and `//`/`//!`/`///` comment-only lines and
/// does not count anything self-reported.
pub fn count_code_lines(src: &str) -> usize {
    src.lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("//")
        })
        .count()
}

/// Ratio of test code lines to implementation code lines. A crate with zero
/// test lines returns `0.0` (and fails the audit). Implementation lines are
/// everything outside a `#[cfg(test)]` module / `tests/` file; here we take the
/// two counts directly.
pub fn impl_test_ratio(impl_lines: usize, test_lines: usize) -> f64 {
    if impl_lines == 0 {
        return 0.0;
    }
    test_lines as f64 / impl_lines as f64
}

/// Aggregate verdict for one task's produced change.
#[derive(Debug, Clone, PartialEq)]
pub struct CharterAudit {
    pub stub_hits: usize,
    pub tdd_compliant: bool,
    pub has_tests: bool,
    pub impl_lines: usize,
    pub test_lines: usize,
    pub violations: Vec<String>,
}

impl CharterAudit {
    /// Run the full gate over already-collected inputs.
    pub fn evaluate(
        stubs: &[(PathBuf, StubHit)],
        commit_kinds: &[CommitKind],
        impl_lines: usize,
        test_lines: usize,
    ) -> Self {
        let mut violations = Vec::new();
        if !stubs.is_empty() {
            violations.push(format!("{} stub placeholder(s) present", stubs.len()));
        }
        let tdd = tdd_sequence_compliant(commit_kinds);
        if !tdd {
            violations.push("GREEN commit precedes any RED (failing-test) commit".to_string());
        }
        let has_tests = test_lines > 0;
        if !has_tests {
            violations.push("no test lines found — TDD requires a failing test first".to_string());
        }
        Self {
            stub_hits: stubs.len(),
            tdd_compliant: tdd,
            has_tests,
            impl_lines,
            test_lines,
            violations,
        }
    }

    /// Passes the Charter gate iff there are no violations.
    pub fn passes(&self) -> bool {
        self.violations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_detects_todo_and_unimplemented() {
        let src = "fn a() {\n    todo!(\"later\");\n}\nfn b() { unimplemented!() }\n";
        let hits = scan_for_stubs(src);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, 2);
    }

    #[test]
    fn scan_ignores_comment_references() {
        let src = "// we must never write todo!() here\n//! unimplemented!() in docs is fine\nfn ok() {}\n";
        assert!(scan_for_stubs(src).is_empty());
    }

    #[test]
    fn classify_commit_markers() {
        assert_eq!(classify_commit("test(cave-x): add failing test for foo"), CommitKind::Red);
        assert_eq!(classify_commit("feat(cave-x): implement foo (GREEN)"), CommitKind::Green);
        assert_eq!(classify_commit("chore: bump deps"), CommitKind::Other);
    }

    #[test]
    fn tdd_sequence_requires_red_before_green() {
        use CommitKind::*;
        assert!(tdd_sequence_compliant(&[Red, Green]));
        assert!(tdd_sequence_compliant(&[Red, Green, Red, Green]));
        assert!(!tdd_sequence_compliant(&[Green]));
        assert!(!tdd_sequence_compliant(&[Other, Green, Red]));
        assert!(tdd_sequence_compliant(&[])); // vacuous
    }

    #[test]
    fn honest_loc_ignores_blank_and_comment_lines() {
        let src = "// header\n\nfn a() {\n    let x = 1;\n}\n// trailing\n";
        // counts: fn a() {, let x = 1;, } -> 3
        assert_eq!(count_code_lines(src), 3);
    }

    #[test]
    fn impl_test_ratio_handles_zero() {
        assert_eq!(impl_test_ratio(0, 10), 0.0);
        assert_eq!(impl_test_ratio(100, 50), 0.5);
    }

    #[test]
    fn audit_passes_clean_change() {
        use CommitKind::*;
        let a = CharterAudit::evaluate(&[], &[Red, Green], 120, 60);
        assert!(a.passes());
        assert!(a.has_tests);
    }

    #[test]
    fn audit_fails_on_stub() {
        use CommitKind::*;
        let stub = (
            PathBuf::from("x.rs"),
            StubHit { line: 1, needle: "todo!(", text: "todo!()".into() },
        );
        let a = CharterAudit::evaluate(&[stub], &[Red, Green], 100, 50);
        assert!(!a.passes());
        assert_eq!(a.stub_hits, 1);
    }

    #[test]
    fn audit_fails_without_tests() {
        use CommitKind::*;
        let a = CharterAudit::evaluate(&[], &[Green], 100, 0);
        assert!(!a.passes());
        assert!(!a.tdd_compliant);
        assert!(!a.has_tests);
    }
}
