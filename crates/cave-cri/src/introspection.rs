// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Introspection service — containerd v1 `IntrospectionService` parity.
//!
//! Operator tooling (containerd's `ctr plugins ls`, Kubernetes node
//! debugging, the cave-portal admin view) consumes the introspection
//! API to discover which plugins are loaded, their version, exported
//! types, and the server-level identity (UUID, PID, deprecations).
//!
//! Upstream parity:
//! - containerd: `core/introspection/` + `api/services/introspection/v1/`.
//! - method `Plugins(filters) → PluginsResponse`  → here as
//!   [`IntrospectionService::plugins`].
//! - method `Server() → ServerResponse`           → here as
//!   [`IntrospectionService::server`].
//!
//! HTTP shape:
//! - `GET /v1/introspection/plugins`  →  list every registered plugin.
//! - `GET /v1/introspection/server`   →  bind UUID + PID + deprecations.
//!
//! Per containerd, introspection is read-only and side-effect-free.
//! The cave-cri implementation snapshots the registry under a `RwLock`;
//! callers see a coherent view but writes (plugin register) and reads
//! never block each other for long.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// A single plugin row in `PluginsResponse`.
///
/// `kind` matches containerd's `Plugin.Type` enum
/// (`image`, `snapshot`, `runtime`, `sandbox`, etc.).
/// `name` is the plugin id under that kind. The pair `(kind, name)`
/// is the registry primary key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub kind: PluginKind,
    pub name: String,
    pub version: String,
    pub capabilities: Vec<String>,
    pub exports: BTreeMap<String, String>,
}

impl PluginInfo {
    pub fn new(
        kind: PluginKind,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            name: name.into(),
            version: version.into(),
            capabilities: Vec::new(),
            exports: BTreeMap::new(),
        }
    }

    pub fn with_capability(mut self, cap: impl Into<String>) -> Self {
        self.capabilities.push(cap.into());
        self
    }

    pub fn with_export(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.exports.insert(key.into(), val.into());
        self
    }
}

/// The four containerd plugin kinds cave-cri implements.
///
/// containerd has more (`metadata`, `gc`, `streaming`, …) but the
/// snapshot above is the subset the cave-cri parity surface covers.
/// Unknown kinds round-trip via the catch-all `Other(...)` variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Image,
    Snapshot,
    Runtime,
    Sandbox,
    Other(String),
}

impl PluginKind {
    pub fn as_str(&self) -> &str {
        match self {
            PluginKind::Image => "image",
            PluginKind::Snapshot => "snapshot",
            PluginKind::Runtime => "runtime",
            PluginKind::Sandbox => "sandbox",
            PluginKind::Other(s) => s.as_str(),
        }
    }
}

/// Deprecation warning surfaced by `ServerResponse`.
///
/// containerd encodes deprecations as opaque ids (e.g.
/// `io.containerd.deprecation/runtime-v1-shim`) so administrators can
/// `grep` configuration without scraping freeform text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deprecation {
    pub id: String,
    pub message: String,
}

/// Server-level introspection (containerd's `ServerResponse`).
///
/// `uuid` is generated per cave-cri process start and is stable for
/// the lifetime of the process; the kubelet uses it as a tie-breaker
/// when reconciling sandboxes across restarts. `pid` lets the
/// operator confirm they're hitting the right daemon. `deprecations`
/// surfaces config that should be migrated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerResponse {
    pub uuid: String,
    pub pid: u32,
    pub deprecations: Vec<Deprecation>,
    pub started_at: DateTime<Utc>,
}

/// `IntrospectionService.Plugins` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginsResponse {
    pub plugins: Vec<PluginInfo>,
}

/// The cave-cri introspection service. Cheap to clone (Arc'd).
#[derive(Clone)]
pub struct IntrospectionService {
    inner: Arc<Inner>,
}

struct Inner {
    plugins: RwLock<Vec<PluginInfo>>,
    server_uuid: String,
    pid: u32,
    started_at: DateTime<Utc>,
    deprecations: RwLock<Vec<Deprecation>>,
}

impl IntrospectionService {
    /// New empty service. The bind UUID is a fresh v4. `pid` is the
    /// process pid as reported by `std::process::id`.
    pub fn new() -> Self {
        Self::with_uuid(Uuid::new_v4().to_string())
    }

    /// New service with a caller-supplied bind UUID (used in tests
    /// for determinism).
    pub fn with_uuid(uuid: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Inner {
                plugins: RwLock::new(Vec::new()),
                server_uuid: uuid.into(),
                pid: std::process::id(),
                started_at: Utc::now(),
                deprecations: RwLock::new(Vec::new()),
            }),
        }
    }

    /// Default set of plugins matching the cave-cri build —
    /// matches containerd's "out-of-the-box" install.
    pub fn with_defaults() -> Self {
        let svc = Self::new();
        svc.register(
            PluginInfo::new(PluginKind::Image, "registry", env!("CARGO_PKG_VERSION"))
                .with_capability("pull")
                .with_capability("manifest_list")
                .with_export("transport", "https"),
        );
        svc.register(
            PluginInfo::new(PluginKind::Snapshot, "overlayfs", env!("CARGO_PKG_VERSION"))
                .with_capability("prepare")
                .with_capability("mount")
                .with_export("driver", "overlay"),
        );
        svc.register(
            PluginInfo::new(PluginKind::Runtime, "runc", env!("CARGO_PKG_VERSION"))
                .with_capability("oci_spec_v1")
                .with_capability("cgroup_v2"),
        );
        svc.register(
            PluginInfo::new(PluginKind::Sandbox, "podsandbox", env!("CARGO_PKG_VERSION"))
                .with_capability("network_attach")
                .with_capability("portforward"),
        );
        svc
    }

    /// Register a plugin. If `(kind, name)` already exists, the entry
    /// is replaced (idempotent registration).
    pub fn register(&self, plugin: PluginInfo) {
        let mut g = self.inner.plugins.write().expect("plugins write");
        if let Some(slot) = g
            .iter_mut()
            .find(|p| p.kind == plugin.kind && p.name == plugin.name)
        {
            *slot = plugin;
        } else {
            g.push(plugin);
        }
    }

    /// Add a deprecation warning surfaced by `server()`.
    pub fn add_deprecation(&self, id: impl Into<String>, message: impl Into<String>) {
        self.inner
            .deprecations
            .write()
            .expect("deprecations write")
            .push(Deprecation {
                id: id.into(),
                message: message.into(),
            });
    }

    /// Snapshot of every registered plugin, optionally filtered to a
    /// single kind. Mirrors containerd's
    /// `IntrospectionService.Plugins(filters)`.
    pub fn plugins(&self, kind_filter: Option<&PluginKind>) -> PluginsResponse {
        let g = self.inner.plugins.read().expect("plugins read");
        let plugins: Vec<PluginInfo> = match kind_filter {
            Some(k) => g.iter().filter(|p| &p.kind == k).cloned().collect(),
            None => g.clone(),
        };
        PluginsResponse { plugins }
    }

    /// Server-identity snapshot. Mirrors `IntrospectionService.Server()`.
    pub fn server(&self) -> ServerResponse {
        ServerResponse {
            uuid: self.inner.server_uuid.clone(),
            pid: self.inner.pid,
            deprecations: self
                .inner
                .deprecations
                .read()
                .expect("deprecations read")
                .clone(),
            started_at: self.inner.started_at,
        }
    }

    /// Number of registered plugins (any kind).
    pub fn plugin_count(&self) -> usize {
        self.inner.plugins.read().expect("plugins read").len()
    }
}

impl Default for IntrospectionService {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// HTTP route helper. The caller mounts these on whichever axum
/// `Router` they're using (cave-cri's own router or a higher-level
/// cave-runtime mux). Kept as a free function so this module can be
/// pulled into other crates (cave-portal, cavectl) without dragging
/// the entire cave-cri router along.
pub fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec {
            method: "GET",
            path: "/v1/introspection/plugins",
            handler: "introspection_plugins",
        },
        RouteSpec {
            method: "GET",
            path: "/v1/introspection/server",
            handler: "introspection_server",
        },
    ]
}

/// Declarative route description used for documentation + tests
/// (avoids a heavy axum dep in code that only wants to assert the
/// surface shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteSpec {
    pub method: &'static str,
    pub path: &'static str,
    pub handler: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_info_builder_collects_capabilities_and_exports() {
        let p = PluginInfo::new(PluginKind::Image, "registry", "1.2.3")
            .with_capability("pull")
            .with_capability("push")
            .with_export("transport", "https")
            .with_export("auth", "bearer");
        assert_eq!(p.name, "registry");
        assert_eq!(p.version, "1.2.3");
        assert_eq!(p.capabilities, vec!["pull", "push"]);
        assert_eq!(p.exports.get("transport").map(String::as_str), Some("https"));
        assert_eq!(p.exports.get("auth").map(String::as_str), Some("bearer"));
    }

    #[test]
    fn plugin_kind_as_str_round_trip() {
        assert_eq!(PluginKind::Image.as_str(), "image");
        assert_eq!(PluginKind::Snapshot.as_str(), "snapshot");
        assert_eq!(PluginKind::Runtime.as_str(), "runtime");
        assert_eq!(PluginKind::Sandbox.as_str(), "sandbox");
        assert_eq!(PluginKind::Other("metadata".into()).as_str(), "metadata");
    }

    #[test]
    fn defaults_register_four_canonical_plugins() {
        let svc = IntrospectionService::with_defaults();
        let all = svc.plugins(None);
        assert_eq!(all.plugins.len(), 4);

        let kinds: Vec<&str> = all.plugins.iter().map(|p| p.kind.as_str()).collect();
        assert!(kinds.contains(&"image"));
        assert!(kinds.contains(&"snapshot"));
        assert!(kinds.contains(&"runtime"));
        assert!(kinds.contains(&"sandbox"));
    }

    #[test]
    fn register_is_idempotent_on_kind_plus_name() {
        let svc = IntrospectionService::new();
        svc.register(PluginInfo::new(PluginKind::Image, "registry", "1.0.0"));
        svc.register(PluginInfo::new(PluginKind::Image, "registry", "1.0.1"));
        assert_eq!(svc.plugin_count(), 1);

        let r = svc.plugins(None);
        assert_eq!(r.plugins[0].version, "1.0.1");
    }

    #[test]
    fn plugins_filter_by_kind_returns_subset() {
        let svc = IntrospectionService::with_defaults();
        let only_runtime = svc.plugins(Some(&PluginKind::Runtime));
        assert_eq!(only_runtime.plugins.len(), 1);
        assert_eq!(only_runtime.plugins[0].kind, PluginKind::Runtime);
    }

    #[test]
    fn plugins_filter_no_match_returns_empty() {
        let svc = IntrospectionService::with_defaults();
        let none = svc.plugins(Some(&PluginKind::Other("metadata".into())));
        assert!(none.plugins.is_empty());
    }

    #[test]
    fn server_returns_pinned_uuid_and_real_pid() {
        let svc = IntrospectionService::with_uuid("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        let s = svc.server();
        assert_eq!(s.uuid, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        assert_eq!(s.pid, std::process::id());
        assert!(s.deprecations.is_empty());
    }

    #[test]
    fn server_surfaces_added_deprecations() {
        let svc = IntrospectionService::with_uuid("test-uuid");
        svc.add_deprecation(
            "io.cave.deprecation/cgroup-v1",
            "cgroup v1 is unsupported as of 2026 — migrate to v2",
        );
        svc.add_deprecation(
            "io.cave.deprecation/legacy-runc-shim",
            "containerd-shim-runc-v1 has been removed",
        );
        let s = svc.server();
        assert_eq!(s.deprecations.len(), 2);
        assert_eq!(s.deprecations[0].id, "io.cave.deprecation/cgroup-v1");
        assert!(s.deprecations[1].message.contains("removed"));
    }

    #[test]
    fn server_uuid_is_unique_across_default_instances() {
        let a = IntrospectionService::new();
        let b = IntrospectionService::new();
        assert_ne!(a.server().uuid, b.server().uuid);
    }

    #[test]
    fn plugins_response_serializes_with_serde_json() {
        let svc = IntrospectionService::with_defaults();
        let r = svc.plugins(None);
        let json = serde_json::to_string(&r).expect("serialize");
        assert!(json.contains("\"image\""));
        assert!(json.contains("\"runc\""));
        let round: PluginsResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round.plugins.len(), 4);
    }

    #[test]
    fn server_response_serializes_with_serde_json() {
        let svc = IntrospectionService::with_uuid("ser-test");
        svc.add_deprecation("a", "b");
        let s = svc.server();
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(json.contains("\"ser-test\""));
        assert!(json.contains("\"a\""));
        let round: ServerResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round.uuid, "ser-test");
        assert_eq!(round.deprecations.len(), 1);
    }

    #[test]
    fn route_specs_expose_both_endpoints() {
        let specs = route_specs();
        assert_eq!(specs.len(), 2);
        let paths: Vec<&str> = specs.iter().map(|s| s.path).collect();
        assert!(paths.contains(&"/v1/introspection/plugins"));
        assert!(paths.contains(&"/v1/introspection/server"));
        for s in &specs {
            assert_eq!(s.method, "GET", "introspection is read-only");
        }
    }
}
