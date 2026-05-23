// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! `.trivyignore` + `trivy.yaml` ignore-policy loader.
//!
//! Mirrors trivy's `pkg/result/ignore`. cave-trivy supports two formats:
//! line-delimited `.trivyignore` (one ID per line, `#` comments) and a
//! YAML `trivy.yaml` block: `ignore: { vulnerabilities: [..], misconfig:
//! [..] }`. Expiry dates of the form `CVE-x exp:2026-12-31` are honoured.

use chrono::NaiveDate;
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct IgnorePolicy {
    ids: HashSet<String>,
    expirations: Vec<(String, NaiveDate)>,
}

impl IgnorePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, id: &str) {
        self.ids.insert(id.to_string());
    }

    pub fn len(&self) -> usize {
        self.ids.len() + self.expirations.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.expirations.is_empty()
    }

    pub fn matches_id(&self, id: &str) -> bool {
        if self.ids.contains(id) {
            return true;
        }
        let today = chrono::Utc::now().date_naive();
        for (i, exp) in &self.expirations {
            if i == id && today <= *exp {
                return true;
            }
        }
        false
    }

    /// Line-delimited `.trivyignore` form, e.g.
    ///   # comment
    ///   CVE-2026-0001
    ///   CVE-2026-0010 exp:2026-12-31
    pub fn parse_trivyignore(text: &str) -> Self {
        let mut p = Self::new();
        for line in text.lines() {
            let l = line.split('#').next().unwrap_or("").trim();
            if l.is_empty() {
                continue;
            }
            if let Some((id, rest)) = l.split_once(' ') {
                if let Some(exp) = rest.strip_prefix("exp:") {
                    if let Ok(d) = NaiveDate::parse_from_str(exp.trim(), "%Y-%m-%d") {
                        p.expirations.push((id.trim().to_string(), d));
                        continue;
                    }
                }
            }
            p.ids.insert(l.to_string());
        }
        p
    }

    /// YAML block `trivy.yaml` form. Best-effort: only the `ignore:` block
    /// is consulted.
    pub fn parse_yaml_block(text: &str) -> Self {
        let mut p = Self::new();
        if let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(text) {
            if let Some(blk) = v.get("ignore") {
                for key in ["vulnerabilities", "misconfig", "secrets"] {
                    if let Some(arr) = blk.get(key).and_then(|v| v.as_sequence()) {
                        for item in arr {
                            if let Some(s) = item.as_str() {
                                p.ids.insert(s.to_string());
                            }
                        }
                    }
                }
            }
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_format() {
        let p = IgnorePolicy::parse_trivyignore("CVE-1\n# comment\nCVE-2\n");
        assert!(p.matches_id("CVE-1"));
        assert!(p.matches_id("CVE-2"));
        assert!(!p.matches_id("CVE-3"));
    }

    #[test]
    fn expiry_in_future_matches() {
        let p =
            IgnorePolicy::parse_trivyignore("CVE-FUTURE exp:9999-12-31\n");
        assert!(p.matches_id("CVE-FUTURE"));
    }

    #[test]
    fn expiry_in_past_does_not_match() {
        let p =
            IgnorePolicy::parse_trivyignore("CVE-PAST exp:1970-01-01\n");
        assert!(!p.matches_id("CVE-PAST"));
    }

    #[test]
    fn yaml_block_form() {
        let y = "ignore:\n  vulnerabilities: [CVE-1, CVE-2]\n  misconfig: [AVD-X-1]\n";
        let p = IgnorePolicy::parse_yaml_block(y);
        assert!(p.matches_id("CVE-1"));
        assert!(p.matches_id("CVE-2"));
        assert!(p.matches_id("AVD-X-1"));
    }

    #[test]
    fn add_and_len() {
        let mut p = IgnorePolicy::new();
        p.add("X");
        assert_eq!(p.len(), 1);
        assert!(!p.is_empty());
    }

    #[test]
    fn malformed_yaml_yields_empty() {
        let p = IgnorePolicy::parse_yaml_block("not yaml: :\n  - [");
        assert!(p.is_empty());
    }

    #[test]
    fn comments_only_yields_empty() {
        let p = IgnorePolicy::parse_trivyignore("# only comments\n\n");
        assert!(p.is_empty());
    }
}
