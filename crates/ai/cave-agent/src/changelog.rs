// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-improvement, step 3: upstream changelog watch. Parses a
//! keep-a-changelog-style document into versioned, classified [`Entry`]s, and
//! answers "what changed since the version I'm pinned to?" and "does upgrading
//! cross a breaking change?".
//!
//! OpenJarvis upstream: `jarvis/improve/watch.py`. The network fetch of the
//! upstream CHANGELOG is owned by cave-changelog / cave-upstream; this module
//! is the pure parser + diff that consumes the fetched text.

use crate::error::{AgentError, Result};
use serde::{Deserialize, Serialize};

/// A `major.minor.patch` semantic version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// Parse `vX.Y.Z` or `X.Y.Z`. All three components are required.
    pub fn parse(s: &str) -> Result<Version> {
        let core = s.trim().strip_prefix('v').unwrap_or(s.trim());
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 {
            return Err(AgentError::Parse(format!(
                "version `{s}` must be major.minor.patch"
            )));
        }
        let num = |p: &str| -> Result<u64> {
            p.parse::<u64>()
                .map_err(|_| AgentError::Parse(format!("non-numeric version component in `{s}`")))
        };
        Ok(Version {
            major: num(parts[0])?,
            minor: num(parts[1])?,
            patch: num(parts[2])?,
        })
    }
}

/// The class of a changelog entry, inferred from its bullet prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Feature,
    Fix,
    Breaking,
    Other,
}

impl ChangeKind {
    fn classify(bullet: &str) -> (ChangeKind, &str) {
        let trimmed = bullet.trim();
        // split on the first ':' to separate the prefix from the summary
        if let Some((prefix, rest)) = trimmed.split_once(':') {
            let p = prefix.trim().to_lowercase();
            let kind = match p.as_str() {
                "feat" | "feature" => ChangeKind::Feature,
                "fix" | "bugfix" => ChangeKind::Fix,
                "breaking" | "break" => ChangeKind::Breaking,
                _ => ChangeKind::Other,
            };
            (kind, rest.trim())
        } else {
            (ChangeKind::Other, trimmed)
        }
    }
}

/// One classified changelog line under a version header.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub version: Version,
    pub kind: ChangeKind,
    pub summary: String,
}

/// Parse a changelog. Recognises `## vX.Y.Z` (or `## X.Y.Z`) headers and
/// `- prefix: summary` bullets beneath them. Unparseable headers reset the
/// "current version" to `None` so stray bullets are ignored.
pub fn parse_changelog(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut current: Option<Version> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("##") {
            current = Version::parse(rest.trim()).ok();
        } else if let Some(bullet) = t.strip_prefix('-') {
            if let Some(v) = current {
                let (kind, summary) = ChangeKind::classify(bullet);
                entries.push(Entry { version: v, kind, summary: summary.to_string() });
            }
        }
    }
    entries
}

/// Entries strictly newer than `current`.
pub fn actionable_since<'a>(entries: &'a [Entry], current: &Version) -> Vec<&'a Entry> {
    entries.iter().filter(|e| e.version > *current).collect()
}

/// Whether any entry strictly newer than `current` is a breaking change.
pub fn has_breaking_since(entries: &[Entry], current: &Version) -> bool {
    entries
        .iter()
        .any(|e| e.version > *current && e.kind == ChangeKind::Breaking)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bullet_without_colon_is_other() {
        let (k, s) = ChangeKind::classify("just some note");
        assert_eq!(k, ChangeKind::Other);
        assert_eq!(s, "just some note");
    }

    #[test]
    fn stray_bullets_without_header_are_ignored() {
        assert!(parse_changelog("- feat: orphan\n").is_empty());
    }
}
