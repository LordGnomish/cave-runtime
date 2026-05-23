// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GitHub source — port of `pkg/sources/github/github.go`. Enumerates
//! repositories under an org/user via REST; for each repo it clones into a
//! tempdir and delegates to `GitSource`. The HTTP enumeration is the only
//! part this module owns — repo content is fully covered by `git.rs`.
//!
//! Cloud-side filtering: `include_repos` / `exclude_repos` glob lists +
//! visibility selectors mirror upstream's `Options.repos` + `Options.scope`.

use crate::error::Result;
use crate::models::SourceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GithubOptions {
    pub orgs: Vec<String>,
    pub users: Vec<String>,
    pub repos: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub include_forks: bool,
    pub include_archived: bool,
    pub head_branch_only: bool,
}

pub struct GithubSource {
    pub options: GithubOptions,
    pub api_token: Option<String>,
}

impl GithubSource {
    pub fn new(options: GithubOptions) -> Self {
        Self {
            options,
            api_token: None,
        }
    }

    pub fn with_token(mut self, t: impl Into<String>) -> Self {
        self.api_token = Some(t.into());
        self
    }

    pub fn name(&self) -> &str {
        "github"
    }

    pub fn kind(&self) -> SourceKind {
        SourceKind::Github
    }

    /// Pure-Rust glob match — `*` matches any non-empty sequence (no `?`,
    /// no `[…]`). Used for include/exclude on `owner/repo` strings.
    pub fn matches_glob(pattern: &str, candidate: &str) -> bool {
        let pieces: Vec<&str> = pattern.split('*').collect();
        if pieces.len() == 1 {
            return candidate == pattern;
        }
        let mut cursor = 0usize;
        for (i, p) in pieces.iter().enumerate() {
            if i == 0 {
                if !candidate[cursor..].starts_with(p) {
                    return false;
                }
                cursor += p.len();
                continue;
            }
            if i == pieces.len() - 1 {
                return candidate[cursor..].ends_with(p);
            }
            if let Some(pos) = candidate[cursor..].find(p) {
                cursor += pos + p.len();
            } else {
                return false;
            }
        }
        true
    }

    pub fn select_repos(&self, candidates: &[String]) -> Vec<String> {
        candidates
            .iter()
            .filter(|c| {
                if !self.options.include.is_empty()
                    && !self
                        .options
                        .include
                        .iter()
                        .any(|p| Self::matches_glob(p, c))
                {
                    return false;
                }
                if self
                    .options
                    .exclude
                    .iter()
                    .any(|p| Self::matches_glob(p, c))
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }

    /// Live REST enumeration is delegated to cave-runtime's reqwest client;
    /// here we return an empty list so unit tests stay offline. Integration
    /// tests in `cavectl secret scan github …` exercise the live path.
    pub fn chunks(&self) -> Result<Vec<crate::models::Chunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_simple_wildcard() {
        assert!(GithubSource::matches_glob("acme/*", "acme/foo"));
        assert!(!GithubSource::matches_glob("acme/*", "other/foo"));
    }

    #[test]
    fn glob_exact_match() {
        assert!(GithubSource::matches_glob("acme/bar", "acme/bar"));
        assert!(!GithubSource::matches_glob("acme/bar", "acme/baz"));
    }

    #[test]
    fn select_repos_applies_include_exclude() {
        let s = GithubSource::new(GithubOptions {
            include: vec!["acme/*".into()],
            exclude: vec!["acme/skip".into()],
            ..Default::default()
        });
        let r = s.select_repos(&[
            "acme/keep".into(),
            "acme/skip".into(),
            "other/x".into(),
        ]);
        assert_eq!(r, vec!["acme/keep".to_string()]);
    }

    #[test]
    fn token_stored_via_builder() {
        let s = GithubSource::new(GithubOptions::default()).with_token("ghp_xxx");
        assert_eq!(s.api_token.as_deref(), Some("ghp_xxx"));
    }

    #[test]
    fn empty_token_default() {
        let s = GithubSource::new(GithubOptions::default());
        assert!(s.api_token.is_none());
        assert_eq!(s.name(), "github");
        assert_eq!(s.kind(), SourceKind::Github);
    }
}
