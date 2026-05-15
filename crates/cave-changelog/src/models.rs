// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChangelogEntry {
    pub id: Uuid,
    pub version: String,
    pub date: NaiveDate,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Change {
    pub change_type: ChangeType,
    pub description: String,
    pub commit_sha: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Added,
    Changed,
    Deprecated,
    Removed,
    Fixed,
    Security,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn make_change(ct: ChangeType) -> Change {
        Change {
            change_type: ct,
            description: "some change".to_string(),
            commit_sha: Some("abc1234".to_string()),
            author: Some("alice".to_string()),
        }
    }

    #[test]
    fn test_change_roundtrip() {
        let c = make_change(ChangeType::Added);
        let json = serde_json::to_string(&c).unwrap();
        let decoded: Change = serde_json::from_str(&json).unwrap();
        assert_eq!(c, decoded);
    }

    #[test]
    fn test_change_type_serde_names() {
        let ct = ChangeType::Security;
        let json = serde_json::to_string(&ct).unwrap();
        assert_eq!(json, "\"security\"");
        let decoded: ChangeType = serde_json::from_str(&json).unwrap();
        assert_eq!(ct, decoded);
    }

    #[test]
    fn test_changelog_entry_roundtrip() {
        let entry = ChangelogEntry {
            id: Uuid::new_v4(),
            version: "1.2.3".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            changes: vec![make_change(ChangeType::Fixed), make_change(ChangeType::Added)],
        };
        let json = serde_json::to_string(&entry).unwrap();
        let decoded: ChangelogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, decoded);
    }

    #[test]
    fn test_change_no_author_roundtrip() {
        let c = Change {
            change_type: ChangeType::Removed,
            description: "dropped legacy endpoint".to_string(),
            commit_sha: None,
            author: None,
        };
        let json = serde_json::to_string(&c).unwrap();
        let decoded: Change = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.author, None);
        assert_eq!(decoded.commit_sha, None);
    }

    #[test]
    fn test_all_change_types_roundtrip() {
        for ct in [
            ChangeType::Added,
            ChangeType::Changed,
            ChangeType::Deprecated,
            ChangeType::Removed,
            ChangeType::Fixed,
            ChangeType::Security,
        ] {
            let json = serde_json::to_string(&ct).unwrap();
            let decoded: ChangeType = serde_json::from_str(&json).unwrap();
            assert_eq!(ct, decoded);
        }
    }

    // Silence unused import warning for DateTime<Utc> if not used in this file
    #[allow(dead_code)]
    fn _use_datetime(_: DateTime<Utc>) {}
}
