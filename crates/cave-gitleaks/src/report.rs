// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Report writers — JSON (upstream-parity field names) and SARIF 2.1.0
//! (in-scope subset).
//!
//! Mirrors `report/json.go` + `report/sarif.go` upstream (`v8.29.1`).
//! Out-of-scope: CSV, JUnit, Go-template render. `findings.go` redact
//! flag is honoured at the [`crate::detect`] layer, so writers never see
//! raw secrets.

use std::io::Write;

use serde::Serialize;

use crate::finding::Finding;

/// Write findings as a JSON array. Upstream parity: `report/json.go`
/// encodes findings as a plain array (no envelope) with the upstream
/// `Finding` field names preserved.
pub fn write_json<W: Write>(w: &mut W, findings: &[Finding]) -> std::io::Result<()> {
    serde_json::to_writer_pretty(w, findings).map_err(std::io::Error::other)
}

/// Write findings as CSV with an upstream-compatible header row.
/// Mirrors `report/csv.go` upstream (`v8.29.1`). Quotes are doubled
/// inside fields per RFC 4180; fields with commas/newlines/quotes are
/// quoted.
pub fn write_csv<W: Write>(w: &mut W, findings: &[Finding]) -> std::io::Result<()> {
    let header = [
        "RuleID",
        "Commit",
        "File",
        "StartLine",
        "EndLine",
        "StartColumn",
        "EndColumn",
        "Match",
        "Secret",
        "Description",
        "Author",
        "Email",
        "Date",
        "Tags",
        "Fingerprint",
    ];
    writeln!(w, "{}", header.join(","))?;
    for f in findings {
        let row = [
            csv_field(&f.rule_id),
            csv_field(&f.commit),
            csv_field(&f.file),
            f.start_line.to_string(),
            f.end_line.to_string(),
            f.start_column.to_string(),
            f.end_column.to_string(),
            csv_field(&f.match_text),
            csv_field(&f.secret),
            csv_field(&f.description),
            csv_field(&f.author),
            csv_field(&f.email),
            csv_field(&f.date),
            csv_field(&f.tags.join("|")),
            csv_field(&f.fingerprint),
        ];
        writeln!(w, "{}", row.join(","))?;
    }
    Ok(())
}

/// Write findings as JUnit XML. Each finding becomes one `<testcase>`
/// containing a `<failure>` element. Mirrors `report/junit.go`
/// (`v8.29.1`) — minimal envelope so CI runners can ingest gitleaks
/// findings via their JUnit support.
pub fn write_junit<W: Write>(w: &mut W, findings: &[Finding]) -> std::io::Result<()> {
    writeln!(w, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
    writeln!(
        w,
        "<testsuite name=\"cave-gitleaks\" tests=\"{}\" failures=\"{}\">",
        findings.len(),
        findings.len()
    )?;
    for f in findings {
        writeln!(
            w,
            "  <testcase classname=\"{}\" name=\"{}:{}:{}\">",
            xml_escape(&f.rule_id),
            xml_escape(&f.file),
            f.start_line,
            f.start_column,
        )?;
        writeln!(
            w,
            "    <failure type=\"{}\" message=\"{}\">{}:{}</failure>",
            xml_escape(&f.rule_id),
            xml_escape(&f.description),
            xml_escape(&f.file),
            f.start_line,
        )?;
        writeln!(w, "  </testcase>")?;
    }
    writeln!(w, "</testsuite>")?;
    Ok(())
}

/// CSV field quoting per RFC 4180.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Minimal XML character escape for attributes + text bodies.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Write findings as SARIF 2.1.0. In-scope subset:
/// - `runs[0].tool.driver.name = "cave-gitleaks"`
/// - `runs[0].tool.driver.semanticVersion`
/// - `runs[0].tool.driver.rules` from the unique `rule_id`s found
/// - `runs[0].results[]` with `ruleId`, `message.text`, `locations[]`
///
/// Out-of-scope: invocations, conversion, fingerprint dedup, deprecatedIds.
pub fn write_sarif<W: Write>(w: &mut W, findings: &[Finding]) -> std::io::Result<()> {
    let report = build_sarif(findings);
    serde_json::to_writer_pretty(w, &report).map_err(std::io::Error::other)
}

fn build_sarif(findings: &[Finding]) -> SarifLog {
    // Deduplicate rule descriptors.
    let mut rule_ids: Vec<&str> = findings.iter().map(|f| f.rule_id.as_str()).collect();
    rule_ids.sort_unstable();
    rule_ids.dedup();
    let rules: Vec<SarifRule> = rule_ids
        .iter()
        .map(|id| {
            // Pull the first finding's description for this rule (stable
            // because findings are produced in encounter order).
            let desc = findings
                .iter()
                .find(|f| f.rule_id == *id)
                .map(|f| f.description.clone())
                .unwrap_or_default();
            SarifRule {
                id: id.to_string(),
                name: id.to_string(),
                short_description: SarifMessage { text: desc.clone() },
                full_description: SarifMessage { text: desc },
            }
        })
        .collect();
    let results: Vec<SarifResult> = findings
        .iter()
        .map(|f| SarifResult {
            rule_id: f.rule_id.clone(),
            message: SarifMessage {
                text: f.description.clone(),
            },
            locations: vec![SarifLocation {
                physical_location: SarifPhysicalLocation {
                    artifact_location: SarifArtifactLocation {
                        uri: f.file.clone(),
                    },
                    region: SarifRegion {
                        start_line: f.start_line,
                        end_line: f.end_line,
                        start_column: f.start_column,
                        end_column: f.end_column,
                    },
                },
            }],
            partial_fingerprints: SarifFingerprints {
                commit_sha_line: f.fingerprint.clone(),
            },
        })
        .collect();
    SarifLog {
        schema: "https://json.schemastore.org/sarif-2.1.0.json".to_string(),
        version: "2.1.0".to_string(),
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "cave-gitleaks".to_string(),
                    semantic_version: env!("CARGO_PKG_VERSION").to_string(),
                    information_uri:
                        "https://github.com/cave-runtime/cave-runtime"
                            .to_string(),
                    rules,
                },
            },
            results,
        }],
    }
}

// ── SARIF 2.1.0 minimal schema ──────────────────────────────────────────────
// Field names follow the OASIS spec verbatim (camelCase via serde rename).

#[derive(Debug, Serialize)]
struct SarifLog {
    #[serde(rename = "$schema")]
    schema: String,
    version: String,
    runs: Vec<SarifRun>,
}

#[derive(Debug, Serialize)]
struct SarifRun {
    tool: SarifTool,
    results: Vec<SarifResult>,
}

#[derive(Debug, Serialize)]
struct SarifTool {
    driver: SarifDriver,
}

#[derive(Debug, Serialize)]
struct SarifDriver {
    name: String,
    #[serde(rename = "semanticVersion")]
    semantic_version: String,
    #[serde(rename = "informationUri")]
    information_uri: String,
    rules: Vec<SarifRule>,
}

#[derive(Debug, Serialize)]
struct SarifRule {
    id: String,
    name: String,
    #[serde(rename = "shortDescription")]
    short_description: SarifMessage,
    #[serde(rename = "fullDescription")]
    full_description: SarifMessage,
}

#[derive(Debug, Serialize)]
struct SarifMessage {
    text: String,
}

#[derive(Debug, Serialize)]
struct SarifResult {
    #[serde(rename = "ruleId")]
    rule_id: String,
    message: SarifMessage,
    locations: Vec<SarifLocation>,
    #[serde(rename = "partialFingerprints")]
    partial_fingerprints: SarifFingerprints,
}

#[derive(Debug, Serialize)]
struct SarifLocation {
    #[serde(rename = "physicalLocation")]
    physical_location: SarifPhysicalLocation,
}

#[derive(Debug, Serialize)]
struct SarifPhysicalLocation {
    #[serde(rename = "artifactLocation")]
    artifact_location: SarifArtifactLocation,
    region: SarifRegion,
}

#[derive(Debug, Serialize)]
struct SarifArtifactLocation {
    uri: String,
}

#[derive(Debug, Serialize)]
struct SarifRegion {
    #[serde(rename = "startLine")]
    start_line: usize,
    #[serde(rename = "endLine")]
    end_line: usize,
    #[serde(rename = "startColumn")]
    start_column: usize,
    #[serde(rename = "endColumn")]
    end_column: usize,
}

#[derive(Debug, Serialize)]
struct SarifFingerprints {
    #[serde(rename = "commitShaLine/v1")]
    commit_sha_line: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_finding() -> Finding {
        let mut f = Finding {
            description: "AWS Access Token".into(),
            start_line: 12,
            end_line: 12,
            start_column: 5,
            end_column: 25,
            match_text: "AKIA********".into(),
            secret: "AKIA********".into(),
            file: "src/main.rs".into(),
            symlink_file: String::new(),
            commit: "abc123".into(),
            entropy: 4.0,
            author: String::new(),
            email: String::new(),
            date: String::new(),
            message: String::new(),
            tags: vec![],
            rule_id: "aws-access-token".into(),
            fingerprint: String::new(),
        };
        f.fingerprint = f.compute_fingerprint();
        f
    }

    #[test]
    fn write_json_produces_array_with_upstream_field_names() {
        let mut buf = Vec::new();
        write_json(&mut buf, &[one_finding()]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with('['));
        assert!(s.contains("\"RuleID\""));
        assert!(s.contains("\"Commit\""));
        assert!(s.contains("\"StartLine\""));
        // Never persist raw secret.
        assert!(!s.contains("AKIAIOSFODNN7"));
    }

    #[test]
    fn write_json_empty_findings_emits_empty_array() {
        let mut buf = Vec::new();
        write_json(&mut buf, &[]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.trim(), "[]");
    }

    #[test]
    fn write_sarif_has_required_envelope_fields() {
        let mut buf = Vec::new();
        write_sarif(&mut buf, &[one_finding()]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("\"version\""));
        assert!(s.contains("\"2.1.0\""));
        assert!(s.contains("\"$schema\""));
        assert!(s.contains("cave-gitleaks"));
        assert!(s.contains("aws-access-token"));
    }

    #[test]
    fn sarif_dedups_rule_descriptors() {
        let f = one_finding();
        let v: serde_json::Value = serde_json::from_str(&{
            let mut buf = Vec::new();
            write_sarif(&mut buf, &[f.clone(), f.clone(), f]).unwrap();
            String::from_utf8(buf).unwrap()
        })
        .unwrap();
        let rules = v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1, "rule descriptors must dedup by id");
        let results = v["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 3, "results never dedup, every match emits");
    }

    #[test]
    fn sarif_result_carries_region_and_uri() {
        let f = one_finding();
        let mut buf = Vec::new();
        write_sarif(&mut buf, &[f]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        let region = &v["runs"][0]["results"][0]["locations"][0]
            ["physicalLocation"]["region"];
        assert_eq!(region["startLine"], 12);
        assert_eq!(region["endColumn"], 25);
        let uri = &v["runs"][0]["results"][0]["locations"][0]
            ["physicalLocation"]["artifactLocation"]["uri"];
        assert_eq!(uri, "src/main.rs");
    }
}
