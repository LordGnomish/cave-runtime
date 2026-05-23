// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Admission-chain coordinator.
//!
//! Mirrors `pkg/admission` of upstream Kubernetes. A `Chain` is a list
//! of plugins; each `evaluate()` returns `Allow`, `Deny(msg)`, or
//! `Mutate(patch)`. Plugins are visited in registration order; mutation
//! plugins run before validation plugins, with cave-k8s enforcing the
//! K8s-canonical relative order between built-in plugins.

use crate::error::Error;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Create,
    Update,
    Delete,
    Connect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub operation: Operation,
    pub namespace: String,
    pub kind: String,
    pub name: String,
    pub user: String,
    /// JSON patch to be applied to the resource before storage.
    pub object: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
    Mutate(serde_json::Value),
}

pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn evaluate(&self, req: &mut Request) -> Decision;
    fn is_mutating(&self) -> bool;
}

pub struct Chain {
    plugins: Vec<Box<dyn Plugin>>,
}

impl Default for Chain {
    fn default() -> Self {
        Self::new()
    }
}

impl Chain {
    pub fn new() -> Self {
        Self { plugins: Vec::new() }
    }

    pub fn add(mut self, p: Box<dyn Plugin>) -> Self {
        self.plugins.push(p);
        self
    }

    /// Canonical K8s built-in admission ordering — see
    /// `pkg/kubeapiserver/options/plugins.go`.
    pub fn k8s_canonical_order() -> &'static [&'static str] {
        &[
            "NamespaceLifecycle",
            "LimitRanger",
            "ServiceAccount",
            "DefaultStorageClass",
            "DefaultTolerationSeconds",
            "PodSecurity",
            "PodTopologySpread",
            "MutatingAdmissionPolicy",
            "MutatingAdmissionWebhook",
            "ResourceQuota",
            "ValidatingAdmissionPolicy",
            "ValidatingAdmissionWebhook",
        ]
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Run the chain. Mutating plugins first (in registration order), then
    /// validators. Returns `Ok(())` if all admit, `Err(AdmissionRejected)`
    /// at the first deny.
    pub fn admit(&self, req: &mut Request) -> Result<(), Error> {
        for p in self.plugins.iter().filter(|p| p.is_mutating()) {
            match p.evaluate(req) {
                Decision::Allow => {}
                Decision::Deny(msg) => {
                    return Err(Error::AdmissionRejected(format!("{}: {}", p.name(), msg)));
                }
                Decision::Mutate(patch) => {
                    req.object = merge_patch(req.object.clone(), patch);
                }
            }
        }
        for p in self.plugins.iter().filter(|p| !p.is_mutating()) {
            match p.evaluate(req) {
                Decision::Allow => {}
                Decision::Deny(msg) => {
                    return Err(Error::AdmissionRejected(format!("{}: {}", p.name(), msg)));
                }
                Decision::Mutate(_) => {
                    return Err(Error::AdmissionRejected(format!(
                        "{}: validating plugin returned a Mutate decision",
                        p.name()
                    )));
                }
            }
        }
        Ok(())
    }
}

fn merge_patch(mut base: serde_json::Value, patch: serde_json::Value) -> serde_json::Value {
    match (base.as_object_mut(), patch.as_object()) {
        (Some(obj), Some(p)) => {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
            base
        }
        _ => patch,
    }
}

// ─── Built-in plugins ───────────────────────────────────────────────────────

/// NamespaceLifecycle — denies writes to a namespace in terminating
/// phase + blocks deletion of `kube-system` / `kube-public` / `default`.
pub struct NamespaceLifecycle {
    pub terminating: Vec<String>,
    pub protected: Vec<String>,
}

impl Default for NamespaceLifecycle {
    fn default() -> Self {
        Self {
            terminating: Vec::new(),
            protected: vec!["kube-system".into(), "kube-public".into(), "default".into()],
        }
    }
}

impl Plugin for NamespaceLifecycle {
    fn name(&self) -> &'static str {
        "NamespaceLifecycle"
    }
    fn is_mutating(&self) -> bool {
        false
    }
    fn evaluate(&self, req: &mut Request) -> Decision {
        if req.operation == Operation::Delete
            && req.kind == "Namespace"
            && self.protected.iter().any(|n| n == &req.name)
        {
            return Decision::Deny(format!("namespace {} is protected", req.name));
        }
        if self.terminating.iter().any(|n| n == &req.namespace) {
            return Decision::Deny(format!(
                "namespace {} is terminating",
                req.namespace
            ));
        }
        Decision::Allow
    }
}

/// ServiceAccount — sets `.spec.serviceAccountName = "default"` for pods
/// missing one. Mutating.
pub struct ServiceAccountDefaulter;
impl Plugin for ServiceAccountDefaulter {
    fn name(&self) -> &'static str {
        "ServiceAccount"
    }
    fn is_mutating(&self) -> bool {
        true
    }
    fn evaluate(&self, req: &mut Request) -> Decision {
        if req.kind != "Pod" || req.operation != Operation::Create {
            return Decision::Allow;
        }
        let spec = req.object.get("spec").and_then(|v| v.as_object());
        let name = spec.and_then(|m| m.get("serviceAccountName")).and_then(|v| v.as_str());
        if name.is_some() {
            return Decision::Allow;
        }
        Decision::Mutate(serde_json::json!({
            "spec": {"serviceAccountName": "default"}
        }))
    }
}

/// LimitRanger — enforces a minimum CPU request on Pod containers.
pub struct LimitRanger {
    pub min_cpu_request_millis: u32,
}

impl Plugin for LimitRanger {
    fn name(&self) -> &'static str {
        "LimitRanger"
    }
    fn is_mutating(&self) -> bool {
        false
    }
    fn evaluate(&self, req: &mut Request) -> Decision {
        if req.kind != "Pod" {
            return Decision::Allow;
        }
        let cs = req
            .object
            .get("spec")
            .and_then(|s| s.get("containers"))
            .and_then(|v| v.as_array());
        let Some(cs) = cs else { return Decision::Allow };
        for c in cs {
            let cpu = c
                .get("resources")
                .and_then(|r| r.get("requests"))
                .and_then(|r| r.get("cpu"))
                .and_then(|c| c.as_str())
                .unwrap_or("0");
            let millis = parse_cpu_millis(cpu);
            if millis < self.min_cpu_request_millis {
                return Decision::Deny(format!(
                    "container CPU request {}m below namespace minimum {}m",
                    millis, self.min_cpu_request_millis
                ));
            }
        }
        Decision::Allow
    }
}

fn parse_cpu_millis(s: &str) -> u32 {
    if let Some(num) = s.strip_suffix('m') {
        num.parse().unwrap_or(0)
    } else if let Ok(whole) = s.parse::<f64>() {
        (whole * 1000.0) as u32
    } else {
        0
    }
}

/// PodSecurity — restricted profile.  Denies privileged containers.
pub struct PodSecurityRestricted;
impl Plugin for PodSecurityRestricted {
    fn name(&self) -> &'static str {
        "PodSecurity"
    }
    fn is_mutating(&self) -> bool {
        false
    }
    fn evaluate(&self, req: &mut Request) -> Decision {
        if req.kind != "Pod" {
            return Decision::Allow;
        }
        let cs = req
            .object
            .get("spec")
            .and_then(|s| s.get("containers"))
            .and_then(|v| v.as_array());
        let Some(cs) = cs else { return Decision::Allow };
        for c in cs {
            let priv_flag = c
                .get("securityContext")
                .and_then(|s| s.get("privileged"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if priv_flag {
                return Decision::Deny(
                    "restricted profile forbids privileged containers".into(),
                );
            }
        }
        Decision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pod_req(spec: serde_json::Value) -> Request {
        Request {
            operation: Operation::Create,
            namespace: "default".into(),
            kind: "Pod".into(),
            name: "p1".into(),
            user: "alice".into(),
            object: json!({"spec": spec}),
        }
    }

    #[test]
    fn empty_chain_admits() {
        let c = Chain::new();
        let mut r = pod_req(json!({}));
        c.admit(&mut r).unwrap();
    }

    #[test]
    fn canonical_order_is_twelve() {
        assert_eq!(Chain::k8s_canonical_order().len(), 12);
    }

    #[test]
    fn service_account_defaulter_mutates() {
        let c = Chain::new().add(Box::new(ServiceAccountDefaulter));
        let mut r = pod_req(json!({"containers": []}));
        c.admit(&mut r).unwrap();
        assert_eq!(
            r.object
                .get("spec")
                .and_then(|s| s.get("serviceAccountName"))
                .and_then(|v| v.as_str()),
            Some("default")
        );
    }

    #[test]
    fn limit_ranger_rejects_low_cpu() {
        let c = Chain::new().add(Box::new(LimitRanger {
            min_cpu_request_millis: 100,
        }));
        let mut r = pod_req(json!({
            "containers": [{"resources": {"requests": {"cpu": "10m"}}}]
        }));
        let e = c.admit(&mut r).unwrap_err();
        assert!(matches!(e, Error::AdmissionRejected(_)));
    }

    #[test]
    fn pod_security_rejects_privileged() {
        let c = Chain::new().add(Box::new(PodSecurityRestricted));
        let mut r = pod_req(json!({
            "containers": [{"securityContext": {"privileged": true}}]
        }));
        assert!(matches!(c.admit(&mut r), Err(Error::AdmissionRejected(_))));
    }

    #[test]
    fn pod_security_admits_unprivileged() {
        let c = Chain::new().add(Box::new(PodSecurityRestricted));
        let mut r = pod_req(json!({
            "containers": [{"securityContext": {"privileged": false}}]
        }));
        c.admit(&mut r).unwrap();
    }

    #[test]
    fn namespace_lifecycle_denies_terminating() {
        let c = Chain::new().add(Box::new(NamespaceLifecycle {
            terminating: vec!["dead".into()],
            ..Default::default()
        }));
        let mut r = pod_req(json!({"containers": []}));
        r.namespace = "dead".into();
        assert!(matches!(c.admit(&mut r), Err(Error::AdmissionRejected(_))));
    }

    #[test]
    fn namespace_lifecycle_protects_default_namespaces() {
        let p = NamespaceLifecycle::default();
        let mut r = Request {
            operation: Operation::Delete,
            namespace: "default".into(),
            kind: "Namespace".into(),
            name: "kube-system".into(),
            user: "root".into(),
            object: json!({}),
        };
        assert_eq!(
            p.evaluate(&mut r),
            Decision::Deny("namespace kube-system is protected".into())
        );
    }

    #[test]
    fn parse_cpu_millis_handles_whole_and_milli() {
        assert_eq!(parse_cpu_millis("100m"), 100);
        assert_eq!(parse_cpu_millis("1"), 1000);
        assert_eq!(parse_cpu_millis("0.5"), 500);
        assert_eq!(parse_cpu_millis("garbage"), 0);
    }

    #[test]
    fn merge_patch_merges_top_level_keys() {
        let base = json!({"a": 1, "b": 2});
        let patch = json!({"b": 20, "c": 3});
        let merged = merge_patch(base, patch);
        assert_eq!(merged, json!({"a": 1, "b": 20, "c": 3}));
    }

    #[test]
    fn validator_returning_mutate_is_an_error() {
        struct BadValidator;
        impl Plugin for BadValidator {
            fn name(&self) -> &'static str {
                "BadValidator"
            }
            fn is_mutating(&self) -> bool {
                false
            }
            fn evaluate(&self, _: &mut Request) -> Decision {
                Decision::Mutate(json!({}))
            }
        }
        let c = Chain::new().add(Box::new(BadValidator));
        let mut r = pod_req(json!({}));
        assert!(matches!(c.admit(&mut r), Err(Error::AdmissionRejected(_))));
    }
}
