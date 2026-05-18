// SPDX-License-Identifier: AGPL-3.0-or-later
//! xDS server abstraction.
//!
//! Mirrors `pkg/xds/experimental/client/`. The agent runs an xDS server
//! that envoy connects to via gRPC over a unix socket; per-resource
//! versioning, ack/nack handling, and the resource type URLs follow the
//! envoy-aligned conventions.
//!
//! We port the version-and-ack book-keeping plus the canonical type URLs.

use crate::cilium::types::{Cite, TenantId};
use std::collections::BTreeMap;

/// Canonical xDS resource type URLs (v3). Each matches the upstream
/// envoy proto package name.
pub mod type_url {
    pub const LISTENER: &str = "type.googleapis.com/envoy.config.listener.v3.Listener";
    pub const ROUTE: &str = "type.googleapis.com/envoy.config.route.v3.RouteConfiguration";
    pub const CLUSTER: &str = "type.googleapis.com/envoy.config.cluster.v3.Cluster";
    pub const ENDPOINT: &str = "type.googleapis.com/envoy.config.endpoint.v3.ClusterLoadAssignment";
    pub const SECRET: &str = "type.googleapis.com/envoy.extensions.transport_sockets.tls.v3.Secret";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckState { Pending, Acked, Nacked }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceVersion {
    pub type_url: String,
    pub version: u64,
    pub state: AckState,
    pub last_error: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum XdsError {
    #[error("unknown type URL {0}")]
    UnknownTypeUrl(String),
    #[error("nack received for {type_url} version {version}: {detail}")]
    Nacked { type_url: String, version: u64, detail: String },
    #[error("tenant {tenant} cannot drive xds server")]
    TenantDenied { tenant: TenantId },
}

/// Per-tenant xDS server state.
#[derive(Debug)]
pub struct XdsServer {
    pub tenant: TenantId,
    versions: BTreeMap<String, ResourceVersion>,
}

impl XdsServer {
    pub fn new(tenant: TenantId) -> Self { Self { tenant, versions: BTreeMap::new() } }

    /// Push a new version for `type_url`. Returns the new version number.
    pub fn push(&mut self, type_url: &str) -> Result<u64, XdsError> {
        if !is_known_type_url(type_url) {
            return Err(XdsError::UnknownTypeUrl(type_url.to_string()));
        }
        let entry = self.versions.entry(type_url.to_string()).or_insert(ResourceVersion {
            type_url: type_url.to_string(), version: 0,
            state: AckState::Acked, last_error: None,
        });
        entry.version += 1;
        entry.state = AckState::Pending;
        entry.last_error = None;
        Ok(entry.version)
    }

    pub fn record_ack(&mut self, type_url: &str, version: u64) -> Result<(), XdsError> {
        let entry = self.versions.get_mut(type_url)
            .ok_or_else(|| XdsError::UnknownTypeUrl(type_url.to_string()))?;
        if entry.version == version {
            entry.state = AckState::Acked;
            entry.last_error = None;
        }
        Ok(())
    }

    pub fn record_nack(&mut self, type_url: &str, version: u64, detail: &str) -> Result<(), XdsError> {
        let entry = self.versions.get_mut(type_url)
            .ok_or_else(|| XdsError::UnknownTypeUrl(type_url.to_string()))?;
        if entry.version == version {
            entry.state = AckState::Nacked;
            entry.last_error = Some(detail.to_string());
        }
        Ok(())
    }

    pub fn version_of(&self, type_url: &str) -> Option<&ResourceVersion> {
        self.versions.get(type_url)
    }
}

pub fn is_known_type_url(t: &str) -> bool {
    matches!(t, _ if t == type_url::LISTENER
        || t == type_url::ROUTE
        || t == type_url::CLUSTER
        || t == type_url::ENDPOINT
        || t == type_url::SECRET)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/xds/experimental/client/client.go", "Client");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn type_urls_use_v3_envoy_proto_paths() {
        let (_c, _t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "TypeURL.V3", "tenant-xd-v3");
        assert!(type_url::LISTENER.starts_with("type.googleapis.com/envoy.config.listener.v3."));
        assert!(type_url::CLUSTER.contains("envoy.config.cluster.v3"));
        assert!(type_url::ROUTE.contains("envoy.config.route.v3"));
        assert!(type_url::ENDPOINT.contains("envoy.config.endpoint.v3"));
    }

    #[test]
    fn is_known_type_url_recognises_canonical() {
        let (_c, _t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "Known", "tenant-xd-k");
        assert!(is_known_type_url(type_url::LISTENER));
        assert!(is_known_type_url(type_url::ROUTE));
        assert!(is_known_type_url(type_url::CLUSTER));
        assert!(is_known_type_url(type_url::ENDPOINT));
        assert!(is_known_type_url(type_url::SECRET));
        assert!(!is_known_type_url("not-a-type-url"));
    }

    #[test]
    fn push_increments_version() {
        let (_c, t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "Push", "tenant-xd-p");
        let mut s = XdsServer::new(t);
        assert_eq!(s.push(type_url::LISTENER).unwrap(), 1);
        assert_eq!(s.push(type_url::LISTENER).unwrap(), 2);
        assert_eq!(s.push(type_url::LISTENER).unwrap(), 3);
    }

    #[test]
    fn push_unknown_type_url_errors() {
        let (_c, t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "Push.Unknown", "tenant-xd-pu");
        let mut s = XdsServer::new(t);
        let e = s.push("type.googleapis.com/envoy.weird.thing").unwrap_err();
        assert!(matches!(e, XdsError::UnknownTypeUrl(_)));
    }

    #[test]
    fn push_marks_state_pending() {
        let (_c, t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "Push.Pending", "tenant-xd-pp");
        let mut s = XdsServer::new(t);
        s.push(type_url::CLUSTER).unwrap();
        assert_eq!(s.version_of(type_url::CLUSTER).unwrap().state, AckState::Pending);
    }

    #[test]
    fn record_ack_for_current_version_marks_acked() {
        let (_c, t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "Ack", "tenant-xd-a");
        let mut s = XdsServer::new(t);
        let v = s.push(type_url::CLUSTER).unwrap();
        s.record_ack(type_url::CLUSTER, v).unwrap();
        assert_eq!(s.version_of(type_url::CLUSTER).unwrap().state, AckState::Acked);
    }

    #[test]
    fn record_ack_for_stale_version_is_noop() {
        let (_c, t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "Ack.Stale", "tenant-xd-as");
        let mut s = XdsServer::new(t);
        s.push(type_url::CLUSTER).unwrap();
        s.push(type_url::CLUSTER).unwrap();
        s.record_ack(type_url::CLUSTER, 1).unwrap();
        // Latest version is still Pending.
        assert_eq!(s.version_of(type_url::CLUSTER).unwrap().state, AckState::Pending);
    }

    #[test]
    fn record_nack_marks_nacked_with_detail() {
        let (_c, t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "Nack", "tenant-xd-n");
        let mut s = XdsServer::new(t);
        let v = s.push(type_url::ROUTE).unwrap();
        s.record_nack(type_url::ROUTE, v, "bad route").unwrap();
        let r = s.version_of(type_url::ROUTE).unwrap();
        assert_eq!(r.state, AckState::Nacked);
        assert_eq!(r.last_error.as_deref(), Some("bad route"));
    }

    #[test]
    fn unknown_type_url_in_ack_errors() {
        let (_c, t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "Ack.Unknown", "tenant-xd-au");
        let mut s = XdsServer::new(t);
        let e = s.record_ack("ghost", 1).unwrap_err();
        assert!(matches!(e, XdsError::UnknownTypeUrl(_)));
    }

    #[test]
    fn xds_error_renders() {
        let (_c, _t) = cilium_test_ctx!("pkg/xds/experimental/client/client.go", "Errors", "tenant-xd-er");
        let e = XdsError::Nacked { type_url: "x".into(), version: 1, detail: "boom".into() };
        assert!(format!("{}", e).contains("nack"));
    }
}
