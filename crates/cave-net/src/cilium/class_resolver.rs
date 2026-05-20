// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! IngressClass / GatewayClass parameter resolver.
//!
//! Mirrors `pkg/ingress/class_resolver.go` and the gateway-API
//! `GatewayClass.Spec.ParametersRef` resolution in
//! `pkg/gateway-api/translation/gatewayclass.go`. Both K8s
//! `IngressClass` and `GatewayClass` allow a `parameters` reference
//! pointing at a CRD that customises behaviour (per-class LB IP pool,
//! TLS profile, etc.). The agent resolves the reference + caches the
//! parameter object.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParametersRef {
    pub api_group: String,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassSpec {
    pub name: String,
    pub controller: String, // e.g. "cilium.io/ingress-controller"
    pub parameters: Option<ParametersRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassParameters {
    pub config: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolverError {
    #[error("class `{0}` not registered")]
    ClassNotFound(String),
    #[error("parameters ref `{0}/{1}` not registered")]
    ParametersNotFound(String, String),
    #[error("class `{0}` controller `{1}` not owned by Cilium")]
    NotOwned(String, String),
    #[error("tenant {tenant} cannot mutate resolver owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct ClassResolver {
    pub tenant: TenantId,
    pub controller_name: String, // controller string Cilium claims
    classes: BTreeMap<String, ClassSpec>,
    /// Parameter objects keyed by `<kind>/<name>`.
    parameters: BTreeMap<String, ClassParameters>,
}

impl ClassResolver {
    pub fn new(tenant: TenantId, controller_name: impl Into<String>) -> Self {
        Self {
            tenant,
            controller_name: controller_name.into(),
            classes: BTreeMap::new(),
            parameters: BTreeMap::new(),
        }
    }

    pub fn upsert_class(&mut self, spec: ClassSpec) {
        self.classes.insert(spec.name.clone(), spec);
    }

    pub fn remove_class(&mut self, name: &str) -> Result<(), ResolverError> {
        self.classes
            .remove(name)
            .ok_or_else(|| ResolverError::ClassNotFound(name.to_string()))?;
        Ok(())
    }

    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    pub fn upsert_parameters(&mut self, kind: &str, name: &str, params: ClassParameters) {
        self.parameters.insert(format!("{kind}/{name}"), params);
    }

    pub fn remove_parameters(&mut self, kind: &str, name: &str) -> bool {
        self.parameters.remove(&format!("{kind}/{name}")).is_some()
    }

    /// Resolve the parameters object referenced by `class_name`.
    /// Returns `Ok(None)` if the class has no `parameters_ref`.
    pub fn resolve(&self, class_name: &str) -> Result<Option<&ClassParameters>, ResolverError> {
        let class = self
            .classes
            .get(class_name)
            .ok_or_else(|| ResolverError::ClassNotFound(class_name.to_string()))?;
        if class.controller != self.controller_name {
            return Err(ResolverError::NotOwned(
                class_name.to_string(),
                class.controller.clone(),
            ));
        }
        let pref = match &class.parameters {
            None => return Ok(None),
            Some(p) => p,
        };
        let key = format!("{}/{}", pref.kind, pref.name);
        let params = self.parameters.get(&key).ok_or_else(|| {
            ResolverError::ParametersNotFound(pref.kind.clone(), pref.name.clone())
        })?;
        Ok(Some(params))
    }

    /// All Cilium-owned classes in the registry.
    pub fn owned_classes(&self) -> Vec<&ClassSpec> {
        self.classes
            .values()
            .filter(|c| c.controller == self.controller_name)
            .collect()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ingress/class_resolver.go", "Resolver");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn resolver(tenant: TenantId) -> ClassResolver {
        ClassResolver::new(tenant, "cilium.io/ingress-controller")
    }

    fn class(name: &str, controller: &str, params: Option<ParametersRef>) -> ClassSpec {
        ClassSpec {
            name: name.into(),
            controller: controller.into(),
            parameters: params,
        }
    }

    fn params(pairs: &[(&str, &str)]) -> ClassParameters {
        ClassParameters {
            config: pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        }
    }

    fn ref_to(kind: &str, name: &str) -> ParametersRef {
        ParametersRef {
            api_group: "cilium.io".into(),
            kind: kind.into(),
            name: name.into(),
            namespace: None,
        }
    }

    // ── Class lifecycle ────────────────────────────────────────────────────

    #[test]
    fn upsert_class_records_it() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/ingress/class_resolver.go", "Upsert", "tenant-cr-u");
        let mut r = resolver(tenant);
        r.upsert_class(class("cilium", "cilium.io/ingress-controller", None));
        assert_eq!(r.class_count(), 1);
    }

    #[test]
    fn remove_class_drops_it() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/ingress/class_resolver.go", "Remove", "tenant-cr-rm");
        let mut r = resolver(tenant);
        r.upsert_class(class("cilium", "cilium.io/ingress-controller", None));
        r.remove_class("cilium").unwrap();
        assert_eq!(r.class_count(), 0);
    }

    #[test]
    fn remove_unknown_class_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "Remove.NotFound",
            "tenant-cr-rmnf"
        );
        let mut r = resolver(tenant);
        let err = r.remove_class("ghost").unwrap_err();
        assert!(matches!(err, ResolverError::ClassNotFound(_)));
    }

    // ── Parameters lifecycle ───────────────────────────────────────────────

    #[test]
    fn upsert_parameters_keyed_by_kind_name() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "UpsertParameters",
            "tenant-cr-up"
        );
        let mut r = resolver(tenant);
        r.upsert_parameters(
            "CiliumIngressClassParams",
            "default",
            params(&[("scope", "cluster")]),
        );
        // Verify by resolution path.
        r.upsert_class(class(
            "cilium",
            "cilium.io/ingress-controller",
            Some(ref_to("CiliumIngressClassParams", "default")),
        ));
        let resolved = r.resolve("cilium").unwrap().unwrap();
        assert_eq!(
            resolved.config.get("scope").map(|s| s.as_str()),
            Some("cluster")
        );
    }

    #[test]
    fn remove_parameters_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "RemoveParameters",
            "tenant-cr-rmp"
        );
        let mut r = resolver(tenant);
        r.upsert_parameters("CiliumIngressClassParams", "default", params(&[]));
        assert!(r.remove_parameters("CiliumIngressClassParams", "default"));
        assert!(!r.remove_parameters("CiliumIngressClassParams", "default"));
    }

    // ── Resolve ────────────────────────────────────────────────────────────

    #[test]
    fn resolve_unknown_class_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "Resolve.ClassNotFound",
            "tenant-cr-rcnf"
        );
        let r = resolver(tenant);
        let err = r.resolve("ghost").unwrap_err();
        assert!(matches!(err, ResolverError::ClassNotFound(_)));
    }

    #[test]
    fn resolve_class_owned_by_other_controller_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "Resolve.NotOwned",
            "tenant-cr-rno"
        );
        let mut r = resolver(tenant);
        r.upsert_class(class("nginx", "nginx.io/ingress-controller", None));
        let err = r.resolve("nginx").unwrap_err();
        assert!(matches!(err, ResolverError::NotOwned(_, _)));
    }

    #[test]
    fn resolve_class_without_parameters_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "Resolve.NoParams",
            "tenant-cr-rnp"
        );
        let mut r = resolver(tenant);
        r.upsert_class(class("cilium", "cilium.io/ingress-controller", None));
        let resolved = r.resolve("cilium").unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_with_dangling_parameters_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "Resolve.Dangling",
            "tenant-cr-rd"
        );
        let mut r = resolver(tenant);
        r.upsert_class(class(
            "cilium",
            "cilium.io/ingress-controller",
            Some(ref_to("CiliumIngressClassParams", "missing")),
        ));
        let err = r.resolve("cilium").unwrap_err();
        assert!(matches!(err, ResolverError::ParametersNotFound(_, _)));
    }

    #[test]
    fn resolve_returns_full_parameter_set() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "Resolve.Full",
            "tenant-cr-rf"
        );
        let mut r = resolver(tenant);
        r.upsert_parameters(
            "Params",
            "default",
            params(&[("lb_pool", "10.0.0.0/24"), ("tls_profile", "modern")]),
        );
        r.upsert_class(class(
            "cilium",
            "cilium.io/ingress-controller",
            Some(ref_to("Params", "default")),
        ));
        let resolved = r.resolve("cilium").unwrap().unwrap();
        assert_eq!(resolved.config.len(), 2);
    }

    // ── Owned classes ──────────────────────────────────────────────────────

    #[test]
    fn owned_classes_filters_by_controller_name() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/ingress/class_resolver.go", "Owned", "tenant-cr-o");
        let mut r = resolver(tenant);
        r.upsert_class(class("cilium", "cilium.io/ingress-controller", None));
        r.upsert_class(class("nginx", "nginx.io/ingress-controller", None));
        r.upsert_class(class("cilium-extra", "cilium.io/ingress-controller", None));
        let owned = r.owned_classes();
        assert_eq!(owned.len(), 2);
    }

    #[test]
    fn owned_classes_empty_when_none_match() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "Owned.None",
            "tenant-cr-on"
        );
        let mut r = resolver(tenant);
        r.upsert_class(class("nginx", "nginx.io/ingress-controller", None));
        assert!(r.owned_classes().is_empty());
    }

    // ── Class registry ─────────────────────────────────────────────────────

    #[test]
    fn class_count_tracks_upserts() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/ingress/class_resolver.go", "Count", "tenant-cr-c");
        let mut r = resolver(tenant);
        for i in 0..5u8 {
            r.upsert_class(class(
                &format!("c-{i}"),
                "cilium.io/ingress-controller",
                None,
            ));
        }
        assert_eq!(r.class_count(), 5);
    }

    #[test]
    fn upsert_class_replaces_existing() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "Upsert.Replace",
            "tenant-cr-rep"
        );
        let mut r = resolver(tenant);
        r.upsert_class(class("cilium", "cilium.io/ingress-controller", None));
        r.upsert_class(class(
            "cilium",
            "cilium.io/ingress-controller",
            Some(ref_to("Params", "v2")),
        ));
        assert!(r.classes.get("cilium").unwrap().parameters.is_some());
        assert_eq!(r.class_count(), 1);
    }

    // ── Gateway-API equivalence ────────────────────────────────────────────

    #[test]
    fn resolver_works_for_gateway_class_controller_name() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/gateway-api/translation/gatewayclass.go",
            "Resolve",
            "tenant-cr-gw"
        );
        let mut r = ClassResolver::new(tenant, "io.cilium/gateway-controller");
        r.upsert_class(class("cilium-gw", "io.cilium/gateway-controller", None));
        assert_eq!(r.owned_classes().len(), 1);
    }

    // ── Cross-controller ──────────────────────────────────────────────────

    #[test]
    fn resolve_handles_mixed_owners_correctly() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/ingress/class_resolver.go", "Mixed", "tenant-cr-mx");
        let mut r = resolver(tenant);
        r.upsert_class(class("cilium", "cilium.io/ingress-controller", None));
        r.upsert_class(class("nginx", "nginx.io/ingress-controller", None));
        // Cilium resolves; nginx errors NotOwned.
        assert!(r.resolve("cilium").is_ok());
        assert!(matches!(
            r.resolve("nginx").unwrap_err(),
            ResolverError::NotOwned(_, _)
        ));
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn class_spec_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "ClassSpec.Serde",
            "tenant-cr-cserde"
        );
        let c = class(
            "cilium",
            "cilium.io/ingress-controller",
            Some(ref_to("Params", "v1")),
        );
        let s = serde_json::to_string(&c).unwrap();
        let back: ClassSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn class_parameters_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "Parameters.Serde",
            "tenant-cr-pserde"
        );
        let p = params(&[("scope", "cluster")]);
        let s = serde_json::to_string(&p).unwrap();
        let back: ClassParameters = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn parameters_ref_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/ingress/class_resolver.go",
            "ParametersRef.Serde",
            "tenant-cr-prserde"
        );
        let r = ref_to("Params", "v1");
        let s = serde_json::to_string(&r).unwrap();
        let back: ParametersRef = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }
}
