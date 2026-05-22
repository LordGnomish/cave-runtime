// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/model/Context.java
//
//! ZAP context model — an ordered list of include / exclude regex
//! patterns that decides whether a URL is "in scope" for active
//! scanning. ZAP semantics: a URL is in-scope if any include matches
//! AND no exclude matches. Empty include list ⇒ everything matches
//! (subject to excludes). Mirrors `Context.isInContext()`.

use regex::Regex;

#[derive(Debug)]
pub struct Context {
    pub name: String,
    includes: Vec<Regex>,
    excludes: Vec<Regex>,
}

impl Context {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            includes: Vec::new(),
            excludes: Vec::new(),
        }
    }

    /// Append an include regex (anchor-free; caller may add `^…$`).
    pub fn include(&mut self, pattern: &str) -> Result<&mut Self, regex::Error> {
        self.includes.push(Regex::new(pattern)?);
        Ok(self)
    }

    pub fn exclude(&mut self, pattern: &str) -> Result<&mut Self, regex::Error> {
        self.excludes.push(Regex::new(pattern)?);
        Ok(self)
    }

    pub fn is_in_scope(&self, url: &str) -> bool {
        if self.excludes.iter().any(|r| r.is_match(url)) {
            return false;
        }
        if self.includes.is_empty() {
            return true;
        }
        self.includes.iter().any(|r| r.is_match(url))
    }

    /// Filter a list of candidate URLs down to the in-scope subset.
    pub fn filter<'a>(&self, urls: &'a [String]) -> Vec<&'a String> {
        urls.iter().filter(|u| self.is_in_scope(u)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_context_matches_everything() {
        let c = Context::new("default");
        assert!(c.is_in_scope("http://anywhere.test/x"));
    }

    #[test]
    fn include_pattern_gates() {
        let mut c = Context::new("acme");
        c.include(r"^https://acme\.test/").unwrap();
        assert!(c.is_in_scope("https://acme.test/api"));
        assert!(!c.is_in_scope("https://other.test/api"));
    }

    #[test]
    fn exclude_overrides_include() {
        let mut c = Context::new("acme");
        c.include(r"^https://acme\.test/").unwrap();
        c.exclude(r"/admin/").unwrap();
        assert!(c.is_in_scope("https://acme.test/users"));
        assert!(!c.is_in_scope("https://acme.test/admin/secret"));
    }

    #[test]
    fn multiple_includes_any_match() {
        let mut c = Context::new("multi");
        c.include(r"acme\.test").unwrap();
        c.include(r"corp\.test").unwrap();
        assert!(c.is_in_scope("https://acme.test/x"));
        assert!(c.is_in_scope("https://corp.test/y"));
        assert!(!c.is_in_scope("https://evil.test/z"));
    }

    #[test]
    fn filter_returns_in_scope_subset() {
        let mut c = Context::new("acme");
        c.include(r"acme\.test").unwrap();
        let urls = vec![
            "https://acme.test/a".to_string(),
            "https://other.test/b".to_string(),
            "https://acme.test/c".to_string(),
        ];
        let kept = c.filter(&urls);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn bad_pattern_errors() {
        let mut c = Context::new("bad");
        assert!(c.include("[unterminated").is_err());
    }
}
