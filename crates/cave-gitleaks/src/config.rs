// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Configuration loader and allowlist semantics.
//!
//! Mirrors `config/config.go` + `config/allowlist.go` upstream (`v8.29.1`).
//! Parses the upstream `.gitleaks.toml` schema (subset: `title`, `[allowlist]`,
//! `[[rules]]`, per-rule `[rules.allowlist]`).
//!
//! Out-of-scope (deferred): `extend`/`useDefault` config composition,
//! `stopwords` array, `condition = "AND"|"OR"` (always OR), tag-based rule
//! filtering, target-environment overrides.

use regex::Regex;
use serde::Deserialize;
use thiserror::Error;

use crate::rule::Rule;

/// Top-level config struct (deserialised from `.gitleaks.toml`).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub allowlist: RawAllowlist,
    #[serde(default, rename = "rules")]
    pub raw_rules: Vec<RawRule>,
    /// `[extend]` table — config composition over the built-in rule pack.
    #[serde(default)]
    pub extend: ExtendConfig,
    /// `stopwords` array — anti-FP post-match filter applied after detection.
    #[serde(default)]
    pub stopwords: Vec<String>,
}

/// `[extend]` block — controls config composition.
///
/// Mirrors `config/config.go::Config.Extend` upstream (`v8.29.1`).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtendConfig {
    /// If true, the built-in rule pack is included alongside the user's
    /// `[[rules]]`. User rule IDs that collide with built-ins override
    /// (last-write-wins).
    #[serde(default, rename = "useDefault")]
    pub use_default: bool,
    /// Per-rule disable list — IDs listed here are removed even if
    /// `use_default = true`.
    #[serde(default, rename = "disabledRules")]
    pub disabled_rules: Vec<String>,
}

/// Allowlist as represented on disk (regex strings, not compiled).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawAllowlist {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub regexes: Vec<String>,
    #[serde(default)]
    pub commits: Vec<String>,
    /// Match against the regex secret value, not the full line.
    /// Default `false` matches upstream behaviour.
    #[serde(default, rename = "regexTarget")]
    pub regex_target: String,
}

/// Rule as represented on disk.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawRule {
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub regex: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub entropy: Option<f64>,
    #[serde(default, rename = "secretGroup")]
    pub secret_group: Option<usize>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub allowlist: RawAllowlist,
}

/// Compiled allowlist — regexes ready to test.
///
/// Upstream type: `config.Allowlist`. The `commits` set lets callers
/// skip whole revisions during history scans (e.g. known-bad merges).
#[derive(Debug, Default, Clone)]
pub struct Allowlist {
    pub description: String,
    pub paths: Vec<Regex>,
    pub regexes: Vec<Regex>,
    pub commits: Vec<String>,
}

impl Allowlist {
    /// Returns true if `path` matches any allowlisted path regex.
    pub fn path_allowed(&self, path: &str) -> bool {
        self.paths.iter().any(|r| r.is_match(path))
    }

    /// Returns true if `secret` matches any allowlisted regex.
    pub fn secret_allowed(&self, secret: &str) -> bool {
        self.regexes.iter().any(|r| r.is_match(secret))
    }

    /// Returns true if `commit_sha` is allowlisted.
    pub fn commit_allowed(&self, commit_sha: &str) -> bool {
        self.commits.iter().any(|c| c.eq_ignore_ascii_case(commit_sha))
    }
}

impl TryFrom<RawAllowlist> for Allowlist {
    type Error = ConfigError;

    fn try_from(raw: RawAllowlist) -> Result<Self, Self::Error> {
        let paths = raw
            .paths
            .iter()
            .map(|p| Regex::new(p).map_err(ConfigError::BadPathRegex))
            .collect::<Result<Vec<_>, _>>()?;
        let regexes = raw
            .regexes
            .iter()
            .map(|p| Regex::new(p).map_err(ConfigError::BadSecretRegex))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            description: raw.description,
            paths,
            regexes,
            commits: raw.commits,
        })
    }
}

impl Config {
    /// Parse a `.gitleaks.toml` string. Does not compile rule regexes
    /// (callers go via [`Config::into_rules`]).
    pub fn parse(toml_text: &str) -> Result<Self, ConfigError> {
        toml::from_str(toml_text).map_err(ConfigError::Toml)
    }

    /// Compile rules + global allowlist. Consumes self.
    pub fn into_rules(self) -> Result<(Vec<Rule>, Allowlist), ConfigError> {
        let global = Allowlist::try_from(self.allowlist)?;
        let mut rules = Vec::with_capacity(self.raw_rules.len());
        for raw in self.raw_rules {
            let regex = Regex::new(&raw.regex).map_err(ConfigError::BadRuleRegex)?;
            let path = raw
                .path
                .as_deref()
                .map(Regex::new)
                .transpose()
                .map_err(ConfigError::BadPathRegex)?;
            let allowlist = Allowlist::try_from(raw.allowlist)?;
            rules.push(Rule {
                id: raw.id,
                description: raw.description,
                regex,
                path,
                entropy: raw.entropy,
                secret_group: raw.secret_group,
                keywords: raw.keywords,
                allowlist,
            });
        }
        Ok((rules, global))
    }

    /// Compile rules with `[extend]` resolution — when `extend.use_default`
    /// is true, the built-in rule pack is loaded first, then user rules
    /// override by id, then `extend.disabled_rules` strips any leftover.
    ///
    /// Mirrors upstream `config.Config.Translate` with `Extend.UseDefault`
    /// set.
    pub fn into_rules_with_extend(self) -> Result<(Vec<Rule>, Allowlist), ConfigError> {
        let disabled: std::collections::HashSet<String> =
            self.extend.disabled_rules.iter().cloned().collect();
        let use_default = self.extend.use_default;
        let (user_rules, global) = self.into_rules()?;

        let mut combined: Vec<Rule> = if use_default {
            crate::rule::builtin_rules()
        } else {
            Vec::new()
        };

        for user_rule in user_rules {
            // Last-write-wins by id.
            if let Some(pos) = combined.iter().position(|r| r.id == user_rule.id) {
                combined[pos] = user_rule;
            } else {
                combined.push(user_rule);
            }
        }

        combined.retain(|r| !disabled.contains(&r.id));
        Ok((combined, global))
    }
}

/// Config-loader error tree.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("malformed TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid rule regex: {0}")]
    BadRuleRegex(regex::Error),
    #[error("invalid path regex: {0}")]
    BadPathRegex(regex::Error),
    #[error("invalid secret-allowlist regex: {0}")]
    BadSecretRegex(regex::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_inline_rule() {
        let toml_text = r#"
title = "test"

[[rules]]
id          = "demo-key"
description = "demo"
regex       = "abc[0-9]+"
keywords    = ["abc"]
"#;
        let cfg = Config::parse(toml_text).expect("parse");
        let (rules, allow) = cfg.into_rules().expect("compile");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "demo-key");
        assert!(rules[0].regex.is_match("abc123"));
        assert!(allow.paths.is_empty());
    }

    #[test]
    fn parses_global_allowlist_paths_and_secrets() {
        let toml_text = r#"
[allowlist]
description = "ignore test fixtures"
paths       = ["testdata/.*", "\\.lock$"]
regexes     = ["EXAMPLE", "FAKE_[A-Z]+"]
commits     = ["deadbeefcafebabedeadbeefcafebabedeadbeef"]
"#;
        let cfg = Config::parse(toml_text).expect("parse");
        let (_, allow) = cfg.into_rules().expect("compile");
        assert!(allow.path_allowed("testdata/foo.txt"));
        assert!(allow.path_allowed("Cargo.lock"));
        assert!(!allow.path_allowed("src/main.rs"));
        assert!(allow.secret_allowed("AKIA_EXAMPLE_FAKE"));
        assert!(allow.commit_allowed("DEADBEEFCAFEBABEDEADBEEFCAFEBABEDEADBEEF"));
    }

    #[test]
    fn parses_per_rule_allowlist() {
        let toml_text = r#"
[[rules]]
id    = "x"
regex = "secret"

[rules.allowlist]
paths   = ["fixtures/.*"]
regexes = ["dummy"]
"#;
        let cfg = Config::parse(toml_text).expect("parse");
        let (rules, _) = cfg.into_rules().expect("compile");
        assert_eq!(rules.len(), 1);
        assert!(rules[0].allowlist.path_allowed("fixtures/a.txt"));
        assert!(rules[0].allowlist.secret_allowed("dummy"));
    }

    #[test]
    fn rejects_unknown_field() {
        // Charter no-backcompat gate: deny-unknown-fields catches typos
        // and rejects future upstream extensions until we map them.
        let toml_text = r#"
[[rules]]
id    = "x"
regex = "y"
bogus = "should fail"
"#;
        let err = Config::parse(toml_text).unwrap_err();
        let s = format!("{err}");
        assert!(s.contains("bogus") || s.contains("unknown"), "got {s}");
    }

    #[test]
    fn rejects_bad_rule_regex() {
        let toml_text = r#"
[[rules]]
id    = "x"
regex = "[unclosed"
"#;
        let cfg = Config::parse(toml_text).expect("parse");
        let err = cfg.into_rules().unwrap_err();
        assert!(matches!(err, ConfigError::BadRuleRegex(_)));
    }
}
