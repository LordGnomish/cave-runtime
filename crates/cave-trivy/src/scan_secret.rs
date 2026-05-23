// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Secret-detection scanner.
//!
//! Mirrors trivy's `pkg/fanal/secret` ruleset. Each rule has a category
//! (aws/gcp/azure/github/slack/jwt/etc), a severity, an opt-in keyword
//! (Aho–Corasick pre-filter) and a regex anchored against the matching
//! line. cave-trivy ships ~30 built-in rules. Custom rules can be added
//! via `SecretRules::push`.

use crate::models::{Secret, ScanResult};
use crate::scan_fs::FsTree;
use crate::severity::Severity;
use aho_corasick::AhoCorasick;
use regex::Regex;

#[derive(Debug, Clone)]
pub struct SecretRule {
    pub id: &'static str,
    pub category: &'static str,
    pub severity: Severity,
    pub keyword: &'static str,
    pub pattern: &'static str,
}

#[derive(Debug, Clone)]
pub struct SecretRules {
    rules: Vec<(SecretRule, Regex)>,
    pre: Option<AhoCorasick>,
}

impl SecretRules {
    pub fn new(rules: Vec<SecretRule>) -> Self {
        let mut compiled = Vec::new();
        let mut keys: Vec<&str> = Vec::new();
        for r in rules {
            if let Ok(re) = Regex::new(r.pattern) {
                if !r.keyword.is_empty() {
                    keys.push(r.keyword);
                }
                compiled.push((r, re));
            }
        }
        let pre = if keys.is_empty() {
            None
        } else {
            AhoCorasick::new(keys).ok()
        };
        Self { rules: compiled, pre }
    }

    pub fn default_rules() -> Self {
        Self::new(default_rule_set())
    }

    pub fn push(&mut self, r: SecretRule) {
        if let Ok(re) = Regex::new(r.pattern) {
            self.rules.push((r, re));
            let keys: Vec<&str> = self
                .rules
                .iter()
                .filter_map(|(rule, _)| if rule.keyword.is_empty() { None } else { Some(rule.keyword) })
                .collect();
            self.pre = if keys.is_empty() {
                None
            } else {
                AhoCorasick::new(keys).ok()
            };
        }
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn scan(&self, file: &str, body: &str) -> Vec<Secret> {
        let mut out = Vec::new();
        if let Some(pre) = &self.pre {
            if pre.find(body).is_none() {
                return out;
            }
        }
        for (i, line) in body.lines().enumerate() {
            for (rule, re) in &self.rules {
                if !rule.keyword.is_empty() && !line.contains(rule.keyword) {
                    continue;
                }
                if let Some(m) = re.find(line) {
                    out.push(Secret {
                        rule_id: rule.id.into(),
                        category: rule.category.into(),
                        severity: rule.severity,
                        start_line: i as u32 + 1,
                        end_line: i as u32 + 1,
                        match_text: m.as_str().to_string(),
                        file: file.to_string(),
                    });
                }
            }
        }
        out
    }
}

pub fn scan_secrets_in_tree(tree: &FsTree, rules: &SecretRules) -> Vec<Secret> {
    let mut out = Vec::new();
    for (path, text) in &tree.files {
        out.extend(rules.scan(path, text));
    }
    out
}

pub fn scan_secrets_report(name: &str, tree: &FsTree, rules: &SecretRules) -> crate::models::Report {
    let mut report = crate::models::Report::new(name, "filesystem");
    let secrets = scan_secrets_in_tree(tree, rules);
    if !secrets.is_empty() {
        report.results.push(ScanResult {
            target: name.into(),
            class: "secrets".into(),
            secrets,
            ..Default::default()
        });
    }
    report
}

fn default_rule_set() -> Vec<SecretRule> {
    vec![
        SecretRule { id: "aws-access-key-id", category: "aws", severity: Severity::Critical, keyword: "AKIA", pattern: r"AKIA[0-9A-Z]{16}" },
        SecretRule { id: "aws-secret-access-key", category: "aws", severity: Severity::Critical, keyword: "AWS_SECRET", pattern: r"AWS_SECRET[A-Z_]*\s*=\s*[A-Za-z0-9/+=]{20,}" },
        SecretRule { id: "gcp-service-account", category: "gcp", severity: Severity::Critical, keyword: "private_key_id", pattern: r#""private_key_id"\s*:\s*"[a-f0-9]{20,}""# },
        SecretRule { id: "azure-storage-key", category: "azure", severity: Severity::High, keyword: "AccountKey", pattern: r"AccountKey=[A-Za-z0-9+/=]{30,}" },
        SecretRule { id: "github-pat", category: "github", severity: Severity::Critical, keyword: "ghp_", pattern: r"ghp_[A-Za-z0-9]{30,}" },
        SecretRule { id: "github-fine-grained-pat", category: "github", severity: Severity::Critical, keyword: "github_pat_", pattern: r"github_pat_[A-Za-z0-9_]{30,}" },
        SecretRule { id: "github-oauth", category: "github", severity: Severity::High, keyword: "gho_", pattern: r"gho_[A-Za-z0-9]{30,}" },
        SecretRule { id: "slack-bot-token", category: "slack", severity: Severity::High, keyword: "xoxb-", pattern: r"xoxb-[0-9A-Za-z\-]{20,}" },
        SecretRule { id: "slack-webhook", category: "slack", severity: Severity::Medium, keyword: "hooks.slack.com", pattern: r"https://hooks\.slack\.com/services/[A-Z0-9/]{20,}" },
        SecretRule { id: "stripe-key", category: "stripe", severity: Severity::Critical, keyword: "sk_live_", pattern: r"sk_live_[0-9A-Za-z]{20,}" },
        SecretRule { id: "stripe-test", category: "stripe", severity: Severity::Low, keyword: "sk_test_", pattern: r"sk_test_[0-9A-Za-z]{20,}" },
        SecretRule { id: "pkcs8-private-key", category: "pem", severity: Severity::Critical, keyword: "BEGIN PRIVATE KEY", pattern: r"-----BEGIN PRIVATE KEY-----" },
        SecretRule { id: "rsa-private-key", category: "pem", severity: Severity::Critical, keyword: "BEGIN RSA PRIVATE KEY", pattern: r"-----BEGIN RSA PRIVATE KEY-----" },
        SecretRule { id: "openssh-private-key", category: "pem", severity: Severity::Critical, keyword: "BEGIN OPENSSH PRIVATE KEY", pattern: r"-----BEGIN OPENSSH PRIVATE KEY-----" },
        SecretRule { id: "jwt-token", category: "jwt", severity: Severity::Medium, keyword: "eyJ", pattern: r"eyJ[A-Za-z0-9_-]+?\.[A-Za-z0-9_-]+?\.[A-Za-z0-9_-]+" },
        SecretRule { id: "generic-api-key", category: "generic", severity: Severity::Medium, keyword: "api_key", pattern: r#"(?i)api[_-]?key\s*[=:]\s*['"]?[A-Za-z0-9]{20,}['"]?"# },
        SecretRule { id: "generic-secret", category: "generic", severity: Severity::Low, keyword: "secret", pattern: r#"(?i)secret\s*[=:]\s*['"]?[A-Za-z0-9_\-]{16,}['"]?"# },
        SecretRule { id: "twilio-key", category: "twilio", severity: Severity::High, keyword: "SK", pattern: r"SK[0-9a-fA-F]{32}" },
        SecretRule { id: "sendgrid-key", category: "sendgrid", severity: Severity::High, keyword: "SG.", pattern: r"SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}" },
        SecretRule { id: "mailgun-key", category: "mailgun", severity: Severity::High, keyword: "key-", pattern: r"key-[a-z0-9]{32}" },
        SecretRule { id: "datadog-api", category: "datadog", severity: Severity::High, keyword: "dd_api_key", pattern: r#"(?i)dd[_-]?api[_-]?key\s*[=:]\s*['"]?[a-f0-9]{32}['"]?"# },
        SecretRule { id: "newrelic-key", category: "newrelic", severity: Severity::High, keyword: "NRAK-", pattern: r"NRAK-[A-Z0-9]{27}" },
        SecretRule { id: "facebook-token", category: "facebook", severity: Severity::Medium, keyword: "EAACEdEose0cBA", pattern: r"EAACEdEose0cBA[A-Za-z0-9]+" },
        SecretRule { id: "discord-token", category: "discord", severity: Severity::High, keyword: "MTA", pattern: r"M[A-Za-z\d]{23}\.[A-Za-z\d]{6}\.[A-Za-z\d_-]{27}" },
        SecretRule { id: "gitlab-pat", category: "gitlab", severity: Severity::Critical, keyword: "glpat-", pattern: r"glpat-[A-Za-z0-9_\-]{20,}" },
        SecretRule { id: "npm-token", category: "npm", severity: Severity::High, keyword: "npm_", pattern: r"npm_[A-Za-z0-9]{36}" },
        SecretRule { id: "dockerhub-pat", category: "docker", severity: Severity::High, keyword: "dckr_pat_", pattern: r"dckr_pat_[A-Za-z0-9_\-]{20,}" },
        SecretRule { id: "circle-token", category: "circleci", severity: Severity::Medium, keyword: "CIRCLE", pattern: r#"(?i)circle[_-]?token\s*[=:]\s*['"]?[a-f0-9]{40}['"]?"# },
        SecretRule { id: "heroku-key", category: "heroku", severity: Severity::Medium, keyword: "heroku", pattern: r#"(?i)heroku[_-]?api[_-]?key\s*[=:]\s*['"]?[a-f0-9-]{36}['"]?"# },
        SecretRule { id: "shopify-token", category: "shopify", severity: Severity::High, keyword: "shpat_", pattern: r"shpat_[a-fA-F0-9]{32}" },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aws_key_detected() {
        let s = SecretRules::default_rules();
        let v = s.scan("f", "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE");
        assert!(v.iter().any(|x| x.rule_id == "aws-access-key-id"));
    }

    #[test]
    fn github_pat_detected() {
        let s = SecretRules::default_rules();
        let v = s.scan("f", "token: ghp_abcdefghijklmnopqrstuvwxyz0123456789");
        assert!(v.iter().any(|x| x.rule_id == "github-pat"));
        assert_eq!(v[0].severity, Severity::Critical);
    }

    #[test]
    fn private_key_detected() {
        let s = SecretRules::default_rules();
        let v = s.scan("f", "-----BEGIN PRIVATE KEY-----\nMIIE\n-----END PRIVATE KEY-----");
        assert!(v.iter().any(|x| x.rule_id == "pkcs8-private-key"));
    }

    #[test]
    fn aho_corasick_prefilter_skips_clean_files() {
        let s = SecretRules::default_rules();
        let v = s.scan("README.md", "just some boring text\nwith nothing secret");
        assert!(v.is_empty());
    }

    #[test]
    fn line_numbers_recorded() {
        let s = SecretRules::default_rules();
        let v = s.scan("f", "harmless\nAWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\nmore");
        assert_eq!(v[0].start_line, 2);
    }

    #[test]
    fn slack_bot_token() {
        let s = SecretRules::default_rules();
        let v = s.scan("conf", "token=xoxb-12345-abcdef-ghijkl-mnopqr-stuvwxyz0123456789");
        assert!(v.iter().any(|x| x.category == "slack"));
    }

    #[test]
    fn jwt_token() {
        let s = SecretRules::default_rules();
        let v = s.scan(
            "f",
            "header eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
        );
        assert!(v.iter().any(|x| x.rule_id == "jwt-token"));
    }

    #[test]
    fn tree_scan() {
        let t = FsTree::default()
            .push("clean.txt", "nothing")
            .push("bad.env", "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE");
        let v = scan_secrets_in_tree(&t, &SecretRules::default_rules());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].file, "bad.env");
    }

    #[test]
    fn default_rules_count() {
        let s = SecretRules::default_rules();
        assert!(s.len() >= 25);
    }

    #[test]
    fn custom_rule_push() {
        let mut s = SecretRules::default_rules();
        let before = s.len();
        s.push(SecretRule {
            id: "x",
            category: "x",
            severity: Severity::Low,
            keyword: "MYSECRET",
            pattern: "MYSECRET=[a-z]+",
        });
        assert_eq!(s.len(), before + 1);
    }
}
