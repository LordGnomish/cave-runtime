// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pre-commit / GH-Action hook surface.
//!
//! Ports the `cmd/trufflehog/main.go --pre-commit` and `--gh-action` paths:
//! given a list of staged files (path + content), run the detector pipeline,
//! emit a stable exit code, and format a developer-readable summary on
//! stderr-equivalent output.

use crate::detector::{scan, Finding, SecretDetector};

/// Result of a pre-commit run.
#[derive(Debug, Clone)]
pub struct PrecommitResult {
    pub findings: Vec<Finding>,
    pub files_scanned: usize,
    pub lines_scanned: usize,
}

impl PrecommitResult {
    pub fn has_blocking(&self) -> bool {
        !self.findings.is_empty()
    }

    /// Convention from `trufflehog --pre-commit`: `0` clean, `1` findings, `2`
    /// internal error (not surfaced here; reserved for callers).
    pub fn exit_code(&self) -> i32 {
        if self.has_blocking() { 1 } else { 0 }
    }
}

/// A staged file presented to the hook.
#[derive(Debug, Clone)]
pub struct StagedFile<'a> {
    pub path: &'a str,
    pub content: &'a str,
}

/// Run detectors over each staged file. Skips paths matching any entry in
/// `ignore_paths` (substring match — same shape as gitleaks' allowlist).
pub fn run_precommit(
    files: &[StagedFile<'_>],
    detectors: &[SecretDetector],
    ignore_paths: &[String],
) -> PrecommitResult {
    let mut findings = Vec::new();
    let mut files_scanned = 0;
    let mut lines_scanned = 0;

    for f in files {
        if should_ignore(f.path, ignore_paths) {
            continue;
        }
        files_scanned += 1;
        lines_scanned += f.content.lines().count();
        findings.extend(scan(f.content, f.path, detectors));
    }
    PrecommitResult {
        findings,
        files_scanned,
        lines_scanned,
    }
}

fn should_ignore(path: &str, ignore_paths: &[String]) -> bool {
    ignore_paths.iter().any(|p| path.contains(p))
}

/// Render a human-readable summary for the developer terminal. Mirrors the
/// `git pre-commit` convention of "one finding per line, then a count footer".
pub fn format_summary(r: &PrecommitResult) -> String {
    if r.findings.is_empty() {
        return format!(
            "cave-secrets: 0 findings in {} files / {} lines\n",
            r.files_scanned, r.lines_scanned
        );
    }
    let mut out = String::new();
    out.push_str("cave-secrets: blocking findings detected\n");
    for f in &r.findings {
        out.push_str(&format!(
            "  [{}] {}:{}  {} ({:?})\n",
            f.detector, f.file, f.line, f.matched, f.severity
        ));
    }
    out.push_str(&format!(
        "scanned: {} files / {} lines / {} findings\n",
        r.files_scanned,
        r.lines_scanned,
        r.findings.len()
    ));
    out
}

/// GH-Action-style annotation output (one `::error::` line per finding).
pub fn format_gh_action(r: &PrecommitResult) -> String {
    let mut out = String::new();
    for f in &r.findings {
        out.push_str(&format!(
            "::error file={},line={}::secret detected ({})\n",
            f.file, f.line, f.detector
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::builtin_detectors;

    fn det() -> Vec<SecretDetector> {
        builtin_detectors()
    }

    #[test]
    fn clean_files_pass() {
        let files = [
            StagedFile { path: "src/main.rs", content: "fn main() {}\n" },
            StagedFile { path: "README.md", content: "hi\n" },
        ];
        let r = run_precommit(&files, &det(), &[]);
        assert!(!r.has_blocking());
        assert_eq!(r.exit_code(), 0);
        assert_eq!(r.files_scanned, 2);
    }

    #[test]
    fn finding_blocks_commit() {
        let files = [StagedFile {
            path: "config.env",
            content: "AWS_KEY=AKIAIOSFODNN7EXAMPLE\n",
        }];
        let r = run_precommit(&files, &det(), &[]);
        assert!(r.has_blocking());
        assert_eq!(r.exit_code(), 1);
        assert!(r.findings.iter().any(|f| f.detector == "aws-access-key"));
    }

    #[test]
    fn ignore_path_suppresses_scan() {
        let files = [StagedFile {
            path: "tests/fixtures/leak.env",
            content: "AWS_KEY=AKIAIOSFODNN7EXAMPLE\n",
        }];
        let r = run_precommit(&files, &det(), &["tests/fixtures".to_string()]);
        assert_eq!(r.files_scanned, 0);
        assert!(r.findings.is_empty());
    }

    #[test]
    fn summary_clean_message() {
        let r = PrecommitResult {
            findings: vec![],
            files_scanned: 3,
            lines_scanned: 30,
        };
        let s = format_summary(&r);
        assert!(s.contains("0 findings"));
        assert!(s.contains("3 files"));
    }

    #[test]
    fn summary_blocking_lists_each() {
        let files = [StagedFile {
            path: "config.env",
            content: "AWS_KEY=AKIAIOSFODNN7EXAMPLE\n",
        }];
        let r = run_precommit(&files, &det(), &[]);
        let s = format_summary(&r);
        assert!(s.contains("blocking"));
        assert!(s.contains("aws-access-key"));
    }

    #[test]
    fn gh_action_annotation_format() {
        let files = [StagedFile {
            path: "config.env",
            content: "AWS_KEY=AKIAIOSFODNN7EXAMPLE\n",
        }];
        let r = run_precommit(&files, &det(), &[]);
        let s = format_gh_action(&r);
        assert!(s.starts_with("::error file="));
        assert!(s.contains("aws-access-key"));
    }

    #[test]
    fn empty_input_zero_lines_scanned() {
        let r = run_precommit(&[], &det(), &[]);
        assert_eq!(r.files_scanned, 0);
        assert_eq!(r.lines_scanned, 0);
        assert!(r.findings.is_empty());
    }

    #[test]
    fn multi_file_with_one_leak() {
        let files = [
            StagedFile { path: "a.txt", content: "clean\n" },
            StagedFile {
                path: "b.env",
                content: "AWS_KEY=AKIAIOSFODNN7EXAMPLE\n",
            },
            StagedFile { path: "c.md", content: "fine\n" },
        ];
        let r = run_precommit(&files, &det(), &[]);
        assert_eq!(r.files_scanned, 3);
        assert_eq!(r.findings.len(), 1);
    }

    #[test]
    fn ignore_substring_match_does_not_match_arbitrary_path() {
        let files = [StagedFile {
            path: "src/main.rs",
            content: "AWS_KEY=AKIAIOSFODNN7EXAMPLE\n",
        }];
        let r = run_precommit(&files, &det(), &["fixtures".to_string()]);
        assert_eq!(r.files_scanned, 1);
        assert_eq!(r.findings.len(), 1);
    }
}
