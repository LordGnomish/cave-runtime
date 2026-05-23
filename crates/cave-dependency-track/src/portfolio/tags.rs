// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tag normalisation + index — `org.dependencytrack.model.Tag` parity.

use std::collections::HashMap;
use uuid::Uuid;

/// Tags are lowercased and trimmed.  Whitespace runs collapse to a single
/// hyphen so `"Production Build"` and `"production build"` map to the same
/// canonical `"production-build"` slug — matches `TagDao#normalize`.
pub fn normalize_tag(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_space = false;
    for c in raw.trim().chars() {
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push('-');
            }
            prev_space = true;
        } else {
            out.push(c.to_ascii_lowercase());
            prev_space = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[derive(Default, Debug)]
pub struct TagIndex {
    by_tag: HashMap<String, Vec<Uuid>>,
}

impl TagIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, tag: &str, project: Uuid) {
        let key = normalize_tag(tag);
        if key.is_empty() {
            return;
        }
        let entry = self.by_tag.entry(key).or_default();
        if !entry.contains(&project) {
            entry.push(project);
        }
    }

    pub fn remove(&mut self, tag: &str, project: Uuid) {
        let key = normalize_tag(tag);
        if let Some(v) = self.by_tag.get_mut(&key) {
            v.retain(|u| u != &project);
            if v.is_empty() {
                self.by_tag.remove(&key);
            }
        }
    }

    pub fn projects(&self, tag: &str) -> Vec<Uuid> {
        self.by_tag
            .get(&normalize_tag(tag))
            .cloned()
            .unwrap_or_default()
    }

    pub fn tags(&self) -> Vec<String> {
        let mut v: Vec<_> = self.by_tag.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn cardinality(&self, tag: &str) -> usize {
        self.by_tag
            .get(&normalize_tag(tag))
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_and_hyphenates() {
        assert_eq!(normalize_tag("  Production Build  "), "production-build");
        assert_eq!(normalize_tag("DEV"), "dev");
        assert_eq!(normalize_tag("  multi   word "), "multi-word");
        assert_eq!(normalize_tag(""), "");
        assert_eq!(normalize_tag("    "), "");
    }

    #[test]
    fn empty_tag_is_ignored() {
        let mut idx = TagIndex::new();
        idx.add("", Uuid::new_v4());
        assert!(idx.tags().is_empty());
    }

    #[test]
    fn add_then_lookup_canonical() {
        let mut idx = TagIndex::new();
        let p = Uuid::new_v4();
        idx.add("Prod", p);
        idx.add(" prod ", p); // duplicate after normalise → dedup
        assert_eq!(idx.projects("PROD"), vec![p]);
        assert_eq!(idx.cardinality("prod"), 1);
    }

    #[test]
    fn remove_drops_empty_entry() {
        let mut idx = TagIndex::new();
        let p = Uuid::new_v4();
        idx.add("dev", p);
        idx.remove("DEV", p);
        assert!(idx.tags().is_empty());
    }

    #[test]
    fn tags_sorted_alphabetically() {
        let mut idx = TagIndex::new();
        idx.add("zeta", Uuid::new_v4());
        idx.add("alpha", Uuid::new_v4());
        assert_eq!(idx.tags(), vec!["alpha", "zeta"]);
    }
}
