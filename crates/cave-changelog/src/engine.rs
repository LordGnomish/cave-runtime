// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{Change, ChangeType, ChangelogEntry};

/// Parse a conventional commit message into a ChangeType and description.
/// Format: "type(scope): description"
/// feat → Added, fix → Fixed, security/sec → Security, deprecated/deprecate → Deprecated,
/// remove/revert → Removed, chore/refactor/style → Changed
pub fn parse_commit(message: &str) -> Option<(ChangeType, String)> {
    let lower = message.to_lowercase();
    let desc = message
        .splitn(2, ':')
        .nth(1)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if lower.starts_with("feat") {
        Some((ChangeType::Added, desc))
    } else if lower.starts_with("fix") {
        Some((ChangeType::Fixed, desc))
    } else if lower.starts_with("security") || lower.starts_with("sec") {
        Some((ChangeType::Security, desc))
    } else if lower.starts_with("deprecated") || lower.starts_with("deprecate") {
        Some((ChangeType::Deprecated, desc))
    } else if lower.starts_with("remove") || lower.starts_with("revert") {
        Some((ChangeType::Removed, desc))
    } else if lower.starts_with("chore")
        || lower.starts_with("refactor")
        || lower.starts_with("style")
    {
        Some((ChangeType::Changed, desc))
    } else {
        None
    }
}

/// Filter changes by type
pub fn filter_by_type<'a>(changes: &'a [Change], ct: &ChangeType) -> Vec<&'a Change> {
    changes.iter().filter(|c| &c.change_type == ct).collect()
}

/// Count changes by type
pub fn count_by_type(changes: &[Change]) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();
    for c in changes {
        let key = format!("{:?}", c.change_type);
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

/// Sort changelog entries by date descending (newest first)
pub fn sort_by_version(mut entries: Vec<ChangelogEntry>) -> Vec<ChangelogEntry> {
    entries.sort_by(|a, b| b.date.cmp(&a.date));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn make_change(ct: ChangeType, desc: &str) -> Change {
        Change {
            change_type: ct,
            description: desc.to_string(),
            commit_sha: None,
            author: None,
        }
    }

    fn make_entry(version: &str, date: NaiveDate) -> ChangelogEntry {
        ChangelogEntry {
            id: Uuid::new_v4(),
            version: version.to_string(),
            date,
            changes: vec![],
        }
    }

    #[test]
    fn test_parse_commit_feat() {
        let result = parse_commit("feat: add new feature");
        assert!(result.is_some());
        let (ct, desc) = result.unwrap();
        assert_eq!(ct, ChangeType::Added);
        assert_eq!(desc, "add new feature");
    }

    #[test]
    fn test_parse_commit_fix() {
        let result = parse_commit("fix(auth): resolve login issue");
        assert!(result.is_some());
        let (ct, desc) = result.unwrap();
        assert_eq!(ct, ChangeType::Fixed);
        assert_eq!(desc, "resolve login issue");
    }

    #[test]
    fn test_parse_commit_unknown() {
        let result = parse_commit("wip: working on something");
        assert!(result.is_none());
    }

    #[test]
    fn test_filter_by_type() {
        let changes = vec![
            make_change(ChangeType::Added, "new endpoint"),
            make_change(ChangeType::Fixed, "bug fix"),
            make_change(ChangeType::Added, "another feature"),
            make_change(ChangeType::Security, "patch cve"),
        ];
        let added = filter_by_type(&changes, &ChangeType::Added);
        assert_eq!(added.len(), 2);
        for c in &added {
            assert_eq!(c.change_type, ChangeType::Added);
        }
    }

    #[test]
    fn test_count_by_type() {
        let changes = vec![
            make_change(ChangeType::Fixed, "a"),
            make_change(ChangeType::Fixed, "b"),
            make_change(ChangeType::Added, "c"),
        ];
        let counts = count_by_type(&changes);
        assert_eq!(counts.get("Fixed").copied().unwrap_or(0), 2);
        assert_eq!(counts.get("Added").copied().unwrap_or(0), 1);
    }

    #[test]
    fn test_sort_by_version_newest_first() {
        let entries = vec![
            make_entry("1.0.0", NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            make_entry("1.2.0", NaiveDate::from_ymd_opt(2026, 3, 15).unwrap()),
            make_entry("1.1.0", NaiveDate::from_ymd_opt(2026, 2, 10).unwrap()),
        ];
        let sorted = sort_by_version(entries);
        assert_eq!(sorted[0].version, "1.2.0");
        assert_eq!(sorted[1].version, "1.1.0");
        assert_eq!(sorted[2].version, "1.0.0");
    }

    #[test]
    fn test_parse_commit_chore() {
        let result = parse_commit("chore: update dependencies");
        assert!(result.is_some());
        let (ct, _) = result.unwrap();
        assert_eq!(ct, ChangeType::Changed);
    }

    #[test]
    fn test_parse_commit_security() {
        let result = parse_commit("security: patch CVE-2026-1234");
        assert!(result.is_some());
        let (ct, _) = result.unwrap();
        assert_eq!(ct, ChangeType::Security);
    }
}
