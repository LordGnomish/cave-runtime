// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! .trivyignore parser — CVE/misconfiguration suppression.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IgnoreEntry {
    pub id: String,
    pub comment: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrivyIgnore {
    pub entries: Vec<IgnoreEntry>,
}

impl TrivyIgnore {
    /// Parse a `.trivyignore` file.
    ///
    /// Format:
    /// ```text
    /// # Comment
    /// CVE-2021-44228
    /// CVE-2022-22965  # inline comment, exp:2024-01-01
    /// DS002
    /// ```
    pub fn parse(content: &str) -> Self {
        let mut entries = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Split on inline comment
            let (id_part, comment_part) = if let Some(pos) = line.find('#') {
                (line[..pos].trim(), Some(line[pos + 1..].trim()))
            } else {
                (line, None)
            };

            let id = id_part.trim().to_string();
            if id.is_empty() {
                continue;
            }

            // Parse expiry from comment: `exp:2024-01-01`
            let expires_at = comment_part.and_then(|c| {
                c.split_whitespace()
                    .find(|t| t.starts_with("exp:"))
                    .and_then(|t| {
                        let date_str = &t[4..];
                        chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                            .ok()
                            .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
                    })
            });

            entries.push(IgnoreEntry {
                id,
                comment: comment_part.map(str::to_string),
                expires_at,
            });
        }

        TrivyIgnore { entries }
    }

    /// Return true if the given CVE/check ID should be suppressed right now.
    pub fn is_ignored(&self, id: &str) -> bool {
        let now = Utc::now();
        self.entries.iter().any(|e| {
            e.id == id
                && match e.expires_at {
                    Some(exp) => now < exp,
                    None => true,
                }
        })
    }

    /// Filter a list of IDs, returning only the ones NOT ignored.
    pub fn filter_ignored<'a>(&self, ids: &[&'a str]) -> Vec<&'a str> {
        ids.iter()
            .copied()
            .filter(|id| !self.is_ignored(id))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# Accepted risk: vendor-provided image
CVE-2021-44228
CVE-2022-22965  # exp:2030-12-31

# Dockerfile check suppressed
DS002

# Already expired — should NOT suppress
CVE-2020-00001  # exp:2020-01-01
"#;

    #[test]
    fn parse_entries() {
        let ignore = TrivyIgnore::parse(SAMPLE);
        assert!(ignore.entries.iter().any(|e| e.id == "CVE-2021-44228"));
        assert!(ignore.entries.iter().any(|e| e.id == "DS002"));
    }

    #[test]
    fn is_ignored_active() {
        let ignore = TrivyIgnore::parse(SAMPLE);
        assert!(ignore.is_ignored("CVE-2021-44228"));
        assert!(ignore.is_ignored("CVE-2022-22965")); // exp 2030 — not expired
        assert!(ignore.is_ignored("DS002"));
    }

    #[test]
    fn is_not_ignored_expired() {
        let ignore = TrivyIgnore::parse(SAMPLE);
        assert!(!ignore.is_ignored("CVE-2020-00001")); // expired 2020
    }

    #[test]
    fn is_not_ignored_unknown() {
        let ignore = TrivyIgnore::parse(SAMPLE);
        assert!(!ignore.is_ignored("CVE-9999-00000"));
    }

    #[test]
    fn filter_list() {
        let ignore = TrivyIgnore::parse(SAMPLE);
        let ids = vec!["CVE-2021-44228", "CVE-9999-00000"];
        let result = ignore.filter_ignored(&ids);
        assert_eq!(result, vec!["CVE-9999-00000"]);
    }
}
