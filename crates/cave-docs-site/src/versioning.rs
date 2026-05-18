// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::error::*;
use crate::types::*;
use std::collections::HashMap;

pub struct VersionManager;

impl VersionManager {
    /// Diff two versions: which pages were added/removed/changed
    pub fn diff(old_pages: &[Page], new_pages: &[Page]) -> VersionDiff {
        let old_map: HashMap<&str, &Page> =
            old_pages.iter().map(|p| (p.slug.as_str(), p)).collect();
        let new_map: HashMap<&str, &Page> =
            new_pages.iter().map(|p| (p.slug.as_str(), p)).collect();

        let mut added = vec![];
        let mut removed = vec![];
        let mut modified = vec![];
        let mut unchanged = vec![];

        for (slug, new_page) in &new_map {
            if let Some(old_page) = old_map.get(slug) {
                if old_page.markdown_content != new_page.markdown_content {
                    modified.push(slug.to_string());
                } else {
                    unchanged.push(slug.to_string());
                }
            } else {
                added.push(slug.to_string());
            }
        }

        for slug in old_map.keys() {
            if !new_map.contains_key(slug) {
                removed.push(slug.to_string());
            }
        }

        VersionDiff {
            added,
            removed,
            modified,
            unchanged,
        }
    }

    /// Copy all pages from one version to another
    pub fn clone_version(pages: &[Page], from_version: &str, to_version: &str) -> Vec<Page> {
        pages
            .iter()
            .filter(|p| p.version == from_version)
            .map(|p| {
                let mut cloned = p.clone();
                cloned.id = uuid::Uuid::new_v4().to_string();
                cloned.version = to_version.to_string();
                cloned.created_at = chrono::Utc::now();
                cloned.updated_at = chrono::Utc::now();
                cloned
            })
            .collect()
    }

    /// Merge changes from one version into another (simple: take newer updated_at)
    pub fn merge(base: &[Page], incoming: &[Page]) -> Vec<Page> {
        let mut result: HashMap<String, Page> =
            base.iter().map(|p| (p.slug.clone(), p.clone())).collect();
        for page in incoming {
            let entry = result.entry(page.slug.clone()).or_insert_with(|| page.clone());
            if page.updated_at > entry.updated_at {
                *entry = page.clone();
            }
        }
        result.into_values().collect()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
    pub unchanged: Vec<String>,
}

#[allow(dead_code)]
fn validate_version_name(name: &str) -> DocsResult<()> {
    if name.is_empty() {
        return Err(DocsError::InvalidSlug(name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_page(slug: &str, content: &str, version: &str) -> Page {
        Page {
            id: uuid::Uuid::new_v4().to_string(),
            space_id: "s1".to_string(),
            slug: slug.to_string(),
            title: slug.to_string(),
            markdown_content: content.to_string(),
            html_content: None,
            group_id: None,
            parent_id: None,
            order: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            version: version.to_string(),
            metadata: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn version_diff() {
        let old_pages = vec![
            make_page("intro", "# Introduction", "v1"),
            make_page("guide", "# Guide", "v1"),
            make_page("removed-page", "# Old", "v1"),
        ];
        let new_pages = vec![
            make_page("intro", "# Introduction Updated", "v2"), // modified
            make_page("guide", "# Guide", "v2"),                // unchanged
            make_page("new-page", "# New", "v2"),               // added
        ];
        let diff = VersionManager::diff(&old_pages, &new_pages);
        assert!(diff.added.contains(&"new-page".to_string()));
        assert!(diff.removed.contains(&"removed-page".to_string()));
        assert!(diff.modified.contains(&"intro".to_string()));
        assert!(diff.unchanged.contains(&"guide".to_string()));
    }
}
