// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory TracingPolicy store. Backed by `dashmap::DashMap`.
//!
//! Upstream: `pkg/policyfilter/policyfilter.go`, `pkg/sensors/manager.go`.

use crate::error::{ForensicsError, Result};
use crate::events::KernelEvent;
use crate::tracing_policy::TracingPolicy;
use dashmap::DashMap;
use std::sync::Arc;

/// Cluster-wide policy store. Each policy is keyed by `(namespace?, name)`
/// to support both `TracingPolicy` (cluster) and `TracingPolicyNamespaced`.
#[derive(Debug, Default)]
pub struct PolicyStore {
    inner: DashMap<String, TracingPolicy>,
}

fn policy_key(p: &TracingPolicy) -> String {
    match &p.metadata.namespace {
        Some(ns) => format!("{ns}/{}", p.metadata.name),
        None => format!("cluster/{}", p.metadata.name),
    }
}

impl PolicyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(&self, p: TracingPolicy) -> Result<()> {
        p.validate()?;
        let key = policy_key(&p);
        self.inner.insert(key, p);
        Ok(())
    }

    pub fn remove(&self, namespace: Option<&str>, name: &str) -> Result<()> {
        let key = match namespace {
            Some(ns) => format!("{ns}/{name}"),
            None => format!("cluster/{name}"),
        };
        self.inner
            .remove(&key)
            .ok_or_else(|| ForensicsError::InvalidPolicy(format!("policy not found: {key}")))?;
        Ok(())
    }

    pub fn list(&self) -> Vec<TracingPolicy> {
        self.inner.iter().map(|r| r.value().clone()).collect()
    }

    pub fn count(&self) -> usize {
        self.inner.len()
    }

    /// Return every policy whose pod selector matches the given pod-info.
    pub fn matching(
        &self,
        pod: &crate::selectors::PodInfo,
    ) -> Vec<TracingPolicy> {
        self.inner
            .iter()
            .filter(|r| {
                if let Some(ns) = &r.value().metadata.namespace {
                    if ns != &pod.namespace {
                        return false;
                    }
                }
                r.value()
                    .spec
                    .pod_selector
                    .as_ref()
                    .map(|s| s.matches(pod))
                    .unwrap_or(true)
            })
            .map(|r| r.value().clone())
            .collect()
    }
}

/// Convenience: register the store with the `cave-runtime` event router.
/// In tests this returns an `Arc<PolicyStore>` so observers can share it.
pub fn shared_store() -> Arc<PolicyStore> {
    Arc::new(PolicyStore::new())
}

/// Walk every installed policy + every kprobe selector and let the
/// caller see which `(policy_name, FilterGroup)` pairs matched a kernel
/// event. Used as the entry point from the observer loop in cave-runtime.
pub fn matching_groups<'a>(
    store: &'a PolicyStore,
    ev: &KernelEvent,
) -> Vec<(String, crate::filter::FilterGroup)> {
    let mut out = Vec::new();
    for r in store.inner.iter() {
        let p = r.value();
        for kp in &p.spec.kprobes {
            for g in &kp.selectors {
                if g.matches(ev).unwrap_or(false) {
                    out.push((p.metadata.name.clone(), g.clone()));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::process_exec::ProcessExecEvent;
    use crate::filter::{ActionKind, FilterOp, MatchAction, MatchBinary};
    use crate::process::{Credentials, Namespaces, Process};
    use crate::selectors::{ContainerInfo, PodInfo, PodSelector};
    use crate::tracing_policy::{KProbeSpec, PolicyKind, PolicyMeta, TracingPolicySpec};
    use chrono::{TimeZone, Utc};

    fn ts() -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(0, 0).unwrap()
    }

    fn minimal_policy(name: &str, ns: Option<&str>) -> TracingPolicy {
        TracingPolicy {
            api_version: "cilium.io/v1alpha1".into(),
            kind: if ns.is_some() {
                PolicyKind::TracingPolicyNamespaced
            } else {
                PolicyKind::TracingPolicy
            },
            metadata: PolicyMeta {
                name: name.into(),
                namespace: ns.map(String::from),
                ..Default::default()
            },
            spec: TracingPolicySpec {
                kprobes: vec![KProbeSpec {
                    call: "sys_open".into(),
                    syscall: true,
                    return_: false,
                    args: vec![],
                    selectors: vec![],
                }],
                ..Default::default()
            },
        }
    }

    #[test]
    fn test_install_cluster_policy() {
        let s = PolicyStore::new();
        s.install(minimal_policy("p1", None)).unwrap();
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn test_install_namespaced_policy() {
        let s = PolicyStore::new();
        s.install(minimal_policy("p1", Some("kube-system"))).unwrap();
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn test_remove_existing() {
        let s = PolicyStore::new();
        s.install(minimal_policy("p1", None)).unwrap();
        s.remove(None, "p1").unwrap();
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn test_remove_missing_errors() {
        let s = PolicyStore::new();
        assert!(s.remove(None, "nope").is_err());
    }

    #[test]
    fn test_install_rejects_invalid_policy() {
        let s = PolicyStore::new();
        let mut bad = minimal_policy("", None);
        bad.metadata.name = String::new();
        assert!(s.install(bad).is_err());
    }

    #[test]
    fn test_list_returns_all_policies() {
        let s = PolicyStore::new();
        s.install(minimal_policy("a", None)).unwrap();
        s.install(minimal_policy("b", Some("default"))).unwrap();
        s.install(minimal_policy("c", Some("kube-system"))).unwrap();
        assert_eq!(s.list().len(), 3);
    }

    #[test]
    fn test_matching_by_namespace() {
        let s = PolicyStore::new();
        s.install(minimal_policy("cluster-p", None)).unwrap();
        s.install(minimal_policy("ns-p", Some("default"))).unwrap();
        s.install(minimal_policy("kube-p", Some("kube-system"))).unwrap();
        let pod = PodInfo {
            name: "p".into(),
            namespace: "default".into(),
            labels: Default::default(),
            containers: vec![ContainerInfo {
                name: "c".into(),
                image: "x".into(),
            }],
        };
        let matched = s.matching(&pod);
        // cluster-p has no namespace -> matches; ns-p namespace == default -> matches;
        // kube-p namespace != default -> excluded.
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn test_matching_with_pod_selector_label_filter() {
        let s = PolicyStore::new();
        let mut p = minimal_policy("p1", None);
        let mut sel = PodSelector::default();
        sel.match_labels.insert("app".into(), "nginx".into());
        p.spec.pod_selector = Some(sel);
        s.install(p).unwrap();
        let mut pod = PodInfo {
            name: "x".into(),
            namespace: "default".into(),
            labels: Default::default(),
            containers: vec![],
        };
        assert!(s.matching(&pod).is_empty());
        pod.labels.insert("app".into(), "nginx".into());
        assert_eq!(s.matching(&pod).len(), 1);
    }

    #[test]
    fn test_matching_groups_returns_matched_pairs() {
        let s = PolicyStore::new();
        let mut p = minimal_policy("p1", None);
        let mut g = crate::filter::FilterGroup::default();
        g.match_binaries.push(MatchBinary {
            operator: FilterOp::Equal,
            values: vec!["/bin/sh".into()],
        });
        g.match_actions.push(MatchAction {
            action: ActionKind::Post,
            arg_error: None,
            arg_sig: None,
            arg_fd: None,
            arg_name: None,
            rate_limit: None,
        });
        p.spec.kprobes[0].selectors.push(g);
        s.install(p).unwrap();
        let ev = KernelEvent::ProcessExec(ProcessExecEvent {
            process: Process {
                exec_id: "x".into(),
                pid: 1,
                pid_in_ns: 1,
                binary: "/bin/sh".into(),
                arguments: String::new(),
                cwd: "/".into(),
                credentials: Credentials::default(),
                namespaces: Namespaces::default(),
                parent_exec_id: None,
                container_id: None,
                pod_name: None,
                pod_namespace: None,
                start_time: ts(),
                end_time: None,
            },
            ancestors: vec![],
            observed_at: ts(),
        });
        let matches = matching_groups(&s, &ev);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, "p1");
    }
}
