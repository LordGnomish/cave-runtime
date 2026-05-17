// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts integrations::trivy (cave-scan adapter + Trivy/Grype JSON mapper)
//! Trivy / Grype / cave-scan adapter — push artifact, run scan, persist
//! findings as [`crate::core::Vulnerability`].
//!
//! Two backends are wired through one [`Scanner`] trait:
//!
//! 1. [`CaveScanAdapter`] — bridges into the local `cave-scan` engine. cave-
//!    scan today emits SAST [`cave_scan::models::Finding`] rows; this
//!    adapter normalises them into our cross-side [`Vulnerability`] shape
//!    so the Harbor scan pipeline + Pulp container plugin can talk to a
//!    single sink.
//!
//! 2. [`TrivyJsonScanner`] — parses a Trivy `--format=json` report (the
//!    same shape `trivy image -f json` produces) into [`Vulnerability`]
//!    rows. Used when the operator runs Trivy out-of-process and POSTs
//!    the JSON report into `/api/artifacts/harbor/scan/import`.
//!
//! The cross-crate dep on `cave-scan` was already declared (cave-scan's
//! engine is pure-Rust and dep-free), so wiring is real, not mocked.

use crate::core::{AffectedComponent, Severity, Vulnerability, VulnerabilitySource};
use cave_scan::engine::{build_result, scan_content};
use cave_scan::models::{Finding, FindingSeverity, ScanResult as CaveScanResult, ScanRule};
use serde::{Deserialize, Serialize};

/// Cross-backend scanner trait. Every adapter (`CaveScanAdapter`,
/// `TrivyJsonScanner`, future Grype/Clair) implements this.
pub trait Scanner: Send + Sync {
    /// Wire identifier — used by the dashboard `scanner` label.
    fn name(&self) -> &'static str;
    /// Run the scan against `target` (digest, path, or URL — adapter-defined)
    /// and return one [`Vulnerability`] per finding.
    fn scan(&self, target: &str, payload: &[u8]) -> Result<Vec<Vulnerability>, ScannerError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ScannerError {
    #[error("scanner I/O: {0}")]
    Io(String),
    #[error("scanner parse: {0}")]
    Parse(String),
}

// ── cave-scan adapter ────────────────────────────────────────────────────────

/// Adapter that delegates to the in-process `cave-scan` engine. Each call
/// runs a synchronous regex/keyword pass against the provided payload and
/// converts the resulting [`cave_scan::models::Finding`]s to our
/// [`Vulnerability`] shape.
///
/// Pulp container plugin + Harbor scan pipeline both wire this in when the
/// operator has not configured an external scanner.
pub struct CaveScanAdapter {
    rules: Vec<ScanRule>,
}

impl CaveScanAdapter {
    pub fn new(rules: Vec<ScanRule>) -> Self {
        Self { rules }
    }

    /// Convert a single cave-scan finding into a vulnerability row.
    pub fn finding_to_vulnerability(f: &Finding) -> Vulnerability {
        let sev = match f.severity {
            FindingSeverity::Critical => Severity::Critical,
            FindingSeverity::Major => Severity::High,
            FindingSeverity::Minor => Severity::Medium,
            FindingSeverity::Info => Severity::Low,
        };
        let mut v = Vulnerability::new(f.rule_name.clone(), sev, VulnerabilitySource::Native);
        v.cve = None; // cave-scan does not emit CVE rows today.
        v.affected_components.push(AffectedComponent {
            package: f.file_path.clone(),
            version: format!("line:{}", f.line_number),
        });
        v
    }

    /// Run all configured rules against payload and return mapped findings.
    pub fn scan_payload(&self, target: &str, payload: &[u8]) -> CaveScanResult {
        let content = String::from_utf8_lossy(payload);
        let findings = scan_content(&self.rules, &content, target);
        build_result(target, findings, self.rules.len(), 1)
    }
}

impl Scanner for CaveScanAdapter {
    fn name(&self) -> &'static str {
        "cave-scan"
    }
    fn scan(&self, target: &str, payload: &[u8]) -> Result<Vec<Vulnerability>, ScannerError> {
        let result = self.scan_payload(target, payload);
        Ok(result.findings.iter().map(Self::finding_to_vulnerability).collect())
    }
}

// ── Trivy JSON report parser ─────────────────────────────────────────────────

/// Minimal subset of the Trivy `--format=json` schema we need to round-trip
/// into [`Vulnerability`]. Source: aquasecurity/trivy pkg/report/types.go.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TrivyReport {
    pub artifact_name: String,
    #[serde(default)]
    pub results: Vec<TrivyResultBlock>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TrivyResultBlock {
    pub target: String,
    #[serde(default)]
    pub vulnerabilities: Vec<TrivyVulnerability>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TrivyVulnerability {
    #[serde(rename = "VulnerabilityID")]
    pub vulnerability_id: String,
    pub pkg_name: String,
    pub installed_version: String,
    #[serde(default)]
    pub fixed_version: Option<String>,
    pub severity: String,
    #[serde(default)]
    pub cvss: Option<TrivyCvss>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TrivyCvss {
    #[serde(rename = "nvd", default)]
    pub nvd: Option<TrivyCvssMetrics>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TrivyCvssMetrics {
    #[serde(rename = "V3Score", default)]
    pub v3_score: Option<f32>,
}

/// Trivy JSON-report parser. Operator runs `trivy image -f json` externally
/// and POSTs the report; this struct parses and maps it.
pub struct TrivyJsonScanner;

impl TrivyJsonScanner {
    pub fn map_report(report: &TrivyReport) -> Vec<Vulnerability> {
        let mut out = Vec::new();
        for block in &report.results {
            for vuln in &block.vulnerabilities {
                let severity = Severity::from_scanner_wire(&vuln.severity);
                let cvss_v3 = vuln.cvss.as_ref().and_then(|c| c.nvd.as_ref()).and_then(|m| m.v3_score);
                let cve = if vuln.vulnerability_id.starts_with("CVE-") {
                    Some(vuln.vulnerability_id.clone())
                } else {
                    None
                };
                let mut v = Vulnerability::new(vuln.vulnerability_id.clone(), severity, VulnerabilitySource::Trivy);
                v.cve = cve;
                v.cvss_v3 = cvss_v3;
                v.fixed_in = vuln.fixed_version.clone();
                v.affected_components.push(AffectedComponent {
                    package: vuln.pkg_name.clone(),
                    version: vuln.installed_version.clone(),
                });
                out.push(v);
            }
        }
        out
    }
}

impl Scanner for TrivyJsonScanner {
    fn name(&self) -> &'static str {
        "trivy"
    }
    fn scan(&self, _target: &str, payload: &[u8]) -> Result<Vec<Vulnerability>, ScannerError> {
        let report: TrivyReport = serde_json::from_slice(payload)
            .map_err(|e| ScannerError::Parse(format!("trivy json: {e}")))?;
        Ok(Self::map_report(&report))
    }
}

// ── Cross-side scan sink ─────────────────────────────────────────────────────

/// Shared sink that both Pulp container plugin and Harbor scan pipeline
/// push their findings into. Lets the dashboard count once across both.
#[derive(Default)]
pub struct ScanSink {
    inner: std::sync::Mutex<Vec<(String, Vulnerability)>>,
}

impl ScanSink {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn record(&self, target_digest: &str, vulns: Vec<Vulnerability>) {
        let mut g = self.inner.lock().unwrap();
        for v in vulns {
            g.push((target_digest.to_string(), v));
        }
    }
    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
    pub fn count_blocking(&self) -> usize {
        self.inner.lock().unwrap().iter().filter(|(_, v)| v.is_blocking()).count()
    }
    pub fn findings_for(&self, target_digest: &str) -> Vec<Vulnerability> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(d, v)| if d == target_digest { Some(v.clone()) } else { None })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_scan::models::{FindingSeverity, RuleType, ScanRule};
    use uuid::Uuid;

    fn keyword_rule(pattern: &str, severity: FindingSeverity) -> ScanRule {
        ScanRule {
            id: Uuid::new_v4(),
            name: format!("rule-{pattern}"),
            description: "test".into(),
            pattern: pattern.into(),
            rule_type: RuleType::Keyword,
            severity,
            enabled: true,
        }
    }

    #[test]
    fn cave_scan_adapter_finds_keyword_match_and_maps_severity() {
        let rules = vec![
            keyword_rule("password", FindingSeverity::Critical),
            keyword_rule("DEBUG", FindingSeverity::Minor),
        ];
        let adapter = CaveScanAdapter::new(rules);
        let payload = b"line1\nlet password = \"x\"\nDEBUG=true\nfine\n";
        let vulns = adapter.scan("sha256:cafe", payload).unwrap();
        assert!(!vulns.is_empty(), "scanner returned no findings");
        assert!(vulns.iter().any(|v| v.severity == Severity::Critical));
        assert!(vulns.iter().any(|v| v.severity == Severity::Medium));
        for v in &vulns {
            assert_eq!(v.source, VulnerabilitySource::Native);
        }
    }

    #[test]
    fn cave_scan_finding_severity_maps_correctly() {
        for (cs, expected) in [
            (FindingSeverity::Critical, Severity::Critical),
            (FindingSeverity::Major, Severity::High),
            (FindingSeverity::Minor, Severity::Medium),
            (FindingSeverity::Info, Severity::Low),
        ] {
            let f = Finding {
                id: Uuid::new_v4(),
                rule_id: Uuid::new_v4(),
                rule_name: "x".into(),
                file_path: "src/main.rs".into(),
                line_number: 1,
                matched_text: "x".into(),
                severity: cs,
                message: "x".into(),
            };
            let v = CaveScanAdapter::finding_to_vulnerability(&f);
            assert_eq!(v.severity, expected);
        }
    }

    #[test]
    fn trivy_scanner_parses_real_shape_and_extracts_cve_cvss_fix() {
        let payload = br#"{
            "ArtifactName": "alpine:3.14",
            "Results": [{
                "Target": "alpine:3.14 (alpine 3.14.0)",
                "Vulnerabilities": [{
                    "VulnerabilityID": "CVE-2024-12345",
                    "PkgName": "openssl",
                    "InstalledVersion": "1.1.1k",
                    "FixedVersion": "1.1.1w",
                    "Severity": "CRITICAL",
                    "Cvss": { "nvd": { "V3Score": 9.8 } }
                }, {
                    "VulnerabilityID": "GHSA-abcd-1234",
                    "PkgName": "musl",
                    "InstalledVersion": "1.2.2",
                    "Severity": "MEDIUM"
                }]
            }]
        }"#;
        let scanner = TrivyJsonScanner;
        let vulns = scanner.scan("sha256:alpine", payload).unwrap();
        assert_eq!(vulns.len(), 2);
        let cve = vulns.iter().find(|v| v.id == "CVE-2024-12345").unwrap();
        assert_eq!(cve.severity, Severity::Critical);
        assert_eq!(cve.cve, Some("CVE-2024-12345".into()));
        assert_eq!(cve.cvss_v3, Some(9.8));
        assert_eq!(cve.fixed_in, Some("1.1.1w".into()));
        assert_eq!(cve.source, VulnerabilitySource::Trivy);
        assert_eq!(cve.affected_components[0].package, "openssl");
        let ghsa = vulns.iter().find(|v| v.id == "GHSA-abcd-1234").unwrap();
        assert!(ghsa.cve.is_none(), "non-CVE id should not populate cve field");
        assert_eq!(ghsa.severity, Severity::Medium);
    }

    #[test]
    fn trivy_scanner_rejects_bad_json() {
        let scanner = TrivyJsonScanner;
        let err = scanner.scan("x", b"not json").unwrap_err();
        assert!(matches!(err, ScannerError::Parse(_)));
    }

    #[test]
    fn scan_sink_aggregates_across_targets_and_counts_blocking() {
        let sink = ScanSink::new();
        sink.record(
            "sha256:a",
            vec![Vulnerability::new("CVE-1", Severity::Critical, VulnerabilitySource::Trivy)],
        );
        sink.record(
            "sha256:b",
            vec![
                Vulnerability::new("CVE-2", Severity::Low, VulnerabilitySource::Native),
                Vulnerability::new("CVE-3", Severity::High, VulnerabilitySource::Trivy),
            ],
        );
        assert_eq!(sink.count(), 3);
        assert_eq!(sink.count_blocking(), 2); // critical + high
        assert_eq!(sink.findings_for("sha256:a").len(), 1);
        assert_eq!(sink.findings_for("sha256:b").len(), 2);
        assert_eq!(sink.findings_for("sha256:nonexistent").len(), 0);
    }

    #[test]
    fn scanner_names_are_stable_for_dashboard_label() {
        let trivy = TrivyJsonScanner;
        let cave = CaveScanAdapter::new(vec![]);
        assert_eq!(trivy.name(), "trivy");
        assert_eq!(cave.name(), "cave-scan");
    }
}
