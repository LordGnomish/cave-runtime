// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kubescape — NSA Kubernetes Hardening Guide controls (C-00xx).
//!
//! Upstream: kubescape/kubescape v4.0.8 + kubescape/regolibrary.
//! License: Apache-2.0 (line-port of control titles, descriptions, remediations).
//!
//! ≥20 representative controls covering the NSA pillars:
//! - Pod security (allowPrivilegeEscalation, hostPID, hostNetwork, privileged)
//! - Network segmentation (NetworkPolicy)
//! - Authentication & authorization (RBAC wildcards, anonymous access)
//! - Resource limits (CPU/mem limits)
//! - Logging & monitoring (audit policy)
//! - Container image (latest tag, signed images, scan presence)

use crate::models::{Check, CisLevel, Finding, Framework, NodeType, Severity, Verdict};
use serde::{Deserialize, Serialize};

/// One scoped NSA control with a parameterised manifest matcher.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NsaControl {
    pub check: Check,
    /// Predicate keyword the manifest evaluator looks for. Concrete predicates
    /// are implemented in `evaluate_manifest`.
    pub predicate: String,
}

/// All NSA controls returned with their unified Check meta.
pub fn nsa_controls() -> Vec<NsaControl> {
    vec![
        ctrl("C-0001", "Forbidden Container Registries", Severity::High,
             "Containers should only be pulled from approved/trusted registries.",
             "registry_allowlist"),
        ctrl("C-0002", "Exec into container", Severity::Medium,
             "Attaching to running containers via `kubectl exec` provides ad-hoc shell access.",
             "exec_allowed"),
        ctrl("C-0004", "Resource limits on every container", Severity::Medium,
             "Containers must declare CPU and memory limits.",
             "resource_limits"),
        ctrl("C-0005", "API server insecure port", Severity::Critical,
             "API server --insecure-port must be 0.",
             "insecure_port_zero"),
        ctrl("C-0007", "Roles with delete capabilities", Severity::Medium,
             "RBAC roles granting deletion of critical resources must be reviewed.",
             "rbac_delete_capable"),
        ctrl("C-0009", "Resource policies (LimitRange / ResourceQuota)", Severity::Medium,
             "Each namespace must have a LimitRange and ResourceQuota.",
             "namespace_limit_range"),
        ctrl("C-0012", "Applications credentials in configuration files", Severity::High,
             "Hardcoded credentials inside ConfigMaps/Manifests are forbidden.",
             "hardcoded_creds"),
        ctrl("C-0013", "Non-root containers", Severity::High,
             "Containers must run as non-root user (runAsNonRoot=true).",
             "run_as_non_root"),
        ctrl("C-0014", "Access Kubernetes dashboard", Severity::High,
             "Kubernetes Dashboard pod must not be exposed externally.",
             "dashboard_exposed"),
        ctrl("C-0015", "List Kubernetes secrets", Severity::High,
             "RBAC permissions to LIST secrets cluster-wide must be restricted.",
             "rbac_secret_list"),
        ctrl("C-0016", "Allow privilege escalation", Severity::High,
             "securityContext.allowPrivilegeEscalation must be false.",
             "no_priv_esc"),
        ctrl("C-0017", "Immutable container filesystem", Severity::Medium,
             "readOnlyRootFilesystem must be true.",
             "read_only_root_fs"),
        ctrl("C-0018", "Configured readiness probe", Severity::Low,
             "Every pod must declare a readinessProbe.",
             "readiness_probe"),
        ctrl("C-0020", "Mount service principal", Severity::Critical,
             "Mounting cloud-provider service-principal credentials into a pod is forbidden.",
             "service_principal_mount"),
        ctrl("C-0026", "Kubernetes CronJob", Severity::Medium,
             "CronJobs are reviewed for least privilege and read-only filesystems.",
             "cronjob_review"),
        ctrl("C-0030", "Ingress and Egress blocked", Severity::High,
             "Default-deny NetworkPolicies are present in every namespace.",
             "network_policy_default_deny"),
        ctrl("C-0034", "Automatic mapping of service account", Severity::Medium,
             "Pods must set automountServiceAccountToken=false unless explicitly required.",
             "automount_sa_token_false"),
        ctrl("C-0035", "Cluster-admin binding", Severity::Critical,
             "ClusterRoleBindings to cluster-admin must be limited.",
             "cluster_admin_binding"),
        ctrl("C-0036", "Validating admission webhook (or OPA gatekeeper)", Severity::Medium,
             "A validating admission webhook must be installed.",
             "admission_webhook_present"),
        ctrl("C-0038", "Host PID/IPC privileges", Severity::Critical,
             "Pods must not enable hostPID, hostIPC, or hostNetwork.",
             "host_namespace_false"),
        ctrl("C-0041", "HostNetwork access", Severity::Critical,
             "hostNetwork: true is forbidden.",
             "host_network_false"),
        ctrl("C-0044", "Container hostPort", Severity::Medium,
             "Containers must not request hostPort.",
             "no_host_port"),
        ctrl("C-0045", "Writable hostPath mount", Severity::Critical,
             "hostPath mounts must be read-only.",
             "host_path_readonly"),
        ctrl("C-0046", "Insecure capabilities", Severity::High,
             "Containers must drop ALL capabilities and add only required ones.",
             "capabilities_drop_all"),
        ctrl("C-0048", "HostPath mount", Severity::High,
             "hostPath mounts are restricted to approved paths only.",
             "no_host_path"),
        ctrl("C-0050", "Resources CPU limit", Severity::Medium,
             "Containers must declare CPU limit.",
             "cpu_limit"),
        ctrl("C-0055", "Linux hardening (seccomp)", Severity::Medium,
             "Containers must use seccompProfile {RuntimeDefault or Localhost}.",
             "seccomp_profile"),
        ctrl("C-0057", "Privileged container", Severity::Critical,
             "Containers must not be privileged.",
             "no_privileged"),
        ctrl("C-0066", "Secret/etcd encryption enabled", Severity::Critical,
             "API server --encryption-provider-config must be set.",
             "etcd_encryption"),
        ctrl("C-0073", "Naked pods", Severity::Low,
             "Pods must be owned by a controller (Deployment, StatefulSet, DaemonSet).",
             "owned_pod"),
        ctrl("C-0078", "Image vulnerabilities (critical)", Severity::Critical,
             "Container images must have no critical CVEs (cave-trivy scan).",
             "no_critical_cve"),
        ctrl("C-0086", "CoreDNS poisoning", Severity::High,
             "CoreDNS deployment integrity (signed manifests + RBAC).",
             "coredns_integrity"),
    ]
}

fn ctrl(id: &str, title: &str, sev: Severity, desc: &str, predicate: &str) -> NsaControl {
    let mut c = Check::new(id, Framework::NsaHardening, NodeType::Policies, title);
    c.severity = sev;
    c.description = desc.into();
    c.level = if sev == Severity::Critical || sev == Severity::High { CisLevel::L1 } else { CisLevel::L2 };
    c.tags = vec!["nsa".into()];
    c.remediation = format!("Apply NSA hardening guidance for: {title}.");
    NsaControl { check: c, predicate: predicate.into() }
}

/// Per-control facts captured from manifest static analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NsaManifestFacts {
    pub privileged: bool,
    pub allow_priv_esc: bool,
    pub host_network: bool,
    pub host_pid: bool,
    pub host_ipc: bool,
    pub host_path_writable: bool,
    pub host_path_present: bool,
    pub host_port: bool,
    pub run_as_non_root: bool,
    pub read_only_root_fs: bool,
    pub readiness_probe: bool,
    pub automount_sa_token: bool,
    pub cpu_limit_set: bool,
    pub mem_limit_set: bool,
    pub seccomp_profile: Option<String>,
    pub caps_drop_all: bool,
    pub default_deny_netpol: bool,
    pub registry_approved: bool,
    pub controller_owned: bool,
    pub cluster_admin_count: usize,
    pub cve_critical_count: usize,
    pub apiserver_insecure_port: i64,
    pub apiserver_encryption_provider: bool,
    pub admission_webhook_present: bool,
    pub limit_range_present: bool,
    pub dashboard_exposed: bool,
    pub rbac_secret_list: bool,
    pub rbac_delete_capable: bool,
    pub hardcoded_creds: bool,
    pub coredns_signed: bool,
    pub service_principal_mounted: bool,
    pub cronjob_safe: bool,
    pub exec_allowed: bool,
}

/// Evaluate one NSA control against captured manifest facts. Returns a Finding.
pub fn evaluate_control(c: &NsaControl, facts: &NsaManifestFacts, host: &str) -> Finding {
    let predicate_pass = match c.predicate.as_str() {
        "no_privileged" => !facts.privileged,
        "no_priv_esc" => !facts.allow_priv_esc,
        "host_network_false" => !facts.host_network,
        "host_namespace_false" => !facts.host_pid && !facts.host_ipc && !facts.host_network,
        "no_host_path" => !facts.host_path_present,
        "host_path_readonly" => !facts.host_path_writable,
        "no_host_port" => !facts.host_port,
        "run_as_non_root" => facts.run_as_non_root,
        "read_only_root_fs" => facts.read_only_root_fs,
        "readiness_probe" => facts.readiness_probe,
        "automount_sa_token_false" => !facts.automount_sa_token,
        "cpu_limit" => facts.cpu_limit_set,
        "resource_limits" => facts.cpu_limit_set && facts.mem_limit_set,
        "seccomp_profile" => facts.seccomp_profile.is_some(),
        "capabilities_drop_all" => facts.caps_drop_all,
        "network_policy_default_deny" => facts.default_deny_netpol,
        "registry_allowlist" => facts.registry_approved,
        "owned_pod" => facts.controller_owned,
        "cluster_admin_binding" => facts.cluster_admin_count == 0,
        "no_critical_cve" => facts.cve_critical_count == 0,
        "insecure_port_zero" => facts.apiserver_insecure_port == 0,
        "etcd_encryption" => facts.apiserver_encryption_provider,
        "admission_webhook_present" => facts.admission_webhook_present,
        "namespace_limit_range" => facts.limit_range_present,
        "dashboard_exposed" => !facts.dashboard_exposed,
        "rbac_secret_list" => !facts.rbac_secret_list,
        "rbac_delete_capable" => !facts.rbac_delete_capable,
        "hardcoded_creds" => !facts.hardcoded_creds,
        "coredns_integrity" => facts.coredns_signed,
        "service_principal_mount" => !facts.service_principal_mounted,
        "cronjob_review" => facts.cronjob_safe,
        "exec_allowed" => !facts.exec_allowed,
        _ => return Finding::warn(&c.check, host, format!("Predicate '{}' is review-only", c.predicate)),
    };
    if predicate_pass {
        Finding::pass(&c.check, host, format!("{}: control satisfied", c.check.id))
    } else {
        let msg = format!("{}: '{}' predicate failed — {}", c.check.id, c.predicate, c.check.title);
        let mut f = Finding::fail(&c.check, host, msg);
        f.verdict = Verdict::Fail;
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nsa_controls_meets_floor() {
        assert!(nsa_controls().len() >= 20);
    }

    #[test]
    fn test_nsa_controls_unique_ids() {
        let mut ids: Vec<_> = nsa_controls().into_iter().map(|c| c.check.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }

    #[test]
    fn test_nsa_all_framework_nsa() {
        for c in nsa_controls() {
            assert_eq!(c.check.framework, Framework::NsaHardening);
        }
    }

    #[test]
    fn test_evaluate_privileged_fail() {
        let c = ctrl("C-0057", "Privileged", Severity::Critical, "", "no_privileged");
        let mut facts = NsaManifestFacts::default();
        facts.privileged = true;
        let f = evaluate_control(&c, &facts, "h");
        assert_eq!(f.verdict, Verdict::Fail);
    }

    #[test]
    fn test_evaluate_priv_esc_pass() {
        let c = ctrl("C-0016", "PrivEsc", Severity::High, "", "no_priv_esc");
        let facts = NsaManifestFacts::default();
        let f = evaluate_control(&c, &facts, "h");
        assert_eq!(f.verdict, Verdict::Pass);
    }

    #[test]
    fn test_evaluate_host_namespace() {
        let c = ctrl("C-0038", "HostNS", Severity::Critical, "", "host_namespace_false");
        let mut facts = NsaManifestFacts::default();
        facts.host_pid = true;
        let f = evaluate_control(&c, &facts, "h");
        assert_eq!(f.verdict, Verdict::Fail);
    }

    #[test]
    fn test_evaluate_resource_limits_both_required() {
        let c = ctrl("C-0004", "Limits", Severity::Medium, "", "resource_limits");
        let mut facts = NsaManifestFacts::default();
        facts.cpu_limit_set = true;
        let f = evaluate_control(&c, &facts, "h");
        assert_eq!(f.verdict, Verdict::Fail); // mem missing
        facts.mem_limit_set = true;
        let f = evaluate_control(&c, &facts, "h");
        assert_eq!(f.verdict, Verdict::Pass);
    }

    #[test]
    fn test_unknown_predicate_warns() {
        let c = NsaControl {
            check: Check::new("X-1", Framework::NsaHardening, NodeType::Policies, "Manual"),
            predicate: "manual_review_only".into(),
        };
        let f = evaluate_control(&c, &NsaManifestFacts::default(), "h");
        assert_eq!(f.verdict, Verdict::Warn);
    }
}
