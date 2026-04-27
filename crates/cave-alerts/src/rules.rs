//! Loader for the YAML alerting-rule catalog under `crates/cave-alerts/rules/`.
//!
//! Each YAML file describes one crate's eight standard SLO alerts. The
//! loader validates structure and surfaces them as `RuleSpec` records that
//! the runtime can compile into PromQL evaluators (out of scope for this
//! crate — we just expose the parsed shape).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleFile {
    pub crate_: Option<String>,
    pub group: String,
    pub alerts: Vec<RuleSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSpec {
    pub alert: String,
    pub expr: String,
    /// Pending duration, e.g. "5m", "30s".
    #[serde(default)]
    pub r#for: Option<String>,
    pub severity: Severity,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub annotations: Annotations,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotations {
    pub summary: String,
    #[serde(default)]
    pub description: Option<String>,
    pub runbook_url: String,
}

#[derive(Debug, Error)]
pub enum RuleError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("validation: {0}")]
    Validation(String),
}

/// Parse a YAML rule file from a string.
pub fn parse_rule_file(input: &str) -> Result<RuleFile, RuleError> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(input)?;
    // Map top-level "crate" → "crate_" (Rust reserved keyword)
    if let Some(map) = value.as_mapping_mut() {
        if let Some(v) = map.remove(&serde_yaml::Value::String("crate".into())) {
            map.insert(serde_yaml::Value::String("crate_".into()), v);
        }
    }
    let file: RuleFile = serde_yaml::from_value(value)?;
    validate(&file)?;
    Ok(file)
}

/// Load a rule file from disk.
pub fn load_rule_file(path: impl AsRef<Path>) -> Result<RuleFile, RuleError> {
    let s = std::fs::read_to_string(path)?;
    parse_rule_file(&s)
}

/// Load every YAML file in a directory.
pub fn load_directory(dir: impl AsRef<Path>) -> Result<Vec<RuleFile>, RuleError> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            out.push(load_rule_file(&path)?);
        }
    }
    Ok(out)
}

fn validate(file: &RuleFile) -> Result<(), RuleError> {
    if file.alerts.is_empty() {
        return Err(RuleError::Validation(format!("group '{}' has no alerts", file.group)));
    }
    for spec in &file.alerts {
        if spec.alert.is_empty() {
            return Err(RuleError::Validation("alert name empty".into()));
        }
        if spec.expr.trim().is_empty() {
            return Err(RuleError::Validation(format!("alert '{}' has empty expr", spec.alert)));
        }
        if spec.annotations.summary.trim().is_empty() {
            return Err(RuleError::Validation(format!(
                "alert '{}' missing summary",
                spec.alert
            )));
        }
        if !spec.annotations.runbook_url.starts_with("https://") {
            return Err(RuleError::Validation(format!(
                "alert '{}' runbook_url must be https:",
                spec.alert
            )));
        }
        if let Some(s) = &spec.r#for {
            if crate::models::parse_duration(s).is_err() {
                return Err(RuleError::Validation(format!(
                    "alert '{}' has invalid for: '{}'",
                    spec.alert, s
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules_dir() -> std::path::PathBuf {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        std::path::PathBuf::from(manifest_dir).join("rules")
    }

    #[test]
    fn test_parse_valid_minimal_rule() {
        let yaml = r#"
crate: cave-x
group: cave-x-slo
alerts:
  - alert: XAlert
    expr: 'up == 0'
    for: 1m
    severity: critical
    annotations:
      summary: "X is down"
      runbook_url: "https://docs.cave.dev/runbooks/cave-x/down.md"
"#;
        let file = parse_rule_file(yaml).unwrap();
        assert_eq!(file.alerts.len(), 1);
        assert_eq!(file.alerts[0].severity, Severity::Critical);
    }

    #[test]
    fn test_validation_rejects_empty_alerts() {
        let yaml = "crate: x\ngroup: g\nalerts: []\n";
        assert!(parse_rule_file(yaml).is_err());
    }

    #[test]
    fn test_validation_rejects_missing_summary() {
        let yaml = r#"
group: g
alerts:
  - alert: A
    expr: 'up == 0'
    severity: critical
    annotations:
      summary: "  "
      runbook_url: "https://x"
"#;
        assert!(parse_rule_file(yaml).is_err());
    }

    #[test]
    fn test_validation_rejects_non_https_runbook() {
        let yaml = r#"
group: g
alerts:
  - alert: A
    expr: 'up == 0'
    severity: critical
    annotations:
      summary: "X"
      runbook_url: "http://example.com"
"#;
        assert!(parse_rule_file(yaml).is_err());
    }

    #[test]
    fn test_validation_rejects_bad_for() {
        let yaml = r#"
group: g
alerts:
  - alert: A
    expr: 'up == 0'
    for: "junk"
    severity: warning
    annotations:
      summary: "x"
      runbook_url: "https://example.com"
"#;
        assert!(parse_rule_file(yaml).is_err());
    }

    #[test]
    fn test_each_catalog_file_has_eight_alerts() {
        let files = load_directory(rules_dir()).unwrap();
        assert!(!files.is_empty(), "rules dir is empty");
        for f in &files {
            assert_eq!(
                f.alerts.len(),
                8,
                "group {} has {} alerts (expected 8)",
                f.group,
                f.alerts.len()
            );
        }
    }

    #[test]
    fn test_catalog_files_cover_eight_crates() {
        let files = load_directory(rules_dir()).unwrap();
        let crates: std::collections::HashSet<_> = files
            .iter()
            .filter_map(|f| f.crate_.clone())
            .collect();
        assert_eq!(crates.len(), 8, "expected 8 crates, got {}: {:?}", crates.len(), crates);
        for expected in [
            "cave-apiserver",
            "cave-cri",
            "cave-kubelet",
            "cave-scheduler",
            "cave-etcd",
            "cave-net",
            "cave-streams",
            "cave-pg",
        ] {
            assert!(crates.contains(expected), "missing crate {}", expected);
        }
    }

    #[test]
    fn test_catalog_alert_count_total_64() {
        let files = load_directory(rules_dir()).unwrap();
        let total: usize = files.iter().map(|f| f.alerts.len()).sum();
        assert_eq!(total, 64);
    }

    #[test]
    fn test_each_alert_has_runbook_url() {
        let files = load_directory(rules_dir()).unwrap();
        for f in &files {
            for a in &f.alerts {
                assert!(a.annotations.runbook_url.starts_with("https://docs.cave.dev/runbooks/"));
            }
        }
    }

    #[test]
    fn test_each_group_includes_burn_rate_fast_slow_and_health() {
        let files = load_directory(rules_dir()).unwrap();
        for f in &files {
            let names: Vec<_> = f.alerts.iter().map(|a| a.alert.as_str()).collect();
            assert!(
                names.iter().any(|n| n.contains("BurnRateFast")),
                "{} missing fast burn",
                f.group
            );
            assert!(
                names.iter().any(|n| n.contains("BurnRateSlow")),
                "{} missing slow burn",
                f.group
            );
            assert!(
                names.iter().any(|n| n.contains("HealthProbeFailing")),
                "{} missing health probe",
                f.group
            );
        }
    }

    #[test]
    fn test_severity_enum_serde() {
        let s: Severity = serde_yaml::from_str("critical").unwrap();
        assert_eq!(s, Severity::Critical);
    }
}
