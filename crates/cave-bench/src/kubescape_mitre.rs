// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kubescape — MITRE ATT&CK for Kubernetes mapping.
//!
//! Upstream: kubescape/regolibrary `frameworks/MITRE.json`.
//! License: Apache-2.0 (titles + tactic names).
//!
//! 9 tactics × ≥3 techniques each = 27+ entries, covering the full
//! Initial Access → Impact kill chain.

use crate::models::{Check, CisLevel, Framework, NodeType, Severity};
use serde::{Deserialize, Serialize};

/// One MITRE ATT&CK tactic — a phase of attack.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Tactic {
    InitialAccess,
    Execution,
    Persistence,
    PrivilegeEscalation,
    DefenseEvasion,
    CredentialAccess,
    Discovery,
    LateralMovement,
    Collection,
    Impact,
}

impl Tactic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tactic::InitialAccess => "initial-access",
            Tactic::Execution => "execution",
            Tactic::Persistence => "persistence",
            Tactic::PrivilegeEscalation => "privilege-escalation",
            Tactic::DefenseEvasion => "defense-evasion",
            Tactic::CredentialAccess => "credential-access",
            Tactic::Discovery => "discovery",
            Tactic::LateralMovement => "lateral-movement",
            Tactic::Collection => "collection",
            Tactic::Impact => "impact",
        }
    }
}

/// One ATT&CK technique, mapped to a Check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MitreTechnique {
    /// Technique ID, e.g. `T1190` (Exploit Public-Facing Application).
    pub id: String,
    pub tactic: Tactic,
    pub check: Check,
    /// Detection guidance shipped from kubescape.
    pub detection: String,
}

/// All techniques mapped, 9 tactics × ≥3 techniques.
pub fn mitre_techniques() -> Vec<MitreTechnique> {
    let mut out = Vec::new();

    // ── Initial Access ────────────────────────────────────────────────────────
    out.push(tech("T1190", Tactic::InitialAccess, "Exploit Public-Facing Application", Severity::Critical,
        "Detect cluster-edge LoadBalancer/Ingress exposing application endpoints to internet."));
    out.push(tech("T1133", Tactic::InitialAccess, "External Remote Services", Severity::High,
        "Detect exposed Kubernetes Dashboard, kubectl proxy or remote-exec."));
    out.push(tech("T1078", Tactic::InitialAccess, "Valid Accounts", Severity::High,
        "Detect anonymous-auth=true or service-account token reuse from outside cluster."));

    // ── Execution ─────────────────────────────────────────────────────────────
    out.push(tech("T1610", Tactic::Execution, "Deploy Container", Severity::High,
        "Detect new privileged-pod admissions outside known controllers."));
    out.push(tech("T1059", Tactic::Execution, "Command and Scripting Interpreter", Severity::Medium,
        "Detect kubectl exec into prod pods (cross-correlated with audit log)."));
    out.push(tech("T1609", Tactic::Execution, "Container Administration Command", Severity::High,
        "Detect kubectl run with custom-entrypoint."));

    // ── Persistence ───────────────────────────────────────────────────────────
    out.push(tech("T1543", Tactic::Persistence, "Create or Modify System Process", Severity::High,
        "Detect creation of DaemonSets running on every node."));
    out.push(tech("T1098", Tactic::Persistence, "Account Manipulation", Severity::Critical,
        "Detect cluster-admin role-binding modifications."));
    out.push(tech("T1525", Tactic::Persistence, "Implant Internal Image", Severity::High,
        "Detect new image being pushed to internal registry with sensitive labels."));

    // ── Privilege Escalation ──────────────────────────────────────────────────
    out.push(tech("T1611", Tactic::PrivilegeEscalation, "Escape to Host", Severity::Critical,
        "Detect host-namespace flags (hostPID/hostIPC/hostNetwork) on new pods."));
    out.push(tech("T1078.001", Tactic::PrivilegeEscalation, "Default Accounts", Severity::High,
        "Detect default service-account being used to access secrets."));
    out.push(tech("T1068", Tactic::PrivilegeEscalation, "Exploitation for Privilege Escalation", Severity::Critical,
        "Detect kernel-exploit indicators (capabilities + sysctl)."));

    // ── Defense Evasion ───────────────────────────────────────────────────────
    out.push(tech("T1036", Tactic::DefenseEvasion, "Masquerading", Severity::Medium,
        "Detect pod-name patterns mimicking system pods (kube-*, etcd-*)."));
    out.push(tech("T1562", Tactic::DefenseEvasion, "Impair Defenses", Severity::High,
        "Detect deletion of NetworkPolicies, audit-policy or admission webhooks."));
    out.push(tech("T1070", Tactic::DefenseEvasion, "Indicator Removal", Severity::High,
        "Detect log-tail truncation events or audit-policy edit."));

    // ── Credential Access ─────────────────────────────────────────────────────
    out.push(tech("T1552.007", Tactic::CredentialAccess, "Container API", Severity::High,
        "Detect pods reading service-account-token files via mountedSAToken."));
    out.push(tech("T1539", Tactic::CredentialAccess, "Steal Web Session Cookie", Severity::Medium,
        "Detect kubeconfig token harvesting."));
    out.push(tech("T1212", Tactic::CredentialAccess, "Exploitation for Credential Access", Severity::High,
        "Detect kube-apiserver --insecure-port=8080 listening on host."));

    // ── Discovery ─────────────────────────────────────────────────────────────
    out.push(tech("T1613", Tactic::Discovery, "Container and Resource Discovery", Severity::Low,
        "Detect pods enumerating cluster resources via /api or /apis."));
    out.push(tech("T1018", Tactic::Discovery, "Remote System Discovery", Severity::Low,
        "Detect port-scan from inside cluster (mass-connect events)."));
    out.push(tech("T1046", Tactic::Discovery, "Network Service Discovery", Severity::Medium,
        "Detect cluster-internal nmap-style scans via Hubble flows."));

    // ── Lateral Movement ──────────────────────────────────────────────────────
    out.push(tech("T1021", Tactic::LateralMovement, "Remote Services", Severity::High,
        "Detect cross-namespace SSH/RDP-equivalent traffic."));
    out.push(tech("T1550", Tactic::LateralMovement, "Use Alternate Authentication Material", Severity::High,
        "Detect bearer-token reuse from a different IP/pod."));
    out.push(tech("T1080", Tactic::LateralMovement, "Taint Shared Content", Severity::Medium,
        "Detect modification of shared PVCs across namespaces."));

    // ── Collection ────────────────────────────────────────────────────────────
    out.push(tech("T1602", Tactic::Collection, "Data from Configuration Repository", Severity::Medium,
        "Detect mass-read of ConfigMaps."));
    out.push(tech("T1530", Tactic::Collection, "Data from Cloud Storage", Severity::High,
        "Detect S3/GCS-presigned URL emission from pods."));
    out.push(tech("T1213", Tactic::Collection, "Data from Information Repositories", Severity::Medium,
        "Detect bulk secret retrieval."));

    // ── Impact ────────────────────────────────────────────────────────────────
    out.push(tech("T1485", Tactic::Impact, "Data Destruction", Severity::Critical,
        "Detect bulk DELETE against secrets, configmaps, namespaces."));
    out.push(tech("T1499", Tactic::Impact, "Endpoint Denial of Service", Severity::High,
        "Detect node-level cpu/mem flood from unprivileged tenants."));
    out.push(tech("T1496", Tactic::Impact, "Resource Hijacking", Severity::Critical,
        "Detect cryptominer-style sustained 100% CPU."));

    out
}

fn tech(id: &str, tactic: Tactic, title: &str, sev: Severity, detection: &str) -> MitreTechnique {
    let mut c = Check::new(id, Framework::MitreAttack, NodeType::Policies, title);
    c.severity = sev;
    c.level = CisLevel::L2;
    c.tags = vec!["mitre".into(), tactic.as_str().to_string()];
    c.description = detection.to_string();
    c.remediation = format!("Review detection signal for {id} ({}).", tactic.as_str());
    MitreTechnique {
        id: id.into(),
        tactic,
        check: c,
        detection: detection.into(),
    }
}

/// Group techniques by tactic for coverage reports.
pub fn group_by_tactic(t: &[MitreTechnique]) -> std::collections::HashMap<Tactic, Vec<MitreTechnique>> {
    let mut m: std::collections::HashMap<Tactic, Vec<MitreTechnique>> = std::collections::HashMap::new();
    for x in t {
        m.entry(x.tactic).or_default().push(x.clone());
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mitre_techniques_have_at_least_nine_tactics() {
        let tt = mitre_techniques();
        let g = group_by_tactic(&tt);
        assert!(g.len() >= 9, "expected ≥9 tactics, got {}", g.len());
    }

    #[test]
    fn test_each_tactic_has_at_least_three() {
        for (t, v) in group_by_tactic(&mitre_techniques()) {
            assert!(v.len() >= 3, "tactic {t:?} has only {} techniques", v.len());
        }
    }

    #[test]
    fn test_mitre_techniques_unique_ids() {
        let mut ids: Vec<_> = mitre_techniques().into_iter().map(|t| t.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }

    #[test]
    fn test_mitre_all_framework_mitre() {
        for t in mitre_techniques() {
            assert_eq!(t.check.framework, Framework::MitreAttack);
        }
    }

    #[test]
    fn test_tactic_as_str() {
        assert_eq!(Tactic::Impact.as_str(), "impact");
        assert_eq!(Tactic::InitialAccess.as_str(), "initial-access");
    }

    #[test]
    fn test_critical_techniques_present() {
        let c_count: usize = mitre_techniques()
            .iter()
            .filter(|t| t.check.severity == Severity::Critical)
            .count();
        assert!(c_count >= 5);
    }
}
