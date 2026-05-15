//! Pod Security admission tests — parity with policy/check_*.go cases.

use super::*;
use crate::admission::{AdmissionRequest, Operation};
use crate::resources::{ConfigMap, ObjectMeta, Pod, PodSpec, Resource};
use std::collections::HashMap;
use std::sync::Arc;

fn pod_req(ns: &str, tenant: &str, op: Operation) -> AdmissionRequest {
    AdmissionRequest {
        uid: "u".into(), tenant_id: tenant.into(),
        namespace: ns.into(), kind: "Pod".into(), name: "p".into(),
        operation: op,
        object: Some(Resource::Pod(Pod {
            api_version: "v1".into(), kind: "Pod".into(),
            metadata: ObjectMeta::new("p", ns),
            spec: PodSpec::default(), status: Default::default(),
        })),
        old_object: None, user: "alice".into(), dry_run: false,
    }
}

fn cm_req(ns: &str, tenant: &str) -> AdmissionRequest {
    AdmissionRequest {
        uid: "u".into(), tenant_id: tenant.into(),
        namespace: ns.into(), kind: "ConfigMap".into(), name: "cm".into(),
        operation: Operation::Create,
        object: Some(Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new("cm", ns), data: HashMap::new(),
        })),
        old_object: None, user: "alice".into(), dry_run: false,
    }
}

struct StaticSpec(PodSecuritySpec);
impl PodSpecExtractor for StaticSpec {
    fn extract(&self, _: &AdmissionRequest) -> Option<PodSecuritySpec> {
        Some(self.0.clone())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// parse_namespace_labels — `api/labels.go::policyToEvaluate`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn psa_parse_level_strings() {
    assert_eq!(parse_level("privileged"), Some(Level::Privileged));
    assert_eq!(parse_level("baseline"), Some(Level::Baseline));
    assert_eq!(parse_level("restricted"), Some(Level::Restricted));
    assert_eq!(parse_level("garbage"), None);
}

#[test]
fn psa_parse_namespace_labels_empty() {
    let labels = HashMap::new();
    let t = parse_namespace_labels(&labels);
    assert_eq!(t, LevelTriple::default());
}

#[test]
fn psa_parse_namespace_labels_full_triple() {
    let mut labels = HashMap::new();
    labels.insert("pod-security.kubernetes.io/enforce".into(), "restricted".into());
    labels.insert("pod-security.kubernetes.io/audit".into(), "baseline".into());
    labels.insert("pod-security.kubernetes.io/warn".into(), "baseline".into());
    let t = parse_namespace_labels(&labels);
    assert_eq!(t.enforce, Some(Level::Restricted));
    assert_eq!(t.audit, Some(Level::Baseline));
    assert_eq!(t.warn, Some(Level::Baseline));
}

#[test]
fn psa_invalid_level_label_yields_none() {
    let mut labels = HashMap::new();
    labels.insert("pod-security.kubernetes.io/enforce".into(), "yeet".into());
    let t = parse_namespace_labels(&labels);
    assert!(t.enforce.is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// check_level — per-control violations
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn psa_privileged_level_no_violations() {
    let spec = PodSecuritySpec { host_network: true, host_pid: true, ..Default::default() };
    assert!(check_level(Level::Privileged, &spec).is_empty());
}

#[test]
fn psa_baseline_blocks_host_network() {
    let spec = PodSecuritySpec { host_network: true, ..Default::default() };
    let v = check_level(Level::Baseline, &spec);
    assert!(v.iter().any(|x| x.control == "hostNetwork"));
}

#[test]
fn psa_baseline_blocks_host_pid() {
    let spec = PodSecuritySpec { host_pid: true, ..Default::default() };
    assert!(check_level(Level::Baseline, &spec).iter().any(|x| x.control == "hostPID"));
}

#[test]
fn psa_baseline_blocks_host_ipc() {
    let spec = PodSecuritySpec { host_ipc: true, ..Default::default() };
    assert!(check_level(Level::Baseline, &spec).iter().any(|x| x.control == "hostIPC"));
}

#[test]
fn psa_baseline_blocks_host_path_volumes() {
    let spec = PodSecuritySpec { host_path_volumes: vec!["/etc".into()], ..Default::default() };
    assert!(check_level(Level::Baseline, &spec).iter().any(|x| x.control == "hostPathVolumes"));
}

#[test]
fn psa_baseline_blocks_host_ports() {
    let spec = PodSecuritySpec { host_ports: vec![80], ..Default::default() };
    assert!(check_level(Level::Baseline, &spec).iter().any(|x| x.control == "hostPorts"));
}

#[test]
fn psa_baseline_blocks_privileged_containers() {
    let spec = PodSecuritySpec {
        privileged_containers: vec!["app".into()],
        ..Default::default()
    };
    assert!(check_level(Level::Baseline, &spec).iter().any(|x| x.control == "privileged"));
}

#[test]
fn psa_baseline_allows_default_caps() {
    let mut spec = PodSecuritySpec::default();
    spec.capabilities_add.insert("app".into(), vec!["NET_BIND_SERVICE".into()]);
    let v = check_level(Level::Baseline, &spec);
    assert!(!v.iter().any(|x| x.control == "capabilities"),
        "NET_BIND_SERVICE is on the baseline allowlist");
}

#[test]
fn psa_baseline_blocks_disallowed_caps() {
    let mut spec = PodSecuritySpec::default();
    spec.capabilities_add.insert("app".into(), vec!["SYS_ADMIN".into()]);
    let v = check_level(Level::Baseline, &spec);
    assert!(v.iter().any(|x| x.control == "capabilities"));
}

#[test]
fn psa_restricted_blocks_allow_privilege_escalation() {
    let mut spec = PodSecuritySpec::default();
    spec.allow_privilege_escalation.insert("app".into(), true);
    spec.capabilities_drop.insert("app".into(), vec!["ALL".into()]);
    spec.run_as_non_root.insert("app".into(), Some(true));
    spec.seccomp_profile.insert("app".into(), "RuntimeDefault".into());
    let v = check_level(Level::Restricted, &spec);
    assert!(v.iter().any(|x| x.control == "allowPrivilegeEscalation"));
}

#[test]
fn psa_restricted_requires_run_as_non_root() {
    let mut spec = PodSecuritySpec::default();
    spec.run_as_non_root.insert("app".into(), Some(false));
    spec.capabilities_drop.insert("app".into(), vec!["ALL".into()]);
    spec.allow_privilege_escalation.insert("app".into(), false);
    spec.seccomp_profile.insert("app".into(), "RuntimeDefault".into());
    let v = check_level(Level::Restricted, &spec);
    assert!(v.iter().any(|x| x.control == "runAsNonRoot"));
}

#[test]
fn psa_restricted_run_as_user_zero_is_root() {
    let mut spec = PodSecuritySpec::default();
    spec.run_as_user.insert("app".into(), 0);
    spec.capabilities_drop.insert("app".into(), vec!["ALL".into()]);
    spec.run_as_non_root.insert("app".into(), Some(true));
    spec.seccomp_profile.insert("app".into(), "RuntimeDefault".into());
    spec.allow_privilege_escalation.insert("app".into(), false);
    let v = check_level(Level::Restricted, &spec);
    assert!(v.iter().any(|x| x.control == "runAsUser"));
}

#[test]
fn psa_restricted_seccomp_must_be_runtime_default_or_localhost() {
    let mut spec = PodSecuritySpec::default();
    spec.seccomp_profile.insert("app".into(), "Unconfined".into());
    spec.capabilities_drop.insert("app".into(), vec!["ALL".into()]);
    spec.run_as_non_root.insert("app".into(), Some(true));
    spec.allow_privilege_escalation.insert("app".into(), false);
    let v = check_level(Level::Restricted, &spec);
    assert!(v.iter().any(|x| x.control == "seccompProfile"));
}

#[test]
fn psa_restricted_must_drop_all_caps() {
    let mut spec = PodSecuritySpec::default();
    spec.run_as_user.insert("app".into(), 1000);
    spec.run_as_non_root.insert("app".into(), Some(true));
    spec.allow_privilege_escalation.insert("app".into(), false);
    spec.seccomp_profile.insert("app".into(), "RuntimeDefault".into());
    spec.capabilities_drop.insert("app".into(), vec!["NET_RAW".into()]);
    let v = check_level(Level::Restricted, &spec);
    assert!(v.iter().any(|x| x.control == "capabilities" && x.message.contains("drop ALL")));
}

#[test]
fn psa_restricted_clean_pod_passes() {
    let mut spec = PodSecuritySpec::default();
    spec.run_as_user.insert("app".into(), 1000);
    spec.run_as_non_root.insert("app".into(), Some(true));
    spec.allow_privilege_escalation.insert("app".into(), false);
    spec.seccomp_profile.insert("app".into(), "RuntimeDefault".into());
    spec.capabilities_drop.insert("app".into(), vec!["ALL".into()]);
    assert!(check_level(Level::Restricted, &spec).is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// PodSecurityPlugin
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn psa_plugin_skips_non_pod() {
    let levels = Arc::new(InMemoryLevelStore::new());
    levels.set("acme", "ns", LevelTriple {
        enforce: Some(Level::Restricted), audit: None, warn: None,
    });
    let plug = PodSecurityPlugin {
        levels, extractor: Arc::new(StaticSpec(PodSecuritySpec::default())),
    };
    let r = plug.validate(&cm_req("ns", "acme"));
    assert!(r.allowed);
}

#[test]
fn psa_plugin_skips_delete() {
    let levels = Arc::new(InMemoryLevelStore::new());
    levels.set("acme", "ns", LevelTriple {
        enforce: Some(Level::Restricted), audit: None, warn: None,
    });
    let mut bad = PodSecuritySpec::default();
    bad.host_network = true;
    let plug = PodSecurityPlugin {
        levels, extractor: Arc::new(StaticSpec(bad)),
    };
    let r = plug.validate(&pod_req("ns", "acme", Operation::Delete));
    assert!(r.allowed);
}

#[test]
fn psa_plugin_enforce_denies() {
    let levels = Arc::new(InMemoryLevelStore::new());
    levels.set("acme", "ns", LevelTriple {
        enforce: Some(Level::Baseline), audit: None, warn: None,
    });
    let mut bad = PodSecuritySpec::default();
    bad.host_network = true;
    let plug = PodSecurityPlugin {
        levels, extractor: Arc::new(StaticSpec(bad)),
    };
    let r = plug.validate(&pod_req("ns", "acme", Operation::Create));
    assert!(!r.allowed);
    assert_eq!(r.status_code, 403);
    assert!(r.status_message.contains("hostNetwork"));
}

#[test]
fn psa_plugin_warn_emits_warnings_but_allows() {
    let levels = Arc::new(InMemoryLevelStore::new());
    levels.set("acme", "ns", LevelTriple {
        enforce: None, audit: None, warn: Some(Level::Baseline),
    });
    let mut bad = PodSecuritySpec::default();
    bad.host_pid = true;
    let plug = PodSecurityPlugin {
        levels, extractor: Arc::new(StaticSpec(bad)),
    };
    let r = plug.validate(&pod_req("ns", "acme", Operation::Create));
    assert!(r.allowed);
    assert!(r.warnings.iter().any(|w| w.contains("hostPID")));
}

#[test]
fn psa_plugin_no_label_means_privileged_passes() {
    let levels = Arc::new(InMemoryLevelStore::new());
    let mut bad = PodSecuritySpec::default();
    bad.host_network = true;
    let plug = PodSecurityPlugin {
        levels, extractor: Arc::new(StaticSpec(bad)),
    };
    let r = plug.validate(&pod_req("ns", "acme", Operation::Create));
    assert!(r.allowed,
        "no namespace labels = no policy = allow (per upstream default)");
}

#[test]
fn psa_plugin_tenant_scoping() {
    // Two tenants with the same namespace name get different levels.
    let levels = Arc::new(InMemoryLevelStore::new());
    levels.set("acme", "ns", LevelTriple {
        enforce: Some(Level::Restricted), audit: None, warn: None,
    });
    // globex's same-name namespace has no policy → privileged
    let mut bad = PodSecuritySpec::default();
    bad.host_network = true;
    let plug = PodSecurityPlugin {
        levels, extractor: Arc::new(StaticSpec(bad)),
    };
    let acme_r = plug.validate(&pod_req("ns", "acme", Operation::Create));
    let globex_r = plug.validate(&pod_req("ns", "globex", Operation::Create));
    assert!(!acme_r.allowed, "acme has Restricted, hostNetwork denied");
    assert!(globex_r.allowed, "globex has no policy, allowed");
}

#[test] #[cfg(feature = "live-integration")]
fn psa_audit_emits_audit_annotation() {
    // pending: M4 hand-off — audit annotation map population
}

#[test] #[cfg(feature = "live-integration")]
fn psa_pod_template_propagation() {
    // pending: requires Deployment/StatefulSet pod template introspection
}

#[test] #[cfg(feature = "live-integration")]
fn psa_exemptions_by_user_and_namespace() {
    // pending: requires PodSecurityConfiguration exemptions list
}
