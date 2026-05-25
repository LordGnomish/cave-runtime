// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Custom regex rule builder.
//!
//! Mirrors TruffleHog's per-detector regex+keyword+verify schema so operators
//! can author site-specific detectors without recompiling. Rules ship as TOML
//! and load into `SecretDetector` records that the existing scan loop consumes.

use crate::detector::{SecretDetector, Severity};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRule {
    pub name: String,
    pub pattern: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default)]
    pub verify: bool,
}

fn default_severity() -> String {
    "medium".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomRulesFile {
    #[serde(default)]
    pub rules: Vec<CustomRule>,
}

#[derive(Debug, Clone)]
pub struct CustomDetector {
    pub name: String,
    pub pattern: Regex,
    pub keywords: Vec<String>,
    pub severity: Severity,
    pub verify: bool,
}

#[derive(Debug)]
pub enum BuildError {
    Toml(String),
    Regex { rule: String, error: String },
    EmptyName,
    EmptyPattern,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Toml(e) => write!(f, "toml parse error: {}", e),
            Self::Regex { rule, error } => write!(f, "regex compile failed for `{}`: {}", rule, error),
            Self::EmptyName => f.write_str("custom rule name must not be empty"),
            Self::EmptyPattern => f.write_str("custom rule pattern must not be empty"),
        }
    }
}

impl std::error::Error for BuildError {}

pub fn parse_severity(s: &str) -> Severity {
    match s.trim().to_ascii_lowercase().as_str() {
        "critical" | "crit" => Severity::Critical,
        "high" => Severity::High,
        "medium" | "med" => Severity::Medium,
        _ => Severity::Low,
    }
}

pub fn load_custom_rules(toml_text: &str) -> Result<Vec<CustomDetector>, BuildError> {
    let file: CustomRulesFile = toml::from_str(toml_text).map_err(|e| BuildError::Toml(e.to_string()))?;
    build(file.rules)
}

pub fn build(rules: Vec<CustomRule>) -> Result<Vec<CustomDetector>, BuildError> {
    rules.into_iter().map(build_one).collect()
}

pub fn build_one(rule: CustomRule) -> Result<CustomDetector, BuildError> {
    if rule.name.trim().is_empty() {
        return Err(BuildError::EmptyName);
    }
    if rule.pattern.trim().is_empty() {
        return Err(BuildError::EmptyPattern);
    }
    let pattern = Regex::new(&rule.pattern).map_err(|e| BuildError::Regex {
        rule: rule.name.clone(),
        error: e.to_string(),
    })?;
    Ok(CustomDetector {
        name: rule.name,
        pattern,
        keywords: rule.keywords,
        severity: parse_severity(&rule.severity),
        verify: rule.verify,
    })
}

/// Filter `lines` to only those that contain at least one of `keywords`.
/// Returns all line indices that pass the keyword pre-filter, or all indices
/// if `keywords` is empty (no pre-filter).
pub fn lines_passing_keywords(content: &str, keywords: &[String]) -> Vec<usize> {
    let lines: Vec<&str> = content.lines().collect();
    if keywords.is_empty() {
        return (0..lines.len()).collect();
    }
    let lower: Vec<String> = keywords.iter().map(|k| k.to_ascii_lowercase()).collect();
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let lcase = l.to_ascii_lowercase();
            if lower.iter().any(|k| lcase.contains(k)) {
                Some(i)
            } else {
                None
            }
        })
        .collect()
}

/// Convert a `CustomDetector` into a builtin `SecretDetector`.
///
/// Because `SecretDetector::name` is `&'static str`, we leak the rule name string
/// so the dispatcher can share a single representation with the builtins. The
/// leak happens once per rule at startup — negligible for the rule-set sizes
/// (≤ a few dozen) operators load.
pub fn into_secret_detector(d: CustomDetector) -> SecretDetector {
    let leaked: &'static str = Box::leak(d.name.into_boxed_str());
    SecretDetector {
        name: leaked,
        pattern: d.pattern,
        severity: d.severity,
        verify: d.verify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_severity_table() {
        assert_eq!(parse_severity("critical"), Severity::Critical);
        assert_eq!(parse_severity("Crit"), Severity::Critical);
        assert_eq!(parse_severity("HIGH"), Severity::High);
        assert_eq!(parse_severity("medium"), Severity::Medium);
        assert_eq!(parse_severity("Med"), Severity::Medium);
        assert_eq!(parse_severity("low"), Severity::Low);
        assert_eq!(parse_severity("nonsense"), Severity::Low);
    }

    #[test]
    fn load_simple_rule_from_toml() {
        let toml = r#"
            [[rules]]
            name = "company-token"
            pattern = "co_[A-Za-z0-9]{16}"
            keywords = ["co_"]
            severity = "high"
        "#;
        let rules = load_custom_rules(toml).expect("parse");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "company-token");
        assert!(rules[0].pattern.is_match("co_ABCDEFGHIJKLMNOP"));
        assert_eq!(rules[0].severity, Severity::High);
    }

    #[test]
    fn load_empty_rule_set() {
        let rules = load_custom_rules("").expect("parse");
        assert!(rules.is_empty());
    }

    #[test]
    fn rejects_invalid_regex() {
        let toml = r#"
            [[rules]]
            name = "bad"
            pattern = "([unclosed"
        "#;
        let err = load_custom_rules(toml).unwrap_err();
        assert!(matches!(err, BuildError::Regex { .. }));
    }

    #[test]
    fn rejects_empty_name() {
        let toml = r#"
            [[rules]]
            name = ""
            pattern = "x"
        "#;
        assert!(matches!(load_custom_rules(toml).unwrap_err(), BuildError::EmptyName));
    }

    #[test]
    fn rejects_empty_pattern() {
        let toml = r#"
            [[rules]]
            name = "n"
            pattern = ""
        "#;
        assert!(matches!(load_custom_rules(toml).unwrap_err(), BuildError::EmptyPattern));
    }

    #[test]
    fn keyword_filter_no_keywords_returns_all() {
        let lines = "a\nb\nc";
        let idxs = lines_passing_keywords(lines, &[]);
        assert_eq!(idxs, vec![0, 1, 2]);
    }

    #[test]
    fn keyword_filter_case_insensitive_substring() {
        let lines = "AWS=12\nfoo\nbar\nawsKey=99";
        let idxs = lines_passing_keywords(lines, &["aws".to_string()]);
        assert_eq!(idxs, vec![0, 3]);
    }

    #[test]
    fn into_secret_detector_preserves_pattern() {
        let rule = CustomRule {
            name: "n".to_string(),
            pattern: "abc[0-9]+".to_string(),
            keywords: vec![],
            severity: "critical".to_string(),
            verify: true,
        };
        let d = build_one(rule).expect("build");
        let sd = into_secret_detector(d);
        assert!(sd.pattern.is_match("abc1234"));
        assert_eq!(sd.severity, Severity::Critical);
        assert!(sd.verify);
    }

    #[test]
    fn default_severity_is_medium() {
        let toml = r#"
            [[rules]]
            name = "x"
            pattern = "y"
        "#;
        let rules = load_custom_rules(toml).expect("parse");
        assert_eq!(rules[0].severity, Severity::Medium);
    }

    #[test]
    fn invalid_toml_returns_toml_error() {
        let toml = "this is not = valid = toml";
        assert!(matches!(load_custom_rules(toml).unwrap_err(), BuildError::Toml(_)));
    }

    #[test]
    fn build_many_rules() {
        let toml = r#"
            [[rules]]
            name = "r1"
            pattern = "a"
            [[rules]]
            name = "r2"
            pattern = "b"
            [[rules]]
            name = "r3"
            pattern = "c"
        "#;
        let rules = load_custom_rules(toml).expect("parse");
        assert_eq!(rules.len(), 3);
    }
}
