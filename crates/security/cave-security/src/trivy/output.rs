// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Trivy-compatible output formats — JSON, table (text), SARIF.

use crate::trivy::scanner::{ScanResult, VulnFinding};
use crate::trivy::vuln_db::Severity;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Format enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Json,
    Table,
    Sarif,
}

// ---------------------------------------------------------------------------
// JSON output (Trivy-compatible schema)
// ---------------------------------------------------------------------------

pub fn render_json(result: &ScanResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

// ---------------------------------------------------------------------------
// Table output (ASCII, similar to trivy's text output)
// ---------------------------------------------------------------------------

pub fn render_table(result: &ScanResult) -> String {
    let mut out = String::new();

    out.push_str(&format!("\n{} ({})\n", result.target, result.scan_type));
    out.push_str(&"=".repeat(60));
    out.push('\n');

    if result.vulnerabilities.is_empty() {
        out.push_str("No vulnerabilities found.\n");
    } else {
        out.push_str(&format!(
            "\nTotal: {} (CRITICAL: {}, HIGH: {}, MEDIUM: {}, LOW: {}, UNKNOWN: {})\n\n",
            result.vulnerabilities.len(),
            count_by_severity(&result.vulnerabilities, Severity::Critical),
            count_by_severity(&result.vulnerabilities, Severity::High),
            count_by_severity(&result.vulnerabilities, Severity::Medium),
            count_by_severity(&result.vulnerabilities, Severity::Low),
            count_by_severity(&result.vulnerabilities, Severity::Unknown),
        ));

        // Table header
        out.push_str(&format!(
            "{:<20} {:<18} {:<18} {:<10} {:<12}\n",
            "Library", "Vulnerability", "Installed", "Fixed", "Severity"
        ));
        out.push_str(&"-".repeat(80));
        out.push('\n');

        for v in &result.vulnerabilities {
            out.push_str(&format!(
                "{:<20} {:<18} {:<18} {:<10} {:<12}\n",
                truncate(&v.package_name, 19),
                v.cve_id,
                truncate(&v.installed_version, 17),
                v.fixed_version.as_deref().unwrap_or("N/A"),
                v.severity.to_string(),
            ));
        }
    }

    if !result.secrets.is_empty() {
        out.push_str(&format!("\nSecrets ({}):\n", result.secrets.len()));
        out.push_str(&"-".repeat(40));
        out.push('\n');
        for s in &result.secrets {
            out.push_str(&format!(
                "  {} [{}] {} line {}\n",
                s.severity_str(),
                s.rule_id,
                s.file_path,
                s.line_number
            ));
        }
    }

    if !result.misconfigs.is_empty() {
        out.push_str(&format!(
            "\nMisconfigurations ({}):\n",
            result.misconfigs.len()
        ));
        out.push_str(&"-".repeat(40));
        out.push('\n');
        for m in &result.misconfigs {
            out.push_str(&format!(
                "  {} [{:?}] {}\n",
                m.check_id, m.severity, m.title
            ));
        }
    }

    out
}

fn count_by_severity(vulns: &[VulnFinding], sev: Severity) -> usize {
    vulns.iter().filter(|v| v.severity == sev).count()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

// ---------------------------------------------------------------------------
// SARIF 2.1.0
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifReport {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub version: String,
    pub runs: Vec<SarifRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifRun {
    pub tool: SarifTool,
    pub results: Vec<SarifResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifTool {
    pub driver: SarifDriver,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifDriver {
    pub name: String,
    pub version: String,
    pub rules: Vec<SarifRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifRule {
    pub id: String,
    pub name: String,
    #[serde(rename = "shortDescription")]
    pub short_description: SarifMessage,
    #[serde(rename = "defaultConfiguration")]
    pub default_configuration: SarifConfiguration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifMessage {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifConfiguration {
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifResult {
    #[serde(rename = "ruleId")]
    pub rule_id: String,
    pub level: String,
    pub message: SarifMessage,
    pub locations: Vec<SarifLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifLocation {
    #[serde(rename = "physicalLocation")]
    pub physical_location: SarifPhysicalLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifPhysicalLocation {
    #[serde(rename = "artifactLocation")]
    pub artifact_location: SarifArtifactLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<SarifRegion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifArtifactLocation {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SarifRegion {
    #[serde(rename = "startLine")]
    pub start_line: usize,
}

fn severity_to_sarif_level(sev: &Severity) -> &'static str {
    match sev {
        Severity::Critical | Severity::High => "error",
        Severity::Medium => "warning",
        Severity::Low => "note",
        Severity::Unknown => "none",
    }
}

pub fn render_sarif(result: &ScanResult) -> String {
    let mut rules = Vec::new();
    let mut results = Vec::new();

    for v in &result.vulnerabilities {
        // Dedup rules
        if !rules.iter().any(|r: &SarifRule| r.id == v.cve_id) {
            rules.push(SarifRule {
                id: v.cve_id.clone(),
                name: v.cve_id.clone(),
                short_description: SarifMessage {
                    text: v.title.clone().unwrap_or_else(|| v.cve_id.clone()),
                },
                default_configuration: SarifConfiguration {
                    level: severity_to_sarif_level(&v.severity).to_string(),
                },
            });
        }
        results.push(SarifResult {
            rule_id: v.cve_id.clone(),
            level: severity_to_sarif_level(&v.severity).to_string(),
            message: SarifMessage {
                text: format!(
                    "Package: {} {} — {}",
                    v.package_name,
                    v.installed_version,
                    v.fixed_version
                        .as_deref()
                        .map(|f| format!("fixed in {f}"))
                        .unwrap_or_else(|| "no fix available".into())
                ),
            },
            locations: vec![SarifLocation {
                physical_location: SarifPhysicalLocation {
                    artifact_location: SarifArtifactLocation {
                        uri: result.target.clone(),
                    },
                    region: None,
                },
            }],
        });
    }

    for s in &result.secrets {
        let rule_id = s.rule_id.clone();
        if !rules.iter().any(|r: &SarifRule| r.id == rule_id) {
            rules.push(SarifRule {
                id: rule_id.clone(),
                name: s.title.clone(),
                short_description: SarifMessage {
                    text: s.title.clone(),
                },
                default_configuration: SarifConfiguration {
                    level: "error".to_string(),
                },
            });
        }
        results.push(SarifResult {
            rule_id,
            level: "error".to_string(),
            message: SarifMessage {
                text: format!("{}: {}", s.title, s.match_preview),
            },
            locations: vec![SarifLocation {
                physical_location: SarifPhysicalLocation {
                    artifact_location: SarifArtifactLocation {
                        uri: s.file_path.clone(),
                    },
                    region: Some(SarifRegion {
                        start_line: s.line_number,
                    }),
                },
            }],
        });
    }

    let report = SarifReport {
        schema: "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json".into(),
        version: "2.1.0".into(),
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "cave-security".into(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    rules,
                },
            },
            results,
        }],
    };

    serde_json::to_string_pretty(&report).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trivy::scanner::{ScanResult, ScanType};
    use chrono::Utc;

    fn empty_result() -> ScanResult {
        ScanResult {
            target: "test:latest".into(),
            scan_type: ScanType::Image,
            vulnerabilities: vec![],
            secrets: vec![],
            licenses: vec![],
            misconfigs: vec![],
            sbom: None,
            scanned_at: Utc::now(),
        }
    }

    #[test]
    fn render_json_empty() {
        let r = empty_result();
        let j = render_json(&r);
        assert!(j.contains("vulnerabilities"));
    }

    #[test]
    fn render_table_no_vulns() {
        let r = empty_result();
        let t = render_table(&r);
        assert!(t.contains("No vulnerabilities"));
    }

    #[test]
    fn render_sarif_empty() {
        let r = empty_result();
        let s = render_sarif(&r);
        assert!(s.contains("sarif"));
        assert!(s.contains("cave-security"));
    }
}
