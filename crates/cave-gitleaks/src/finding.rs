// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Finding type and redaction.
//!
//! Mirrors `report/finding.go` upstream (`v8.29.1`). One `Finding` per
//! detected secret occurrence, carrying enough context to triage, dedup,
//! and route to a SARIF / JSON report.

use serde::Serialize;

/// A single match from the rule engine.
///
/// Field order, names, and JSON tags mirror upstream `report.Finding`
/// so cave-gitleaks JSON output can be ingested by upstream Gitleaks
/// dashboards / consumers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    #[serde(rename = "Description")]
    pub description: String,
    #[serde(rename = "StartLine")]
    pub start_line: usize,
    #[serde(rename = "EndLine")]
    pub end_line: usize,
    #[serde(rename = "StartColumn")]
    pub start_column: usize,
    #[serde(rename = "EndColumn")]
    pub end_column: usize,
    #[serde(rename = "Match")]
    pub match_text: String,
    /// Redacted secret value (always; we never persist raw secrets).
    #[serde(rename = "Secret")]
    pub secret: String,
    #[serde(rename = "File")]
    pub file: String,
    #[serde(rename = "SymlinkFile")]
    pub symlink_file: String,
    #[serde(rename = "Commit")]
    pub commit: String,
    #[serde(rename = "Entropy")]
    pub entropy: f32,
    #[serde(rename = "Author")]
    pub author: String,
    #[serde(rename = "Email")]
    pub email: String,
    #[serde(rename = "Date")]
    pub date: String,
    #[serde(rename = "Message")]
    pub message: String,
    #[serde(rename = "Tags")]
    pub tags: Vec<String>,
    #[serde(rename = "RuleID")]
    pub rule_id: String,
    #[serde(rename = "Fingerprint")]
    pub fingerprint: String,
}

impl Finding {
    /// A deterministic fingerprint used for dedup across runs:
    /// `commit:file:rule_id:start_line`. Mirrors the upstream
    /// `finding.go::Finding.Fingerprint` derivation for the working-tree
    /// case where commit is empty.
    pub fn compute_fingerprint(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            if self.commit.is_empty() {
                "WORKING"
            } else {
                self.commit.as_str()
            },
            self.file,
            self.rule_id,
            self.start_line
        )
    }
}

/// Redact a secret value: preserve first 4 visible chars + an asterisk
/// run sized to match the original (capped at 12 to keep output tidy).
/// Matches upstream `detect.redact` semantics: redacts the whole secret
/// rather than substituting a fixed token, so column positions in the
/// report stay meaningful while the value is not exfiltrated.
pub fn redact(secret: &str) -> String {
    let len = secret.chars().count();
    if len <= 4 {
        return "*".repeat(len.max(1));
    }
    let prefix: String = secret.chars().take(4).collect();
    let stars = std::cmp::min(len.saturating_sub(4), 12);
    format!("{prefix}{}", "*".repeat(stars))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_finding(rule: &str, commit: &str, file: &str, line: usize) -> Finding {
        Finding {
            description: String::new(),
            start_line: line,
            end_line: line,
            start_column: 0,
            end_column: 0,
            match_text: String::new(),
            secret: String::new(),
            file: file.into(),
            symlink_file: String::new(),
            commit: commit.into(),
            entropy: 0.0,
            author: String::new(),
            email: String::new(),
            date: String::new(),
            message: String::new(),
            tags: Vec::new(),
            rule_id: rule.into(),
            fingerprint: String::new(),
        }
    }

    #[test]
    fn redact_short_secrets_emits_all_stars() {
        assert_eq!(redact(""), "*");
        assert_eq!(redact("a"), "*");
        assert_eq!(redact("abcd"), "****");
    }

    #[test]
    fn redact_long_secret_preserves_4_char_prefix() {
        let s = redact("AKIAIOSFODNN7EXAMPLE");
        assert!(s.starts_with("AKIA"));
        assert!(s.contains('*'));
        // Everything after the 4-char prefix must be asterisks — no
        // payload bytes leak past the prefix.
        assert!(s[4..].chars().all(|c| c == '*'));
    }

    #[test]
    fn redact_caps_star_run_at_twelve() {
        let huge = "X".repeat(80);
        let r = redact(&huge);
        assert_eq!(r.matches('*').count(), 12);
    }

    #[test]
    fn fingerprint_with_commit_uses_commit_sha() {
        let f = make_finding("aws-access-token", "abc123", "src/main.rs", 42);
        assert_eq!(
            f.compute_fingerprint(),
            "abc123:src/main.rs:aws-access-token:42"
        );
    }

    #[test]
    fn fingerprint_without_commit_uses_working_marker() {
        let f = make_finding("aws-access-token", "", "src/main.rs", 42);
        assert_eq!(
            f.compute_fingerprint(),
            "WORKING:src/main.rs:aws-access-token:42"
        );
    }

    #[test]
    fn finding_is_serde_serialize_with_upstream_tags() {
        let mut f = make_finding("aws-access-token", "abc", "f.rs", 1);
        f.secret = "AKIA****".into();
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains("\"RuleID\":\"aws-access-token\""));
        assert!(j.contains("\"Commit\":\"abc\""));
        assert!(j.contains("\"Secret\":\"AKIA****\""));
    }
}
