// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Resource / namespace include-exclude filtering engine.
//!
//! Faithful line-port of Velero `pkg/util/collections/includes_excludes.go`
//! (`IncludesExcludes`, `ShouldInclude`, `IncludeEverything`, `ValidateIncludesExcludes`).
//! Upstream uses `github.com/gobwas/glob` for pattern matching; we port a small,
//! dependency-free glob matcher (`*` = any sequence, `?` = any single char) in-crate
//! to preserve the same wildcard semantics exercised by Velero's TestShouldInclude
//! (`*.bar` matches `foo.bar` but not `bar.foo`).
//!
//! This is the pure in-memory selection logic the `Backup`/`Restore` models already
//! carry include/exclude fields for; it does not touch persistence, networking, the
//! plugin RPC layer, or CRD generation.

use std::collections::BTreeSet;

/// Glob-match a single pattern against a string.
///
/// Ports the subset of `gobwas/glob` semantics Velero relies on: `*` matches any
/// (possibly empty) run of characters, `?` matches exactly one character, and every
/// other character matches literally. There are no path separators, so `*` spans `.`
/// (matching upstream's default `glob.Compile(item)` with no separator argument).
fn glob_match(pattern: &str, s: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = s.chars().collect();
    // Classic two-pointer wildcard match with backtracking on the last `*`.
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A set of glob patterns, mirroring Velero's `globStringSet`.
#[derive(Debug, Clone, Default)]
struct GlobStringSet {
    items: BTreeSet<String>,
}

impl GlobStringSet {
    fn insert(&mut self, items: Vec<String>) {
        for i in items {
            self.items.insert(i);
        }
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    fn has(&self, s: &str) -> bool {
        self.items.contains(s)
    }

    fn list(&self) -> Vec<String> {
        self.items.iter().cloned().collect()
    }

    /// `match` in upstream: true if any contained pattern globs `match`.
    fn matches(&self, m: &str) -> bool {
        self.items.iter().any(|item| glob_match(item, m))
    }
}

/// Determines which items should be included given a set of includes and excludes.
///
/// `*` in the includes list means "include everything", but it is not valid in the
/// excludes list. Port of Velero's `IncludesExcludes` struct.
#[derive(Debug, Clone, Default)]
pub struct IncludesExcludes {
    includes: GlobStringSet,
    excludes: GlobStringSet,
}

impl IncludesExcludes {
    /// Returns a new, empty `IncludesExcludes` (port of `NewIncludesExcludes`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds items to the includes list. `*` is a wildcard meaning "include everything".
    pub fn includes(&mut self, includes: Vec<String>) -> &mut Self {
        self.includes.insert(includes);
        self
    }

    /// Returns the items in the includes list (sorted, like upstream `sets.String.List`).
    pub fn get_includes(&self) -> Vec<String> {
        self.includes.list()
    }

    /// Adds items to the excludes list.
    pub fn excludes(&mut self, excludes: Vec<String>) -> &mut Self {
        self.excludes.insert(excludes);
        self
    }

    /// Returns the items in the excludes list.
    pub fn get_excludes(&self) -> Vec<String> {
        self.excludes.list()
    }

    /// Returns whether the specified item should be included or not. Everything in the
    /// includes list except those items in the excludes list should be included.
    ///
    /// Port of `IncludesExcludes.ShouldInclude`.
    pub fn should_include(&self, s: &str) -> bool {
        if self.excludes.matches(s) {
            return false;
        }
        // len==0 means include everything
        self.includes.len() == 0 || self.includes.has("*") || self.includes.matches(s)
    }

    /// Returns true if the includes list is empty or `*` and the excludes list is empty.
    ///
    /// Port of `IncludesExcludes.IncludeEverything`.
    pub fn include_everything(&self) -> bool {
        self.excludes.len() == 0
            && (self.includes.len() == 0 || (self.includes.len() == 1 && self.includes.has("*")))
    }
}

/// Checks provided lists of included and excluded items to ensure they are a valid
/// set of `IncludesExcludes` data. Returns a list of human-readable error strings
/// (empty when valid).
///
/// Port of Velero's `ValidateIncludesExcludes`.
pub fn validate_includes_excludes(includes_list: &[String], excludes_list: &[String]) -> Vec<String> {
    let mut errs = Vec::new();

    let includes: BTreeSet<&String> = includes_list.iter().collect();
    let excludes: BTreeSet<&String> = excludes_list.iter().collect();

    if includes.len() > 1 && includes.contains(&"*".to_string()) {
        errs.push("includes list must either contain '*' only, or a non-empty list of items".to_string());
    }

    if excludes.contains(&"*".to_string()) {
        errs.push("excludes list cannot contain '*'".to_string());
    }

    // Iterate excludes in sorted order (BTreeSet) to match upstream `sets.String.List`.
    for itm in &excludes {
        if includes.contains(itm) {
            errs.push(format!(
                "excludes list cannot contain an item in the includes list: {itm}"
            ));
        }
    }

    errs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_star_spans_dot() {
        assert!(glob_match("*.bar", "foo.bar"));
        assert!(!glob_match("*.bar", "bar.foo"));
        assert!(glob_match("*", "anything.at.all"));
    }

    #[test]
    fn glob_question_single_char() {
        assert!(glob_match("ba?", "bar"));
        assert!(!glob_match("ba?", "barn"));
    }

    #[test]
    fn glob_literal() {
        assert!(glob_match("foo", "foo"));
        assert!(!glob_match("foo", "foobar"));
    }
}
