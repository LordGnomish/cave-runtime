// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rule definitions and built-in catalogue.
//!
//! Mirrors `config/rule.go` upstream (`v8.29.1`). A `Rule` carries a
//! compiled regex, optional keyword pre-filter, optional entropy floor,
//! and a per-rule allowlist. Matches against the upstream `[[rules]]`
//! TOML schema verbatim — see [`crate::config`].

use regex::Regex;

use crate::config::Allowlist;

/// One Gitleaks-compatible detection rule.
///
/// Upstream type: `config.Rule` (`config/rule.go`). Fields kept in
/// upstream order so future rule TOML imports map structurally.
#[derive(Debug, Clone)]
pub struct Rule {
    /// Stable identifier (e.g. `"aws-access-token"`); appears in findings.
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// Compiled detection regex. Required.
    pub regex: Regex,
    /// Optional regex against the file path (skip if non-match).
    pub path: Option<Regex>,
    /// Optional Shannon-entropy floor (per upstream `Entropy float64`).
    pub entropy: Option<f64>,
    /// Which capture group (if any) the entropy is measured over.
    pub secret_group: Option<usize>,
    /// Keyword pre-filter; line must contain at least one (case-insensitive).
    /// Empty vector means "no pre-filter".
    pub keywords: Vec<String>,
    /// Rule-scoped allowlist (overrides global allowlist).
    pub allowlist: Allowlist,
}

impl Rule {
    /// Construct a Rule with regex-only matching.
    pub fn new(id: &str, description: &str, pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            id: id.to_string(),
            description: description.to_string(),
            regex: Regex::new(pattern)?,
            path: None,
            entropy: None,
            secret_group: None,
            keywords: Vec::new(),
            allowlist: Allowlist::default(),
        })
    }

    /// Add a keyword pre-filter list.
    pub fn with_keywords<I, S>(mut self, kws: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.keywords = kws.into_iter().map(Into::into).collect();
        self
    }

    /// Add an entropy floor over a specific capture group (1-based).
    pub fn with_entropy(mut self, floor: f64, group: usize) -> Self {
        self.entropy = Some(floor);
        self.secret_group = Some(group);
        self
    }

    /// Add a path-scope regex.
    pub fn with_path(mut self, path_pattern: &str) -> Result<Self, regex::Error> {
        self.path = Some(Regex::new(path_pattern)?);
        Ok(self)
    }

    /// Cheap keyword pre-filter: returns true if at least one keyword
    /// appears in `haystack` (case-insensitive ASCII). Mirrors the
    /// upstream "keyword" map check in `detect.detectRule`.
    pub fn keyword_matches(&self, haystack: &str) -> bool {
        if self.keywords.is_empty() {
            return true;
        }
        let lower = haystack.to_ascii_lowercase();
        self.keywords
            .iter()
            .any(|kw| lower.contains(&kw.to_ascii_lowercase()))
    }
}

/// Built-in rule catalogue. MVP scope covers 12 high-signal providers
/// drawn from the upstream `config/gitleaks.toml` master list, chosen
/// for cross-industry frequency. Out-of-scope rules (>700 upstream IDs)
/// are deferred to a follow-up "rule pack" import.
///
/// Naming follows upstream IDs verbatim so cave-gitleaks findings can be
/// joined to upstream Gitleaks dashboards.
pub fn builtin_rules() -> Vec<Rule> {
    vec![
        Rule::new(
            "aws-access-token",
            "AWS Access Token",
            r"\b(A3T[A-Z0-9]|AKIA|ASIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA|ASCA)[A-Z0-9]{16}\b",
        )
        .unwrap()
        .with_keywords(["AKIA", "ASIA", "AGPA", "AIDA", "AROA"]),
        Rule::new(
            "gcp-api-key",
            "GCP API Key",
            r"\bAIza[0-9A-Za-z\-_]{35}\b",
        )
        .unwrap()
        .with_keywords(["AIza"]),
        Rule::new(
            "azure-ad-client-secret",
            "Azure AD Client Secret",
            r"\b[a-zA-Z0-9_~.\-]{3}8Q~[a-zA-Z0-9_~.\-]{34}\b",
        )
        .unwrap()
        .with_keywords(["8Q~"]),
        Rule::new(
            "github-pat",
            "GitHub Personal Access Token",
            r"\bghp_[0-9A-Za-z]{36}\b",
        )
        .unwrap()
        .with_keywords(["ghp_"]),
        Rule::new(
            "github-oauth",
            "GitHub OAuth Access Token",
            r"\bgho_[0-9A-Za-z]{36}\b",
        )
        .unwrap()
        .with_keywords(["gho_"]),
        Rule::new(
            "github-fine-grained-pat",
            "GitHub Fine-Grained PAT",
            r"\bgithub_pat_[0-9A-Za-z_]{82}\b",
        )
        .unwrap()
        .with_keywords(["github_pat_"]),
        Rule::new(
            "slack-bot-token",
            "Slack Bot Token",
            r"\bxoxb-[0-9]{10,13}-[0-9]{10,13}-[a-zA-Z0-9]{24,34}\b",
        )
        .unwrap()
        .with_keywords(["xoxb-"]),
        Rule::new(
            "slack-user-token",
            "Slack User Token",
            r"\bxox[pe]-[0-9]{10,13}-[0-9]{10,13}-[0-9]{10,13}-[a-fA-F0-9]{32}\b",
        )
        .unwrap()
        .with_keywords(["xoxp-", "xoxe-"]),
        Rule::new(
            "stripe-secret-key",
            "Stripe Secret Key",
            r"\b(sk|rk)_(test|live)_[0-9a-zA-Z]{24,99}\b",
        )
        .unwrap()
        .with_keywords(["sk_test_", "sk_live_", "rk_test_", "rk_live_"]),
        Rule::new(
            "npm-access-token",
            "NPM Access Token",
            r"\bnpm_[A-Za-z0-9]{36}\b",
        )
        .unwrap()
        .with_keywords(["npm_"]),
        Rule::new(
            "jwt",
            "JSON Web Token",
            r"\beyJ[A-Za-z0-9_/+\-]{10,}\.eyJ[A-Za-z0-9_/+\-]{10,}\.[A-Za-z0-9_/+\-]{10,}\b",
        )
        .unwrap()
        .with_keywords(["eyJ"]),
        Rule::new(
            "private-key",
            "PEM Private Key",
            r"-----BEGIN ((EC|PGP|DSA|RSA|OPENSSH|ENCRYPTED|SSH2 ENCRYPTED) )?PRIVATE KEY( BLOCK)?-----",
        )
        .unwrap()
        .with_keywords(["BEGIN", "PRIVATE KEY"]),
        Rule::new(
            "generic-api-key",
            "Generic high-entropy API key",
            r#"(?i)(api[_\-]?key|apikey|secret|token|password|passwd|pwd)["'\s:=]{1,5}["']?([A-Za-z0-9_\-]{20,64})["']?"#,
        )
        .unwrap()
        .with_keywords([
            "api_key",
            "apikey",
            "api-key",
            "secret",
            "token",
            "password",
            "passwd",
        ])
        .with_entropy(3.5, 2),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_rules_cover_minimum_twelve_providers() {
        let rules = builtin_rules();
        assert!(
            rules.len() >= 12,
            "MVP requires >= 12 built-in providers (got {})",
            rules.len()
        );
        // IDs must be unique.
        let mut ids: Vec<_> = rules.iter().map(|r| r.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "rule IDs must be unique");
    }

    #[test]
    fn aws_rule_matches_real_format_and_rejects_garbage() {
        let r = builtin_rules()
            .into_iter()
            .find(|r| r.id == "aws-access-token")
            .unwrap();
        assert!(r.regex.is_match("AKIAIOSFODNN7EXAMPLE"));
        assert!(!r.regex.is_match("not-an-aws-key"));
    }

    #[test]
    fn github_pat_rule_is_anchored_to_prefix() {
        let r = builtin_rules()
            .into_iter()
            .find(|r| r.id == "github-pat")
            .unwrap();
        // Upstream regex requires 36 alphanumerics after `ghp_`.
        let real = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(real.len(), 4 + 36);
        assert!(r.regex.is_match(real));
        assert!(!r.regex.is_match("ghs_abcdefghijklmnopqrstuvwxyz0123"));
    }

    #[test]
    fn keyword_prefilter_skips_lines_without_match() {
        let r = builtin_rules()
            .into_iter()
            .find(|r| r.id == "github-pat")
            .unwrap();
        assert!(!r.keyword_matches("no token here"));
        assert!(r.keyword_matches("export GH=ghp_xxxxxxxxxxxxx"));
    }

    #[test]
    fn keyword_prefilter_is_case_insensitive() {
        let r = Rule::new("k", "k", "x").unwrap().with_keywords(["AKIA"]);
        assert!(r.keyword_matches("akia in lowercase line"));
    }

    #[test]
    fn empty_keywords_means_always_eligible() {
        let r = Rule::new("k", "k", "x").unwrap();
        assert!(r.keyword_matches("anything"));
    }

    #[test]
    fn private_key_rule_matches_pem_header() {
        let r = builtin_rules()
            .into_iter()
            .find(|r| r.id == "private-key")
            .unwrap();
        assert!(r.regex.is_match("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(r.regex.is_match("-----BEGIN PRIVATE KEY-----"));
        assert!(!r.regex.is_match("-----BEGIN CERTIFICATE-----"));
    }
}
