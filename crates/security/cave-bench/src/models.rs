// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core data model — Check / Finding / Profile / Target / Scan.
//!
//! Upstream parallels:
//! - kube-bench `check/check.go` `Check`, `State`, `Test`
//! - kubescape `core/cautils/datastructures.go` `OPASessionObj`, `IPolicies`

use serde::{Deserialize, Serialize};

/// Outcome of evaluating a single check against a target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// Compliant — control fully satisfied.
    Pass,
    /// Non-compliant — control violated.
    Fail,
    /// Manual review required (info-only).
    Warn,
    /// Informational, no compliance impact.
    Info,
    /// Not applicable to the target (e.g. control for kubeadm-only clusters).
    NotApplicable,
    /// Check could not run (e.g. file unreadable). Counts as gap, not pass.
    Error,
}

impl Verdict {
    pub fn is_failure(&self) -> bool {
        matches!(self, Verdict::Fail | Verdict::Error)
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Warn => "WARN",
            Verdict::Info => "INFO",
            Verdict::NotApplicable => "NOT_APPLICABLE",
            Verdict::Error => "ERROR",
        }
    }
}

/// Top-level compliance framework a check belongs to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Framework {
    /// CIS Kubernetes Benchmark — kube-bench upstream.
    CisK8s,
    /// NSA / CISA Kubernetes Hardening Guide — kubescape upstream.
    NsaHardening,
    /// MITRE ATT&CK for Kubernetes — kubescape upstream.
    MitreAttack,
    /// kubescape "security" framework — security baseline (37 controls).
    SecurityBaseline,
    /// SOC2 / ISO Common Criteria style controls — derived.
    SocControls,
    /// Operator-authored custom benchmark framework.
    Custom,
}

impl Framework {
    pub fn as_str(&self) -> &'static str {
        match self {
            Framework::CisK8s => "cis-k8s",
            Framework::NsaHardening => "nsa-hardening",
            Framework::MitreAttack => "mitre-attack",
            Framework::SecurityBaseline => "security",
            Framework::SocControls => "soc-controls",
            Framework::Custom => "custom",
        }
    }
}

/// CIS node-type classification — `kube-bench cfg/config.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NodeType {
    Master,
    Node,
    Etcd,
    ControlPlane,
    Policies,
    Managedservices,
}

impl NodeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeType::Master => "master",
            NodeType::Node => "node",
            NodeType::Etcd => "etcd",
            NodeType::ControlPlane => "controlplane",
            NodeType::Policies => "policies",
            NodeType::Managedservices => "managedservices",
        }
    }
}

/// Scoring level: Level 1 (recommended baseline) or Level 2 (defence-in-depth).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CisLevel {
    L1,
    L2,
}

/// Severity classification — kubescape `armoapi-go.armotypes.PostureControl.BaseScore`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
            Severity::Info => "info",
        }
    }
    /// Numeric SARIF-style score (0..=10).
    pub fn score(&self) -> u8 {
        match self {
            Severity::Critical => 9,
            Severity::High => 7,
            Severity::Medium => 5,
            Severity::Low => 3,
            Severity::Info => 1,
        }
    }
}

/// One actionable check — kube-bench `Check` + kubescape `Control` unified.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    /// Stable identifier, e.g. `cis-1.2.5`, `C-0001`, `T1610` (MITRE technique).
    pub id: String,
    pub framework: Framework,
    pub node_type: NodeType,
    /// Human-readable control name, e.g. "Ensure that the API server uses --authorization-mode=Node,RBAC".
    pub title: String,
    pub description: String,
    pub remediation: String,
    pub severity: Severity,
    pub level: CisLevel,
    /// Free-form labels, e.g. ["api-server", "auth"], ["nsa", "pod-security"].
    pub tags: Vec<String>,
}

impl Check {
    pub fn new(id: impl Into<String>, framework: Framework, node_type: NodeType, title: impl Into<String>) -> Self {
        Check {
            id: id.into(),
            framework,
            node_type,
            title: title.into(),
            description: String::new(),
            remediation: String::new(),
            severity: Severity::Medium,
            level: CisLevel::L1,
            tags: Vec::new(),
        }
    }
}

/// One observed finding for a check against a specific target host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Finding {
    pub check_id: String,
    pub verdict: Verdict,
    pub host: String,
    pub message: String,
    /// Optional supporting evidence captured at scan time (file path, flag value, manifest path).
    pub evidence: Option<String>,
    pub remediation: Option<String>,
    pub severity: Severity,
    pub framework: Framework,
    /// Unix timestamp (seconds since epoch) when finding was recorded.
    pub observed_at: i64,
}

impl Finding {
    pub fn pass(check: &Check, host: impl Into<String>, msg: impl Into<String>) -> Self {
        Finding {
            check_id: check.id.clone(),
            verdict: Verdict::Pass,
            host: host.into(),
            message: msg.into(),
            evidence: None,
            remediation: None,
            severity: check.severity,
            framework: check.framework.clone(),
            observed_at: now_ts(),
        }
    }
    pub fn fail(check: &Check, host: impl Into<String>, msg: impl Into<String>) -> Self {
        Finding {
            check_id: check.id.clone(),
            verdict: Verdict::Fail,
            host: host.into(),
            message: msg.into(),
            evidence: None,
            remediation: Some(check.remediation.clone()),
            severity: check.severity,
            framework: check.framework.clone(),
            observed_at: now_ts(),
        }
    }
    pub fn warn(check: &Check, host: impl Into<String>, msg: impl Into<String>) -> Self {
        Finding {
            check_id: check.id.clone(),
            verdict: Verdict::Warn,
            host: host.into(),
            message: msg.into(),
            evidence: None,
            remediation: Some(check.remediation.clone()),
            severity: check.severity,
            framework: check.framework.clone(),
            observed_at: now_ts(),
        }
    }
    pub fn with_evidence(mut self, e: impl Into<String>) -> Self {
        self.evidence = Some(e.into());
        self
    }
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Where a scan executes against — kube-bench `cfg/config.yaml::target_mapping`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TargetKind {
    /// Local filesystem on a host (kubelet/kube-apiserver process files, etc).
    HostFiles,
    /// A kubeconfig pointing at a live cluster.
    Cluster,
    /// One or more YAML/JSON manifest files (offline scan).
    Manifests,
    /// CI-style: a directory of manifests.
    ManifestDir,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Target {
    pub kind: TargetKind,
    pub identifier: String,
    /// Host name (for HostFiles) or cluster context (for Cluster).
    pub host: String,
    pub node_types: Vec<NodeType>,
}

impl Target {
    pub fn host_files(identifier: impl Into<String>, host: impl Into<String>) -> Self {
        Target {
            kind: TargetKind::HostFiles,
            identifier: identifier.into(),
            host: host.into(),
            node_types: vec![NodeType::Master, NodeType::Node, NodeType::Etcd],
        }
    }
    pub fn manifests(identifier: impl Into<String>, host: impl Into<String>) -> Self {
        Target {
            kind: TargetKind::Manifests,
            identifier: identifier.into(),
            host: host.into(),
            node_types: vec![NodeType::Policies],
        }
    }
}

/// A predefined collection of check IDs that constitute one assessment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub framework: Framework,
    pub name: String,
    pub description: String,
    pub check_ids: Vec<String>,
}

/// Roll-up of a single scan execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScanSummary {
    pub scan_id: String,
    pub profile_id: String,
    pub target: Target,
    pub started_at: i64,
    pub finished_at: i64,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub warned: usize,
    pub errored: usize,
    pub na: usize,
    /// PASS / (PASS+FAIL) — excludes warn/info/na/error.
    pub score: f64,
}

impl ScanSummary {
    pub fn compute(scan_id: impl Into<String>, profile_id: impl Into<String>, target: Target, findings: &[Finding], started_at: i64, finished_at: i64) -> Self {
        let mut passed = 0;
        let mut failed = 0;
        let mut warned = 0;
        let mut errored = 0;
        let mut na = 0;
        for f in findings {
            match f.verdict {
                Verdict::Pass => passed += 1,
                Verdict::Fail => failed += 1,
                Verdict::Warn => warned += 1,
                Verdict::Error => errored += 1,
                Verdict::NotApplicable => na += 1,
                Verdict::Info => {}
            }
        }
        let denom = passed + failed;
        let score = if denom == 0 { 0.0 } else { passed as f64 / denom as f64 };
        ScanSummary {
            scan_id: scan_id.into(),
            profile_id: profile_id.into(),
            target,
            started_at,
            finished_at,
            total: findings.len(),
            passed,
            failed,
            warned,
            errored,
            na,
            score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verdict_is_failure() {
        assert!(Verdict::Fail.is_failure());
        assert!(Verdict::Error.is_failure());
        assert!(!Verdict::Pass.is_failure());
        assert!(!Verdict::Warn.is_failure());
    }

    #[test]
    fn test_severity_score_monotonic() {
        assert!(Severity::Critical.score() > Severity::High.score());
        assert!(Severity::High.score() > Severity::Medium.score());
        assert!(Severity::Medium.score() > Severity::Low.score());
        assert!(Severity::Low.score() > Severity::Info.score());
    }

    #[test]
    fn test_check_new_defaults_l1_medium() {
        let c = Check::new("cis-1.0", Framework::CisK8s, NodeType::Master, "Test");
        assert_eq!(c.level, CisLevel::L1);
        assert_eq!(c.severity, Severity::Medium);
        assert!(c.remediation.is_empty());
    }

    #[test]
    fn test_finding_pass_and_fail() {
        let c = Check::new("c1", Framework::CisK8s, NodeType::Master, "T");
        let p = Finding::pass(&c, "h1", "ok");
        assert_eq!(p.verdict, Verdict::Pass);
        let f = Finding::fail(&c, "h1", "bad");
        assert_eq!(f.verdict, Verdict::Fail);
        assert!(f.remediation.is_some());
    }

    #[test]
    fn test_target_host_files() {
        let t = Target::host_files("/etc/kubernetes", "node-1");
        assert!(matches!(t.kind, TargetKind::HostFiles));
        assert_eq!(t.node_types.len(), 3);
    }

    #[test]
    fn test_scan_summary_score() {
        let c = Check::new("c1", Framework::CisK8s, NodeType::Master, "T");
        let findings = vec![
            Finding::pass(&c, "h1", "ok"),
            Finding::pass(&c, "h1", "ok"),
            Finding::fail(&c, "h1", "bad"),
        ];
        let t = Target::host_files("/etc", "h1");
        let s = ScanSummary::compute("s1", "cis-1.9", t, &findings, 0, 1);
        assert_eq!(s.total, 3);
        assert_eq!(s.passed, 2);
        assert_eq!(s.failed, 1);
        assert!((s.score - 2.0 / 3.0).abs() < 0.001);
    }
}
