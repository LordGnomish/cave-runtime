// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pod label resolver — compute identity-defining labels from a K8s
//! Pod + Namespace.
//!
//! Mirrors `pkg/labels/filter.go` and `pkg/k8s/labels.go::filterPodLabels`.
//! Not every label on a Pod participates in the Cilium identity — the
//! agent applies a configurable include/exclude list:
//!
//! * Reserved label sources are kept as-is (`reserved:`).
//! * Pod labels prefixed `k8s:` (the default source).
//! * Namespace labels prefixed `k8s:io.kubernetes.pod.namespace=…`.
//! * The configurable `--labels=…` filter accepts a regex; matching
//!   labels are *kept* (whitelist mode) or *dropped* (blacklist mode).

use crate::cilium::identity::LabelSet;
use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMode {
    /// Only labels matching the filter are kept.
    Whitelist,
    /// Labels matching the filter are dropped.
    Blacklist,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelFilter {
    pub mode: FilterMode,
    pub patterns: Vec<String>,
}

impl LabelFilter {
    pub fn matches(&self, label_key: &str) -> bool {
        self.patterns.iter().any(|p| pattern_match(p, label_key))
    }
    pub fn keep(&self, label_key: &str) -> bool {
        match self.mode {
            FilterMode::Whitelist => self.matches(label_key),
            FilterMode::Blacklist => !self.matches(label_key),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodMeta {
    pub name: String,
    pub namespace: String,
    pub labels: BTreeMap<String, String>,
    pub service_account: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceMeta {
    pub name: String,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolverError {
    #[error("invalid filter pattern `{0}`")]
    BadPattern(String),
    #[error("tenant {tenant} cannot mutate resolver owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct LabelResolver {
    pub tenant: TenantId,
    pub filter: LabelFilter,
    /// Always-include label keys regardless of filter (mirrors upstream
    /// `pkg/labels/filter.go::reservedLabels`).
    pub always_include_prefixes: Vec<String>,
}

impl LabelResolver {
    pub fn new(tenant: TenantId, filter: LabelFilter) -> Self {
        Self {
            tenant, filter,
            always_include_prefixes: vec![
                "io.kubernetes.pod.namespace".into(),
                "io.cilium".into(),
                "k8s-app".into(),
                "app".into(),
            ],
        }
    }

    /// Compute the identity-defining label set for a pod. Mirrors
    /// `pkg/k8s/labels.go::GetPodLabels`.
    pub fn resolve(&self, pod: &PodMeta, namespace: &NamespaceMeta) -> LabelSet {
        let mut out: BTreeMap<String, String> = BTreeMap::new();
        // Always include the namespace under the well-known key.
        out.insert("io.kubernetes.pod.namespace".into(), namespace.name.clone());
        if let Some(sa) = &pod.service_account {
            out.insert("io.cilium.k8s.policy.serviceaccount".into(), sa.clone());
        }
        for (k, v) in &pod.labels {
            if self.is_kept(k) {
                out.insert(k.clone(), v.clone());
            }
        }
        // Namespace labels propagate with a `io.kubernetes.namespace.` prefix.
        for (k, v) in &namespace.labels {
            let key = format!("io.kubernetes.pod.namespace.labels.{k}");
            if self.is_kept(&key) || self.is_kept(k) {
                out.insert(key, v.clone());
            }
        }
        LabelSet::from_iter(out)
    }

    fn is_kept(&self, key: &str) -> bool {
        if self.always_include_prefixes.iter().any(|p| key.starts_with(p)) {
            return true;
        }
        self.filter.keep(key)
    }
}

/// Glob-style pattern matching — `*` matches any single segment.
fn pattern_match(pattern: &str, key: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return key.starts_with(&format!("{prefix}."));
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return key.ends_with(&format!(".{suffix}"));
    }
    pattern == key
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/labels/filter.go", "Filter");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn pod(name: &str, ns: &str, labels: &[(&str, &str)], sa: Option<&str>) -> PodMeta {
        PodMeta {
            name: name.into(), namespace: ns.into(),
            labels: labels.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            service_account: sa.map(String::from),
        }
    }

    fn namespace(name: &str, labels: &[(&str, &str)]) -> NamespaceMeta {
        NamespaceMeta {
            name: name.into(),
            labels: labels.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
        }
    }

    fn whitelist(patterns: &[&str]) -> LabelFilter {
        LabelFilter {
            mode: FilterMode::Whitelist,
            patterns: patterns.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn blacklist(patterns: &[&str]) -> LabelFilter {
        LabelFilter {
            mode: FilterMode::Blacklist,
            patterns: patterns.iter().map(|s| (*s).into()).collect(),
        }
    }

    // ── Pattern matching ────────────────────────────────────────────────────

    #[test]
    fn pattern_matches_exact() {
        let (_c, _t) = cilium_test_ctx!("pkg/labels/filter.go", "Pattern.Exact", "tenant-lr-pe");
        assert!(pattern_match("app", "app"));
        assert!(!pattern_match("app", "other"));
    }

    #[test]
    fn pattern_matches_star_anything() {
        let (_c, _t) = cilium_test_ctx!("pkg/labels/filter.go", "Pattern.Star", "tenant-lr-ps");
        assert!(pattern_match("*", "anything"));
    }

    #[test]
    fn pattern_matches_prefix_dot_star() {
        let (_c, _t) = cilium_test_ctx!("pkg/labels/filter.go", "Pattern.Prefix", "tenant-lr-pp");
        assert!(pattern_match("io.cilium.*", "io.cilium.k8s.policy.serviceaccount"));
        assert!(!pattern_match("io.cilium.*", "io.other.label"));
    }

    #[test]
    fn pattern_matches_suffix_dot_star() {
        let (_c, _t) = cilium_test_ctx!("pkg/labels/filter.go", "Pattern.Suffix", "tenant-lr-pf");
        assert!(pattern_match("*.label", "io.cilium.label"));
        assert!(!pattern_match("*.label", "io.cilium.other"));
    }

    // ── LabelFilter ────────────────────────────────────────────────────────

    #[test]
    fn whitelist_keeps_only_matching_keys() {
        let (_c, _t) = cilium_test_ctx!("pkg/labels/filter.go", "Whitelist", "tenant-lr-wl");
        let f = whitelist(&["app", "tier"]);
        assert!(f.keep("app"));
        assert!(f.keep("tier"));
        assert!(!f.keep("env"));
    }

    #[test]
    fn blacklist_drops_matching_keys() {
        let (_c, _t) = cilium_test_ctx!("pkg/labels/filter.go", "Blacklist", "tenant-lr-bl");
        let f = blacklist(&["pod-template-hash", "controller-revision-hash"]);
        assert!(!f.keep("pod-template-hash"));
        assert!(!f.keep("controller-revision-hash"));
        assert!(f.keep("app"));
    }

    // ── Resolve ────────────────────────────────────────────────────────────

    #[test]
    fn resolve_includes_namespace_label() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.Namespace", "tenant-lr-n");
        let r = LabelResolver::new(tenant, whitelist(&["app"]));
        let pod = pod("p", "default", &[("app", "web")], None);
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(labels.pairs.iter().any(|(k, v)| k == "io.kubernetes.pod.namespace" && v == "default"));
    }

    #[test]
    fn resolve_includes_service_account() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.SA", "tenant-lr-sa");
        let r = LabelResolver::new(tenant, whitelist(&["app"]));
        let pod = pod("p", "default", &[], Some("api-sa"));
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(labels.pairs.iter().any(|(k, v)| k == "io.cilium.k8s.policy.serviceaccount" && v == "api-sa"));
    }

    #[test]
    fn resolve_filters_out_unwanted_labels_in_whitelist_mode() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.Whitelist", "tenant-lr-wl2");
        let r = LabelResolver::new(tenant, whitelist(&["app"]));
        let pod = pod("p", "default", &[
            ("app", "web"),
            ("pod-template-hash", "abc123"),
        ], None);
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(labels.pairs.iter().any(|(k, _)| k == "app"));
        assert!(!labels.pairs.iter().any(|(k, _)| k == "pod-template-hash"));
    }

    #[test]
    fn resolve_keeps_always_include_prefix_even_outside_filter() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.AlwaysInclude", "tenant-lr-ai");
        let r = LabelResolver::new(tenant, whitelist(&["env"]));
        let pod = pod("p", "default", &[("io.cilium.policy.scope", "cluster")], None);
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(labels.pairs.iter().any(|(k, _)| k == "io.cilium.policy.scope"));
    }

    #[test]
    fn resolve_blacklist_drops_specified_labels() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.Blacklist", "tenant-lr-bl2");
        let r = LabelResolver::new(tenant, blacklist(&["pod-template-hash"]));
        let pod = pod("p", "default", &[
            ("env", "prod"),
            ("pod-template-hash", "abc123"),
        ], None);
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(labels.pairs.iter().any(|(k, _)| k == "env"));
        assert!(!labels.pairs.iter().any(|(k, _)| k == "pod-template-hash"));
    }

    #[test]
    fn resolve_namespace_labels_promoted_to_pod_namespace_labels_prefix() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.NamespaceLabels", "tenant-lr-nl");
        let r = LabelResolver::new(tenant, whitelist(&["env", "io.kubernetes.pod.namespace.labels.env"]));
        let pod = pod("p", "default", &[("env", "prod")], None);
        let ns = namespace("default", &[("env", "prod")]);
        let labels = r.resolve(&pod, &ns);
        assert!(labels.pairs.iter().any(|(k, _)| k == "io.kubernetes.pod.namespace.labels.env"));
    }

    #[test]
    fn resolve_no_service_account_omits_sa_label() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.NoSA", "tenant-lr-nsa");
        let r = LabelResolver::new(tenant, whitelist(&["app"]));
        let pod = pod("p", "default", &[], None);
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(!labels.pairs.iter().any(|(k, _)| k == "io.cilium.k8s.policy.serviceaccount"));
    }

    #[test]
    fn resolve_app_label_always_included() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.AppDefault", "tenant-lr-ad");
        // Filter only allows "env" — but "app" is in always_include_prefixes.
        let r = LabelResolver::new(tenant, whitelist(&["env"]));
        let pod = pod("p", "default", &[("app", "web")], None);
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(labels.pairs.iter().any(|(k, _)| k == "app"));
    }

    #[test]
    fn resolve_empty_pod_and_namespace_returns_minimal_set() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.Minimal", "tenant-lr-min");
        let r = LabelResolver::new(tenant, whitelist(&[]));
        let pod = pod("p", "ns", &[], None);
        let ns = namespace("ns", &[]);
        let labels = r.resolve(&pod, &ns);
        // Just the namespace marker.
        assert_eq!(labels.pairs.len(), 1);
    }

    #[test]
    fn resolve_with_glob_filter_keeps_matching_labels() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.Glob", "tenant-lr-glob");
        let r = LabelResolver::new(tenant, whitelist(&["acme.*"]));
        let pod = pod("p", "default", &[
            ("acme.io/team", "platform"),
            ("acme.io/env", "prod"),
            ("k8s.io/version", "v1"),
        ], None);
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(labels.pairs.iter().any(|(k, _)| k == "acme.io/team"));
        assert!(labels.pairs.iter().any(|(k, _)| k == "acme.io/env"));
    }

    // ── Idempotency ────────────────────────────────────────────────────────

    #[test]
    fn resolve_same_input_produces_same_output() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.Idempotent", "tenant-lr-idem");
        let r = LabelResolver::new(tenant, whitelist(&["app"]));
        let pod = pod("p", "default", &[("app", "web")], Some("api-sa"));
        let ns = namespace("default", &[]);
        let a = r.resolve(&pod, &ns);
        let b = r.resolve(&pod, &ns);
        assert_eq!(a, b);
    }

    #[test]
    fn resolve_label_order_does_not_affect_output() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/labels.go", "Resolve.OrderInsensitive", "tenant-lr-oi");
        let r = LabelResolver::new(tenant, whitelist(&["app", "env"]));
        let mut a_labels = BTreeMap::new();
        a_labels.insert("app".into(), "web".into());
        a_labels.insert("env".into(), "prod".into());
        let mut b_labels = BTreeMap::new();
        b_labels.insert("env".into(), "prod".into());
        b_labels.insert("app".into(), "web".into());
        let pod_a = PodMeta { name: "p".into(), namespace: "ns".into(), labels: a_labels, service_account: None };
        let pod_b = PodMeta { name: "p".into(), namespace: "ns".into(), labels: b_labels, service_account: None };
        let ns = namespace("ns", &[]);
        assert_eq!(r.resolve(&pod_a, &ns), r.resolve(&pod_b, &ns));
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn label_filter_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/labels/filter.go", "Filter.Serde", "tenant-lr-fserde");
        let f = whitelist(&["app", "env"]);
        let s = serde_json::to_string(&f).unwrap();
        let back: LabelFilter = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn pod_meta_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/labels.go", "PodMeta.Serde", "tenant-lr-pserde");
        let p = pod("p", "ns", &[("app", "web")], Some("sa"));
        let s = serde_json::to_string(&p).unwrap();
        let back: PodMeta = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn namespace_meta_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/labels.go", "NamespaceMeta.Serde", "tenant-lr-nserde");
        let n = namespace("ns", &[("env", "prod")]);
        let s = serde_json::to_string(&n).unwrap();
        let back: NamespaceMeta = serde_json::from_str(&s).unwrap();
        assert_eq!(back, n);
    }

    // ── Edge: blacklist with empty patterns keeps everything ────────────────

    #[test]
    fn empty_blacklist_keeps_all_labels() {
        let (_c, tenant) = cilium_test_ctx!("pkg/labels/filter.go", "Blacklist.Empty", "tenant-lr-be");
        let r = LabelResolver::new(tenant, blacklist(&[]));
        let pod = pod("p", "default", &[("app", "web"), ("custom", "v")], None);
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(labels.pairs.iter().any(|(k, _)| k == "custom"));
    }

    #[test]
    fn empty_whitelist_drops_non_always_include_labels() {
        let (_c, tenant) = cilium_test_ctx!("pkg/labels/filter.go", "Whitelist.Empty", "tenant-lr-we");
        let r = LabelResolver::new(tenant, whitelist(&[]));
        let pod = pod("p", "default", &[("custom", "v")], None);
        let ns = namespace("default", &[]);
        let labels = r.resolve(&pod, &ns);
        assert!(!labels.pairs.iter().any(|(k, _)| k == "custom"));
    }
}
