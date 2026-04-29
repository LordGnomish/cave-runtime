//! CiliumEnvoyConfig (CEC) reconciler.
//!
//! Mirrors `pkg/ciliumenvoyconfig/cec_resource_parser.go` and
//! `pkg/ciliumenvoyconfig/controller.go`. The CEC and CCEC CRDs let
//! users push raw envoy resources (Listener / Route / Cluster /
//! Endpoint) into the agent's xDS cache.
//!
//! We port the CRD shape and a parser that validates the resource
//! type-URL is one xDS knows about.

use crate::cilium::types::{Cite, TenantId};
use crate::cilium::xds::{is_known_type_url, type_url as t};
use serde::{Deserialize, Serialize};

/// Spec of a `CiliumEnvoyConfig` (namespaced) or `CiliumClusterwideEnvoyConfig`
/// (cluster-scoped) CRD instance.
///
/// We model the public field shape (services, resources, backendServices)
/// — the behavioural hooks are wired through the regular xDS server
/// (see `xds.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CecSpec {
    #[serde(default)]
    pub services: Vec<ServiceListener>,
    #[serde(default)]
    pub backend_services: Vec<BackendService>,
    #[serde(default)]
    pub resources: Vec<EnvoyResource>,
    #[serde(default)]
    pub node_selector: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServiceListener {
    pub name: String,
    pub namespace: String,
    /// Listener names this service redirects to (e.g. envoy listener names).
    #[serde(default)]
    pub listener: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BackendService {
    pub name: String,
    pub namespace: String,
    #[serde(default)]
    pub number: Vec<String>, // port names
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EnvoyResource {
    /// Envoy proto type URL (e.g. `type.googleapis.com/envoy.config.listener.v3.Listener`).
    #[serde(rename = "@type")]
    pub type_url: String,
    /// Resource body (envoy proto JSON).
    #[serde(flatten)]
    pub body: serde_json::Value,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CecError {
    #[error("unknown envoy resource type URL {0}")]
    UnknownTypeUrl(String),
    #[error("missing required field {0}")]
    MissingField(&'static str),
    #[error("tenant {tenant} cannot reconcile CEC owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// Validate that all `resources` reference one of the canonical xDS
/// type URLs. Mirrors the upstream parser's first sanity check.
pub fn validate_spec(spec: &CecSpec) -> Result<(), CecError> {
    for r in &spec.resources {
        if !is_known_type_url(&r.type_url) {
            return Err(CecError::UnknownTypeUrl(r.type_url.clone()));
        }
    }
    Ok(())
}

/// Count resources of each canonical kind. Useful for status reporting.
pub fn count_by_kind(spec: &CecSpec) -> (usize, usize, usize, usize, usize) {
    let mut listeners = 0;
    let mut routes = 0;
    let mut clusters = 0;
    let mut endpoints = 0;
    let mut secrets = 0;
    for r in &spec.resources {
        match r.type_url.as_str() {
            x if x == t::LISTENER => listeners += 1,
            x if x == t::ROUTE => routes += 1,
            x if x == t::CLUSTER => clusters += 1,
            x if x == t::ENDPOINT => endpoints += 1,
            x if x == t::SECRET => secrets += 1,
            _ => {}
        }
    }
    (listeners, routes, clusters, endpoints, secrets)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ciliumenvoyconfig/cec_resource_parser.go", "Parser");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn empty_spec_validates() {
        let (_c, _t) = cilium_test_ctx!("pkg/ciliumenvoyconfig/cec_resource_parser.go", "Spec.Empty", "tenant-cec-e");
        let s = CecSpec::default();
        assert!(validate_spec(&s).is_ok());
    }

    #[test]
    fn unknown_type_url_fails_validation() {
        let (_c, _t) = cilium_test_ctx!("pkg/ciliumenvoyconfig/cec_resource_parser.go", "Spec.Bad", "tenant-cec-b");
        let s = CecSpec {
            resources: vec![EnvoyResource { type_url: "weird".into(), body: serde_json::json!({}) }],
            ..Default::default()
        };
        let e = validate_spec(&s).unwrap_err();
        assert!(matches!(e, CecError::UnknownTypeUrl(_)));
    }

    #[test]
    fn known_listener_passes_validation() {
        let (_c, _t) = cilium_test_ctx!("pkg/ciliumenvoyconfig/cec_resource_parser.go", "Spec.Listener", "tenant-cec-lst");
        let s = CecSpec {
            resources: vec![EnvoyResource { type_url: t::LISTENER.into(), body: serde_json::json!({"name":"l1"}) }],
            ..Default::default()
        };
        assert!(validate_spec(&s).is_ok());
    }

    #[test]
    fn count_by_kind_buckets_correctly() {
        let (_c, _t) = cilium_test_ctx!("pkg/ciliumenvoyconfig/cec_resource_parser.go", "Counts", "tenant-cec-c");
        let s = CecSpec {
            resources: vec![
                EnvoyResource { type_url: t::LISTENER.into(), body: serde_json::json!({}) },
                EnvoyResource { type_url: t::LISTENER.into(), body: serde_json::json!({}) },
                EnvoyResource { type_url: t::ROUTE.into(), body: serde_json::json!({}) },
                EnvoyResource { type_url: t::CLUSTER.into(), body: serde_json::json!({}) },
            ],
            ..Default::default()
        };
        let (l, r, c, e, sec) = count_by_kind(&s);
        assert_eq!((l, r, c, e, sec), (2, 1, 1, 0, 0));
    }

    #[test]
    fn service_listener_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/ciliumenvoyconfig/cec_resource_parser.go", "ServiceListener", "tenant-cec-sl");
        let sl = ServiceListener { name: "s".into(), namespace: "ns".into(), listener: Some("l".into()) };
        let s = serde_json::to_string(&sl).unwrap();
        let back: ServiceListener = serde_json::from_str(&s).unwrap();
        assert_eq!(sl, back);
    }

    #[test]
    fn backend_service_supports_named_ports() {
        let (_c, _t) = cilium_test_ctx!("pkg/ciliumenvoyconfig/cec_resource_parser.go", "BackendService", "tenant-cec-bs");
        let bs = BackendService { name: "x".into(), namespace: "default".into(), number: vec!["http".into(), "https".into()] };
        assert_eq!(bs.number.len(), 2);
    }

    #[test]
    fn envoy_resource_uses_at_type_field_in_json() {
        let (_c, _t) = cilium_test_ctx!("pkg/ciliumenvoyconfig/cec_resource_parser.go", "Resource.AtType", "tenant-cec-at");
        let er = EnvoyResource { type_url: t::CLUSTER.into(), body: serde_json::json!({"name":"c1"}) };
        let s = serde_json::to_string(&er).unwrap();
        // Serde renames type_url to @type
        assert!(s.contains("\"@type\""));
    }

    #[test]
    fn cec_error_renders() {
        let (_c, _t) = cilium_test_ctx!("pkg/ciliumenvoyconfig/cec_resource_parser.go", "Errors", "tenant-cec-er");
        let e = CecError::MissingField("name");
        assert!(format!("{}", e).contains("name"));
    }

    #[test]
    fn spec_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/ciliumenvoyconfig/cec_resource_parser.go", "Spec.Serde", "tenant-cec-s");
        let s = CecSpec {
            services: vec![ServiceListener { name: "s".into(), namespace: "ns".into(), listener: None }],
            backend_services: vec![BackendService { name: "b".into(), namespace: "ns".into(), number: vec![] }],
            resources: vec![EnvoyResource { type_url: t::LISTENER.into(), body: serde_json::json!({"name":"l"}) }],
            node_selector: None,
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: CecSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
