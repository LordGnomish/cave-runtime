// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::ApiSpec;

/// Check if a version string looks like a valid semver
pub fn is_valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() >= 2 && parts.iter().all(|p| p.parse::<u32>().is_ok())
}

/// Check if upgrading from old_content to new_content introduces breaking changes.
/// Simple heuristic: if old content has a line starting with "/" that new content doesn't, it's a removal.
pub fn detect_breaking_changes(old_content: &str, new_content: &str) -> Vec<String> {
    let old_paths: std::collections::HashSet<&str> = old_content
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            (trimmed.starts_with('"') && l.contains("\"path\"")) || trimmed.starts_with('/')
        })
        .collect();
    let new_paths: std::collections::HashSet<&str> = new_content
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            (trimmed.starts_with('"') && l.contains("\"path\"")) || trimmed.starts_with('/')
        })
        .collect();
    old_paths.difference(&new_paths).map(|s| s.to_string()).collect()
}

/// Get the latest version from a list of specs (semver comparison)
pub fn latest_version<'a>(specs: &'a [ApiSpec]) -> Option<&'a ApiSpec> {
    specs.iter().max_by(|a, b| compare_versions(&a.version, &b.version))
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u32> {
        s.split('.').filter_map(|p| p.parse().ok()).collect()
    };
    let va = parse(a);
    let vb = parse(b);
    for i in 0..va.len().max(vb.len()) {
        let x = va.get(i).copied().unwrap_or(0);
        let y = vb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SpecFormat;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_spec(version: &str) -> ApiSpec {
        ApiSpec {
            id: Uuid::new_v4(),
            name: "test-api".to_string(),
            version: version.to_string(),
            format: SpecFormat::OpenApi3,
            content: "{}".to_string(),
            created_at: Utc::now(),
            published_by: "bot".to_string(),
        }
    }

    #[test]
    fn test_valid_version_semver() {
        assert!(is_valid_version("1.2.3"));
        assert!(is_valid_version("0.1.0"));
        assert!(is_valid_version("10.20.30"));
    }

    #[test]
    fn test_invalid_version() {
        assert!(!is_valid_version("v1.2"));
        assert!(!is_valid_version("latest"));
        assert!(!is_valid_version("1"));
        assert!(!is_valid_version("1.x.0"));
    }

    #[test]
    fn test_latest_version() {
        let specs = vec![make_spec("1.0.0"), make_spec("2.0.0"), make_spec("1.9.9")];
        let latest = latest_version(&specs).unwrap();
        assert_eq!(latest.version, "2.0.0");
    }

    #[test]
    fn test_latest_version_empty() {
        let specs: Vec<ApiSpec> = vec![];
        assert!(latest_version(&specs).is_none());
    }

    #[test]
    fn test_breaking_changes_empty() {
        let content = "/users\n/accounts\n";
        let changes = detect_breaking_changes(content, content);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_compare_versions_newer_wins() {
        assert_eq!(
            compare_versions("2.0.0", "1.9.9"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.9.9", "2.0.0"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_versions("1.0.0", "1.0.0"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_breaking_changes_detects_removed_paths() {
        let old = "/users\n/orders\n/items\n";
        let new = "/users\n/items\n";
        let changes = detect_breaking_changes(old, new);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].contains("/orders"));
    }
}
