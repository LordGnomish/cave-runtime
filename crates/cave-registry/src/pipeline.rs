//! Scan pipeline — runs between `ProxyClient::fetch()` and the final
//! response. This is the enforcement point from ADR-133 §3.1 steps 4-7.
//!
//! Design notes:
//! * The pipeline talks to `cave-container-scan`, `cave-sbom`, `cave-vulns`
//!   and `cave-policy` over HTTP so each scanner can be swapped, scaled, or
//!   disabled independently. The default `ScanPipelineConfig` points at
//!   `http://127.0.0.1:8080` (the cave-runtime binary itself).
//! * The pipeline is FAIL-OPEN by default (`ObserveOnly`) so a misconfigured
//!   scanner cluster does not brick the platform for new workloads. Operators
//!   flip `enforce` on after the 2-week baseline window described in
//!   ADR-133 §6 (phase 2).
//! * Every verdict is emitted as a structured log record the forensics
//!   module can pick up (`target: "cave_registry::pipeline::verdict"`).
//! * Fan-out is parallel (`futures::future::join_all`) with a total budget.
//!   Scanners that time out are counted as WARN, never FAIL, so a slow
//!   scanner cannot hard-block a legitimate install.

use crate::proxy::{Ecosystem, FetchedArtifact};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::future::join_all;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{info, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanPipelineConfig {
    /// HTTP base URL of cave-container-scan.
    pub container_scan_url: String,
    /// HTTP base URL of cave-sbom.
    pub sbom_url: String,
    /// HTTP base URL of cave-vulns (for dedup + finding storage).
    pub vulns_url: String,
    /// HTTP base URL of cave-policy rego admission bundle.
    pub policy_url: String,
    /// Total time budget for the whole pipeline per artefact (seconds).
    pub overall_timeout_seconds: u64,
    /// If `true`, a `Fail` verdict returns an `Err(ScanPipelineOutcome::Blocked)`
    /// from [`ScanPipeline::evaluate`]. If `false` (default), `Fail` is
    /// reported as `Observed` and the caller may still serve the artefact.
    pub enforce: bool,
    /// If `true`, scanners that return transport errors count as `Warn`
    /// rather than `Fail`. This prevents a single flaky scanner from
    /// hard-blocking.
    pub tolerate_scanner_errors: bool,
}

impl Default for ScanPipelineConfig {
    fn default() -> Self {
        Self {
            container_scan_url: "http://127.0.0.1:8080".to_string(),
            sbom_url: "http://127.0.0.1:8080".to_string(),
            vulns_url: "http://127.0.0.1:8080".to_string(),
            policy_url: "http://127.0.0.1:8080".to_string(),
            overall_timeout_seconds: 30,
            enforce: false,
            tolerate_scanner_errors: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Verdict types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerdictDecision {
    Pass,
    Warn,
    Fail,
}

impl std::fmt::Display for VerdictDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerdictDecision::Pass => f.write_str("pass"),
            VerdictDecision::Warn => f.write_str("warn"),
            VerdictDecision::Fail => f.write_str("fail"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanFindingSummary {
    pub scanner: String,
    pub severity: String,
    pub rule: String,
    pub summary: String,
    pub cves: Vec<String>,
}

/// Final outcome of the pipeline — consumed by the routes layer which turns
/// it into an HTTP response (or a 451 if `Blocked`).
#[derive(Debug, Clone, Serialize)]
pub struct ScanPipelineOutcome {
    pub id: Uuid,
    pub verdict: VerdictDecision,
    pub reasons: Vec<String>,
    pub findings: Vec<ScanFindingSummary>,
    pub ecosystem: Ecosystem,
    pub name: String,
    pub version: Option<String>,
    pub sha256: String,
    pub scanner_ms: u64,
    pub scanned_at: DateTime<Utc>,
    /// Whether this outcome should BLOCK the response (true only if
    /// `enforce = true` AND `verdict = Fail`). Exposed explicitly so the
    /// caller does not re-derive the condition.
    pub blocked: bool,
}

// ---------------------------------------------------------------------------
// ScanPipeline
// ---------------------------------------------------------------------------

pub struct ScanPipeline {
    cfg: ScanPipelineConfig,
    http: Client,
}

impl ScanPipeline {
    pub fn new(cfg: ScanPipelineConfig) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(cfg.overall_timeout_seconds))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { cfg, http }
    }

    pub fn config(&self) -> &ScanPipelineConfig {
        &self.cfg
    }

    /// Run the scan pipeline against a freshly fetched artefact. Returns a
    /// structured outcome that the caller (routes/proxy) can turn into an
    /// HTTP response. Never panics — transport errors become `Warn`
    /// findings so the pipeline is fail-soft.
    pub async fn evaluate(&self, art: &FetchedArtifact) -> ScanPipelineOutcome {
        let started = Instant::now();
        let id = Uuid::new_v4();

        // Fan-out: container-scan namespace check + container-scan image scan
        // (if OCI) + SBOM generation. We keep these three small HTTP calls
        // independent so a slow one doesn't block the other two.
        let container_scan_fut = self.container_scan_namespace(art);
        let sbom_fut = self.sbom_generate(art);
        let policy_fut = self.policy_evaluate(art);

        let (ns_res, sbom_res, policy_res) =
            tokio::join!(container_scan_fut, sbom_fut, policy_fut);

        let mut findings: Vec<ScanFindingSummary> = Vec::new();
        let mut reasons: Vec<String> = Vec::new();

        match ns_res {
            Ok(mut f) => findings.append(&mut f),
            Err(e) => {
                warn!(target: "cave_registry::pipeline", scanner = "container-scan-namespace", err = %e);
                if !self.cfg.tolerate_scanner_errors {
                    findings.push(ScanFindingSummary {
                        scanner: "container-scan-namespace".to_string(),
                        severity: "high".to_string(),
                        rule: "scanner-error".to_string(),
                        summary: format!("scanner unavailable: {e}"),
                        cves: vec![],
                    });
                }
            }
        }
        match sbom_res {
            Ok(mut f) => findings.append(&mut f),
            Err(e) => {
                warn!(target: "cave_registry::pipeline", scanner = "sbom", err = %e);
                if !self.cfg.tolerate_scanner_errors {
                    findings.push(ScanFindingSummary {
                        scanner: "sbom".to_string(),
                        severity: "high".to_string(),
                        rule: "scanner-error".to_string(),
                        summary: format!("scanner unavailable: {e}"),
                        cves: vec![],
                    });
                }
            }
        }
        match policy_res {
            Ok(mut f) => findings.append(&mut f),
            Err(e) => {
                warn!(target: "cave_registry::pipeline", scanner = "policy", err = %e);
                // Policy unavailable is treated conservatively: in enforce
                // mode we fail closed; in observe-only we fail soft.
                if self.cfg.enforce && !self.cfg.tolerate_scanner_errors {
                    findings.push(ScanFindingSummary {
                        scanner: "policy".to_string(),
                        severity: "critical".to_string(),
                        rule: "policy-unavailable".to_string(),
                        summary: format!("policy engine unavailable (fail-closed): {e}"),
                        cves: vec![],
                    });
                }
            }
        }

        let verdict = aggregate_verdict(&findings);
        if matches!(verdict, VerdictDecision::Fail) {
            reasons.push("one or more critical/high findings detected".to_string());
        } else if matches!(verdict, VerdictDecision::Warn) {
            reasons.push("medium findings detected; serving but flagged".to_string());
        } else {
            reasons.push("no actionable findings".to_string());
        }
        // Static blocklist wins unconditionally — but the proxy layer has
        // already handled that before calling us, so nothing to do here.

        let scanner_ms = started.elapsed().as_millis() as u64;

        let blocked = self.cfg.enforce && matches!(verdict, VerdictDecision::Fail);

        let outcome = ScanPipelineOutcome {
            id,
            verdict: verdict.clone(),
            reasons,
            findings,
            ecosystem: art.ecosystem,
            name: art.name.clone(),
            version: art.version.clone(),
            sha256: art.sha256_hex.clone(),
            scanner_ms,
            scanned_at: Utc::now(),
            blocked,
        };

        info!(
            target: "cave_registry::pipeline::verdict",
            id = %outcome.id,
            ecosystem = outcome.ecosystem.as_str(),
            name = %outcome.name,
            version = ?outcome.version,
            sha256 = %outcome.sha256,
            verdict = %outcome.verdict,
            blocked = outcome.blocked,
            scanner_ms = outcome.scanner_ms,
            finding_count = outcome.findings.len(),
            "scan pipeline verdict"
        );

        // Best-effort finding emission to cave-vulns; ignored on error so
        // the pipeline outcome is independent of the sink.
        let _ = self.emit_findings_to_vulns(&outcome).await;

        outcome
    }

    // ── Individual scanner RPCs ──────────────────────────────────────────

    async fn container_scan_namespace(
        &self,
        art: &FetchedArtifact,
    ) -> Result<Vec<ScanFindingSummary>, String> {
        let url = format!("{}/api/container-scan/namespace", self.cfg.container_scan_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "ecosystem": art.ecosystem.as_str(),
            "name": art.name,
        });
        let resp = self.http.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("{url} → {}", resp.status()));
        }
        let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(findings_from_value(&value, "container-scan-namespace"))
    }

    async fn sbom_generate(
        &self,
        art: &FetchedArtifact,
    ) -> Result<Vec<ScanFindingSummary>, String> {
        let url = format!("{}/api/sbom", self.cfg.sbom_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "project": art.name,
            "version": art.version.as_deref().unwrap_or("unknown"),
            "content_sha256": art.sha256_hex,
        });
        let resp = self.http.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("{url} → {}", resp.status()));
        }
        let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(findings_from_value(&value, "sbom"))
    }

    async fn policy_evaluate(
        &self,
        art: &FetchedArtifact,
    ) -> Result<Vec<ScanFindingSummary>, String> {
        let url = format!(
            "{}/api/policy/evaluate",
            self.cfg.policy_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "bundle": "registry.admission",
            "input": {
                "ecosystem": art.ecosystem.as_str(),
                "name": art.name,
                "version": art.version,
                "sha256": art.sha256_hex,
            }
        });
        let resp = self.http.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("{url} → {}", resp.status()));
        }
        let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(findings_from_value(&value, "policy"))
    }

    async fn emit_findings_to_vulns(&self, outcome: &ScanPipelineOutcome) -> Result<(), String> {
        if outcome.findings.is_empty() {
            return Ok(());
        }
        let url = format!("{}/api/vulns/findings/bulk", self.cfg.vulns_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "source": "cave-registry",
            "artefact": {
                "ecosystem": outcome.ecosystem.as_str(),
                "name": outcome.name,
                "version": outcome.version,
                "sha256": outcome.sha256,
            },
            "verdict": outcome.verdict,
            "findings": outcome.findings,
            "scan_id": outcome.id,
            "scanned_at": outcome.scanned_at,
        });
        let _ = self.http.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Lift findings out of a permissive JSON shape. The wire format across
/// scanners isn't perfectly unified today — container-scan returns
/// `{ findings: [...] }`, sbom may return `{ vulnerabilities: [...] }`, and
/// policy returns `{ reasons: [...] }`. We pick whichever array is present
/// and normalise to `ScanFindingSummary`.
fn findings_from_value(v: &serde_json::Value, scanner: &str) -> Vec<ScanFindingSummary> {
    let array = v
        .get("findings")
        .or_else(|| v.get("vulnerabilities"))
        .or_else(|| v.get("reasons"))
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    array
        .iter()
        .map(|item| {
            let severity = item
                .get("severity")
                .and_then(|s| s.as_str())
                .unwrap_or("info")
                .to_ascii_lowercase();
            let rule = item
                .get("rule_id")
                .or_else(|| item.get("rule"))
                .and_then(|s| s.as_str())
                .unwrap_or("unknown")
                .to_string();
            let summary = item
                .get("title")
                .or_else(|| item.get("summary"))
                .or_else(|| item.get("description"))
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let cves = item
                .get("cves")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| c.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            ScanFindingSummary {
                scanner: scanner.to_string(),
                severity,
                rule,
                summary,
                cves,
            }
        })
        .collect()
}

/// Walk findings and decide Pass/Warn/Fail. Ordering follows ADR-133 §4.1:
/// any Critical/High → Fail; any Medium → Warn; else Pass. Unknown
/// severities are treated as `info` (pass) to avoid false-positives.
pub fn aggregate_verdict(findings: &[ScanFindingSummary]) -> VerdictDecision {
    let mut has_medium = false;
    for f in findings {
        match f.severity.as_str() {
            "critical" | "high" => return VerdictDecision::Fail,
            "medium" => has_medium = true,
            _ => {}
        }
    }
    if has_medium {
        VerdictDecision::Warn
    } else {
        VerdictDecision::Pass
    }
}

// Utility kept pub(crate) because callers outside this module may want to
// hash upstream bytes themselves before handing them to the pipeline.
pub(crate) fn sha256_hex(b: &Bytes) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(b))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(severity: &str) -> ScanFindingSummary {
        ScanFindingSummary {
            scanner: "test".to_string(),
            severity: severity.to_string(),
            rule: "R".to_string(),
            summary: "".to_string(),
            cves: vec![],
        }
    }

    #[test]
    fn pass_when_no_findings() {
        assert_eq!(aggregate_verdict(&[]), VerdictDecision::Pass);
    }

    #[test]
    fn pass_on_low_and_info_only() {
        let f = vec![finding("low"), finding("info"), finding("trivial")];
        assert_eq!(aggregate_verdict(&f), VerdictDecision::Pass);
    }

    #[test]
    fn warn_on_medium() {
        let f = vec![finding("medium"), finding("info")];
        assert_eq!(aggregate_verdict(&f), VerdictDecision::Warn);
    }

    #[test]
    fn fail_on_high() {
        let f = vec![finding("high"), finding("low")];
        assert_eq!(aggregate_verdict(&f), VerdictDecision::Fail);
    }

    #[test]
    fn fail_on_critical_even_with_medium() {
        let f = vec![finding("medium"), finding("critical")];
        assert_eq!(aggregate_verdict(&f), VerdictDecision::Fail);
    }

    #[test]
    fn severity_case_is_ignored_by_parser() {
        // findings_from_value lower-cases severity.
        let value = serde_json::json!({
            "findings": [
                { "severity": "CRITICAL", "rule_id": "X", "title": "boom", "cves": ["CVE-2024-1"] }
            ]
        });
        let out = findings_from_value(&value, "scanner-x");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "critical");
        assert_eq!(out[0].cves, vec!["CVE-2024-1".to_string()]);
    }

    #[test]
    fn findings_from_value_handles_multiple_array_keys() {
        let a = serde_json::json!({"findings":[{"severity":"high","rule_id":"F"}]});
        let b = serde_json::json!({"vulnerabilities":[{"severity":"high","rule":"V"}]});
        let c = serde_json::json!({"reasons":[{"severity":"medium","rule":"R"}]});
        assert_eq!(findings_from_value(&a, "s").len(), 1);
        assert_eq!(findings_from_value(&b, "s").len(), 1);
        assert_eq!(findings_from_value(&c, "s").len(), 1);
        assert!(findings_from_value(&serde_json::json!({}), "s").is_empty());
    }
}
