// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Release-body parser → structured `Changelog`.
//!
//! GitHub release bodies are free-form markdown. The convention this
//! parser targets is the "Keep a Changelog" style:
//!
//! ```text
//! ## Added
//! - foo
//! - bar
//!
//! ## Changed
//! - baz
//!
//! ## Deprecated
//! - quux
//!
//! ## Breaking Changes
//! - old API removed
//! ```
//!
//! Other section headings (`### Bug Fixes`, `Features`, `Bug Fixes`,
//! …) are also recognised via case-insensitive substring match.
//! Bullets are detected for both `- ` and `* ` markers; numbered
//! lists are folded into `Changed` (we treat them as an explicit
//! ordering of changes, not a separate category).
//!
//! ## Why structured
//!
//! The auto-port dispatcher (Charter v2, NOT in this scaffold)
//! consumes the structured list to:
//!
//! * filter `Breaking` separately from `Added` (Charter says: never
//!   auto-port a breaking change without operator review),
//! * pick a "test plan" from `Added` (each new feature becomes a
//!   draft test in the port queue),
//! * prioritise `Deprecated` calls against existing cave callsites.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Changed,
    Fixed,
    Deprecated,
    Removed,
    Breaking,
    Security,
}

impl ChangeKind {
    /// Classify a section heading line (without the leading `## ` or
    /// `### `) into a kind. Substring-matched, case-insensitive.
    /// Returns `Changed` for any unrecognised heading so the entry
    /// isn't silently dropped.
    pub fn from_heading(s: &str) -> Self {
        let lower = s.to_lowercase();
        // Order matters — "breaking" and "security" before the
        // generic substring tests.
        if lower.contains("breaking") {
            return ChangeKind::Breaking;
        }
        if lower.contains("security") || lower.contains("cve") {
            return ChangeKind::Security;
        }
        if lower.contains("removed") || lower.contains("removal") {
            return ChangeKind::Removed;
        }
        if lower.contains("deprecat") {
            return ChangeKind::Deprecated;
        }
        if lower.contains("added") || lower.contains("feature") || lower.contains("new") {
            return ChangeKind::Added;
        }
        if lower.contains("fix") || lower.contains("bug") {
            return ChangeKind::Fixed;
        }
        ChangeKind::Changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangelogEntry {
    pub kind: ChangeKind,
    pub description: String,
    /// `true` when the line is in (or derived from) a heading
    /// containing "breaking" — duplicates the `kind == Breaking`
    /// signal but is convenient for callers that want a flat flag
    /// alongside the bucket.
    #[serde(default)]
    pub breaking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Changelog {
    pub entries: Vec<ChangelogEntry>,
}

impl Changelog {
    /// Number of entries by kind.
    pub fn counts_by_kind(&self) -> std::collections::HashMap<ChangeKind, usize> {
        let mut out = std::collections::HashMap::new();
        for e in &self.entries {
            *out.entry(e.kind).or_insert(0) += 1;
        }
        out
    }

    /// Convenience: any entry classified as Breaking?
    pub fn has_breaking(&self) -> bool {
        self.entries.iter().any(|e| e.breaking || e.kind == ChangeKind::Breaking)
    }

    /// Get entries of a single kind.
    pub fn of_kind(&self, kind: ChangeKind) -> Vec<&ChangelogEntry> {
        self.entries.iter().filter(|e| e.kind == kind).collect()
    }
}

/// Parse a GitHub release body string into a `Changelog`. Empty or
/// non-conforming bodies return a `Changelog::default()`; the
/// parser is intentionally tolerant — junk above the first heading is
/// ignored, junk between headings is dropped.
pub fn parse_release_body(body: &str) -> Changelog {
    let mut entries: Vec<ChangelogEntry> = Vec::new();
    let mut current_kind: Option<ChangeKind> = None;
    let mut current_breaking = false;

    for raw in body.lines() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim_start();

        // Heading? `# Foo`, `## Foo`, `### Foo`, `**Foo:**`.
        if let Some(after_hash) = heading_body(trimmed) {
            let kind = ChangeKind::from_heading(after_hash);
            let lower = after_hash.to_lowercase();
            let breaking_heading = lower.contains("breaking");
            current_kind = Some(kind);
            current_breaking = breaking_heading;
            continue;
        }

        // Bullet? `- foo`, `* foo`, `1. foo`.
        if let Some(desc) = bullet_body(trimmed) {
            let Some(kind) = current_kind else {
                continue;
            };
            // Special-case: a bullet that contains the word
            // "BREAKING" anywhere is bumped to Breaking regardless of
            // its heading.
            let mut effective = kind;
            let mut breaking = current_breaking;
            if desc.to_uppercase().contains("BREAKING") {
                effective = ChangeKind::Breaking;
                breaking = true;
            }
            entries.push(ChangelogEntry {
                kind: effective,
                description: desc.trim().to_string(),
                breaking,
            });
            continue;
        }
    }

    Changelog { entries }
}

/// Extract the body of a heading line, e.g. `"## Added"` → `Some("Added")`,
/// `"**Breaking Changes**"` → `Some("Breaking Changes")`.
fn heading_body(line: &str) -> Option<&str> {
    // `#`/`##`/`###` style.
    let trimmed = line.trim_start_matches('#').trim_start();
    if trimmed.len() < line.len() && !trimmed.is_empty() {
        return Some(trimmed);
    }
    // `**Heading:**` / `**Heading**` style (some upstreams do this).
    if line.starts_with("**") {
        let inner = line.trim_matches('*').trim_end_matches(':').trim();
        if !inner.is_empty() {
            return Some(inner);
        }
    }
    None
}

fn bullet_body(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("- ") {
        return Some(rest);
    }
    if let Some(rest) = line.strip_prefix("* ") {
        return Some(rest);
    }
    // Numbered list: `1. foo`, `12. bar`.
    let mut chars = line.chars();
    let mut saw_digit = false;
    let mut i = 0;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            saw_digit = true;
            i += 1;
            continue;
        }
        if saw_digit && c == '.' {
            let rest = &line[i + 1..];
            if let Some(stripped) = rest.strip_prefix(' ') {
                return Some(stripped);
            }
        }
        break;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keep_a_changelog_sections() {
        let body = "\
## Added
- foo
- bar

## Changed
- baz

## Deprecated
- quux
";
        let c = parse_release_body(body);
        assert_eq!(c.entries.len(), 4);
        assert_eq!(c.entries[0].kind, ChangeKind::Added);
        assert_eq!(c.entries[0].description, "foo");
        assert_eq!(c.entries[1].description, "bar");
        assert_eq!(c.entries[2].kind, ChangeKind::Changed);
        assert_eq!(c.entries[3].kind, ChangeKind::Deprecated);
    }

    #[test]
    fn breaking_section_marks_each_entry_breaking() {
        let body = "\
## Breaking Changes
- old API removed
- behaviour X changed
";
        let c = parse_release_body(body);
        assert_eq!(c.entries.len(), 2);
        assert!(c.entries[0].breaking);
        assert_eq!(c.entries[0].kind, ChangeKind::Breaking);
        assert!(c.has_breaking());
    }

    #[test]
    fn inline_breaking_keyword_overrides_section() {
        let body = "\
## Added
- normal feature
- BREAKING: this also removed legacy flag
";
        let c = parse_release_body(body);
        assert_eq!(c.entries[0].kind, ChangeKind::Added);
        assert_eq!(c.entries[1].kind, ChangeKind::Breaking);
        assert!(c.entries[1].breaking);
    }

    #[test]
    fn bullet_with_star_marker_works() {
        let body = "\
## Fixed
* one
* two
";
        let c = parse_release_body(body);
        assert_eq!(c.entries.len(), 2);
        assert_eq!(c.entries[0].kind, ChangeKind::Fixed);
    }

    #[test]
    fn numbered_bullet_falls_into_changed_when_under_no_section() {
        let body = "\
random preamble
1. should be ignored — no section yet
## Changed
1. now within changed
2. another
";
        let c = parse_release_body(body);
        assert_eq!(c.entries.len(), 2);
        assert_eq!(c.entries[0].description, "now within changed");
    }

    #[test]
    fn unknown_heading_falls_to_changed() {
        let body = "\
## Random Notes
- something
";
        let c = parse_release_body(body);
        assert_eq!(c.entries.len(), 1);
        assert_eq!(c.entries[0].kind, ChangeKind::Changed);
    }

    #[test]
    fn case_insensitive_heading_matching() {
        let body = "\
### features
- new flag

### Bug Fixes
- crash

### Security Patches
- CVE-2026-1234
";
        let c = parse_release_body(body);
        assert_eq!(c.entries[0].kind, ChangeKind::Added);
        assert_eq!(c.entries[1].kind, ChangeKind::Fixed);
        assert_eq!(c.entries[2].kind, ChangeKind::Security);
    }

    #[test]
    fn empty_body_returns_empty_changelog() {
        let c = parse_release_body("");
        assert!(c.entries.is_empty());
        assert!(!c.has_breaking());
    }

    #[test]
    fn counts_by_kind_aggregates() {
        let body = "\
## Added
- a
- b

## Fixed
- c
";
        let c = parse_release_body(body);
        let counts = c.counts_by_kind();
        assert_eq!(counts.get(&ChangeKind::Added).copied(), Some(2));
        assert_eq!(counts.get(&ChangeKind::Fixed).copied(), Some(1));
    }

    #[test]
    fn of_kind_returns_matching_entries() {
        let body = "\
## Added
- a
- b

## Fixed
- c
";
        let c = parse_release_body(body);
        let added = c.of_kind(ChangeKind::Added);
        assert_eq!(added.len(), 2);
    }

    #[test]
    fn bold_style_heading_recognised() {
        let body = "\
**Breaking:**
- removed old endpoint
";
        let c = parse_release_body(body);
        assert_eq!(c.entries.len(), 1);
        assert_eq!(c.entries[0].kind, ChangeKind::Breaking);
    }

    #[test]
    fn removed_section_classified_as_removed() {
        let body = "\
## Removed
- legacy flag
";
        let c = parse_release_body(body);
        assert_eq!(c.entries[0].kind, ChangeKind::Removed);
    }

    #[test]
    fn change_kind_from_heading_matches_substrings() {
        assert_eq!(ChangeKind::from_heading("Added"), ChangeKind::Added);
        assert_eq!(ChangeKind::from_heading("New Features"), ChangeKind::Added);
        assert_eq!(ChangeKind::from_heading("Bug Fixes"), ChangeKind::Fixed);
        assert_eq!(ChangeKind::from_heading("Breaking Changes"), ChangeKind::Breaking);
        assert_eq!(ChangeKind::from_heading("Security Advisory"), ChangeKind::Security);
        assert_eq!(ChangeKind::from_heading("Deprecations"), ChangeKind::Deprecated);
        assert_eq!(ChangeKind::from_heading("Removed APIs"), ChangeKind::Removed);
        assert_eq!(ChangeKind::from_heading("Misc"), ChangeKind::Changed);
    }
}
