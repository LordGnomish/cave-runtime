// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kubescape — "security" framework (security baseline).
//!
//! Upstream: kubescape/kubescape v4.0.8 + kubescape/regolibrary
//! `frameworks/security.json`. License: Apache-2.0 (line-port of the 37
//! control IDs + titles + severities that constitute the security baseline).
//!
//! The security framework is kubescape's curated "assess potential security
//! threats" baseline — a 37-control superset spanning pod hardening, RBAC
//! exposure, kubelet auth, resource limits, encryption and end-of-life
//! component detection. Many controls overlap the NSA catalogue, so the
//! evaluator embeds [`NsaManifestFacts`] and adds the security-specific
//! fact surface (kubelet auth, runtime-socket mounts, version currency …).

use crate::kubescape_nsa::NsaManifestFacts;
use crate::models::{Check, CisLevel, Finding, Framework, NodeType, Severity, Verdict};
use serde::{Deserialize, Serialize};

/// One scoped security-baseline control + the predicate keyword it evaluates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecurityControl {
    pub check: Check,
    pub predicate: String,
}

/// The 37 controls of kubescape `frameworks/security.json`, in catalogue order.
pub fn security_controls() -> Vec<SecurityControl> {
    vec![
        ctrl("C-0005", "API server insecure port", Severity::Critical, "insecure_port_zero"),
        ctrl("C-0012", "Applications credentials in configuration files", Severity::High, "hardcoded_creds"),
        ctrl("C-0013", "Non-root containers", Severity::High, "run_as_non_root"),
        ctrl("C-0016", "Allow privilege escalation", Severity::High, "no_priv_esc"),
        ctrl("C-0017", "Immutable container filesystem", Severity::Medium, "read_only_root_fs"),
        ctrl("C-0034", "Automatic mapping of service account", Severity::Medium, "automount_sa_token_false"),
        ctrl("C-0035", "Administrative Roles (cluster-admin binding)", Severity::Critical, "cluster_admin_binding"),
        ctrl("C-0038", "Host PID/IPC privileges", Severity::Critical, "host_namespace_false"),
        ctrl("C-0041", "HostNetwork access", Severity::Critical, "host_network_false"),
        ctrl("C-0044", "Container hostPort", Severity::Medium, "no_host_port"),
        ctrl("C-0045", "Writable hostPath mount", Severity::Critical, "host_path_readonly"),
        ctrl("C-0046", "Insecure capabilities", Severity::High, "capabilities_drop_all"),
        ctrl("C-0048", "HostPath mount", Severity::High, "no_host_path"),
        ctrl("C-0057", "Privileged container", Severity::Critical, "no_privileged"),
        ctrl("C-0066", "Secret/etcd encryption enabled", Severity::Critical, "etcd_encryption"),
        ctrl("C-0069", "Disable anonymous access to Kubelet service", Severity::High, "kubelet_anonymous_auth_disabled"),
        ctrl("C-0070", "Enforce Kubelet client TLS authentication", Severity::High, "kubelet_client_tls"),
        ctrl("C-0074", "Container runtime socket mounted", Severity::High, "no_runtime_socket_mount"),
        ctrl("C-0211", "Apply Security Context to your pods and containers", Severity::Medium, "security_context_applied"),
        ctrl("C-0255", "Workload with secret access", Severity::Medium, "workload_secret_access_reviewed"),
        ctrl("C-0256", "External facing", Severity::High, "not_external_facing"),
        ctrl("C-0257", "Workload with PVC access", Severity::Medium, "workload_pvc_access_reviewed"),
        ctrl("C-0258", "Workload with ConfigMap access", Severity::Low, "workload_configmap_access_reviewed"),
        ctrl("C-0259", "Workload with credential access", Severity::High, "workload_credential_access_reviewed"),
        ctrl("C-0260", "Missing network policy", Severity::Medium, "network_policy_default_deny"),
        ctrl("C-0261", "ServiceAccount token mounted", Severity::Medium, "automount_sa_token_false"),
        ctrl("C-0262", "Anonymous user has RoleBinding", Severity::High, "no_anonymous_rolebinding"),
        ctrl("C-0264", "PersistentVolume without encryption", Severity::Medium, "pv_encrypted"),
        ctrl("C-0265", "system:authenticated user has elevated roles", Severity::High, "no_authenticated_elevated"),
        ctrl("C-0266", "Exposure to internet via Gateway API or Istio Ingress", Severity::High, "not_gateway_internet_exposed"),
        ctrl("C-0267", "Workload with cluster takeover roles", Severity::High, "no_cluster_takeover_roles"),
        ctrl("C-0270", "Ensure CPU limits are set", Severity::Medium, "cpu_limit"),
        ctrl("C-0271", "Ensure memory limits are set", Severity::Medium, "mem_limit"),
        ctrl("C-0272", "Workload with administrative roles", Severity::High, "no_admin_roles"),
        ctrl("C-0273", "Outdated Kubernetes version", Severity::Medium, "k8s_version_supported"),
        ctrl("C-0274", "Verify Authenticated Service", Severity::Medium, "service_authenticated"),
        ctrl("C-0292", "NGINX Ingress Controller End of Life", Severity::Medium, "no_nginx_ingress_eol"),
    ]
}

fn ctrl(id: &str, title: &str, sev: Severity, predicate: &str) -> SecurityControl {
    let mut c = Check::new(id, Framework::SecurityBaseline, NodeType::Policies, title);
    c.severity = sev;
    c.description = format!("kubescape security framework control {id}: {title}.");
    c.level = if sev == Severity::Critical || sev == Severity::High { CisLevel::L1 } else { CisLevel::L2 };
    c.tags = vec!["security".into(), "kubescape".into()];
    c.remediation = format!("Remediate per kubescape control {id} — {title}.");
    SecurityControl { check: c, predicate: predicate.into() }
}

/// Facts captured for the security framework: the shared NSA surface plus the
/// security-specific signals that the baseline adds on top.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityFacts {
    /// Shared pod/RBAC/resource fact surface (reused from the NSA evaluator).
    pub nsa: NsaManifestFacts,
    // ─ security-specific signals ─
    pub kubelet_anonymous_auth_disabled: bool,
    pub kubelet_client_tls: bool,
    pub container_runtime_socket_mounted: bool,
    pub security_context_applied: bool,
    pub anonymous_rolebinding: bool,
    pub authenticated_elevated_roles: bool,
    pub gateway_internet_exposed: bool,
    pub cluster_takeover_roles: bool,
    pub admin_roles: bool,
    pub pv_encrypted: bool,
    pub k8s_version_outdated: bool,
    pub nginx_ingress_eol: bool,
    pub service_authenticated: bool,
    /// Sensitive-access workloads that have NOT been reviewed/justified.
    pub unreviewed_sensitive_access: bool,
}

/// Evaluate one security-baseline control against captured facts.
pub fn evaluate_security_control(c: &SecurityControl, facts: &SecurityFacts, host: &str) -> Finding {
    let n = &facts.nsa;
    let pass = match c.predicate.as_str() {
        // ─ shared NSA-backed predicates ─
        "insecure_port_zero" => n.apiserver_insecure_port == 0,
        "hardcoded_creds" => !n.hardcoded_creds,
        "run_as_non_root" => n.run_as_non_root,
        "no_priv_esc" => !n.allow_priv_esc,
        "read_only_root_fs" => n.read_only_root_fs,
        "automount_sa_token_false" => !n.automount_sa_token,
        "cluster_admin_binding" => n.cluster_admin_count == 0,
        "host_namespace_false" => !n.host_pid && !n.host_ipc && !n.host_network,
        "host_network_false" => !n.host_network,
        "no_host_port" => !n.host_port,
        "host_path_readonly" => !n.host_path_writable,
        "capabilities_drop_all" => n.caps_drop_all,
        "no_host_path" => !n.host_path_present,
        "no_privileged" => !n.privileged,
        "etcd_encryption" => n.apiserver_encryption_provider,
        "network_policy_default_deny" => n.default_deny_netpol,
        "cpu_limit" => n.cpu_limit_set,
        "mem_limit" => n.mem_limit_set,
        // ─ security-specific predicates ─
        "kubelet_anonymous_auth_disabled" => facts.kubelet_anonymous_auth_disabled,
        "kubelet_client_tls" => facts.kubelet_client_tls,
        "no_runtime_socket_mount" => !facts.container_runtime_socket_mounted,
        "security_context_applied" => facts.security_context_applied,
        "no_anonymous_rolebinding" => !facts.anonymous_rolebinding,
        "no_authenticated_elevated" => !facts.authenticated_elevated_roles,
        "not_gateway_internet_exposed" => !facts.gateway_internet_exposed,
        "no_cluster_takeover_roles" => !facts.cluster_takeover_roles,
        "no_admin_roles" => !facts.admin_roles,
        "pv_encrypted" => facts.pv_encrypted,
        "k8s_version_supported" => !facts.k8s_version_outdated,
        "service_authenticated" => facts.service_authenticated,
        "no_nginx_ingress_eol" => !facts.nginx_ingress_eol,
        "not_external_facing"
        | "workload_secret_access_reviewed"
        | "workload_pvc_access_reviewed"
        | "workload_configmap_access_reviewed"
        | "workload_credential_access_reviewed" => !facts.unreviewed_sensitive_access,
        _ => return Finding::warn(&c.check, host, format!("Predicate '{}' is review-only", c.predicate)),
    };
    if pass {
        Finding::pass(&c.check, host, format!("{}: control satisfied", c.check.id))
    } else {
        let msg = format!("{}: '{}' predicate failed — {}", c.check.id, c.predicate, c.check.title);
        let mut f = Finding::fail(&c.check, host, msg);
        f.verdict = Verdict::Fail;
        f
    }
}

/// All control IDs of the security framework (catalogue order) — used by the
/// profile builder.
pub fn security_control_ids() -> Vec<String> {
    security_controls().into_iter().map(|c| c.check.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_controls_security_framework() {
        for c in security_controls() {
            assert_eq!(c.check.framework, Framework::SecurityBaseline);
        }
    }

    #[test]
    fn test_baseline_count_is_37() {
        assert_eq!(security_controls().len(), 37);
    }

    #[test]
    fn test_kubelet_tls_predicate() {
        let c = security_controls().into_iter().find(|c| c.check.id == "C-0070").unwrap();
        let mut facts = SecurityFacts::default();
        assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Fail);
        facts.kubelet_client_tls = true;
        assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Pass);
    }

    #[test]
    fn test_nginx_eol_predicate() {
        let c = security_controls().into_iter().find(|c| c.check.id == "C-0292").unwrap();
        let mut facts = SecurityFacts::default();
        assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Pass);
        facts.nginx_ingress_eol = true;
        assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Fail);
    }

    #[test]
    fn test_shared_nsa_fact_cpu_limit() {
        let c = security_controls().into_iter().find(|c| c.check.id == "C-0270").unwrap();
        let mut facts = SecurityFacts::default();
        assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Fail);
        facts.nsa.cpu_limit_set = true;
        assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Pass);
    }

    #[test]
    fn test_control_ids_helper_len() {
        assert_eq!(security_control_ids().len(), 37);
    }
}
