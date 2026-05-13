//! WasmPlugin CRD — filter-chain extension surface.
//!
//! Mirrors Istio's `extensions.istio.io/v1alpha1` WasmPlugin: an
//! operator-supplied WebAssembly module slotted into the envoy filter
//! chain at AUTHN / AUTHZ / STATS phases. cave-mesh does not run an
//! embedded wasm VM — that's out of scope for the userspace mesh
//! port — but it owns the CRD lifecycle, manifest validation, target
//! selection, and the envoy filter-chain insertion record that gets
//! handed to the xDS proxy.
//!
//! Scope cut: no actual `wasmtime`/`wasmer` runtime. The plugin's
//! URL/sha256 are recorded and the synthesised xDS filter
//! configuration is emitted; whether an envoy bound to that xDS
//! actually loads the wasm bytecode is the envoy's responsibility.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, thiserror::Error)]
pub enum WasmPluginError {
    #[error("wasm plugin {0} not found")]
    NotFound(String),
    #[error("wasm plugin {0} already exists")]
    AlreadyExists(String),
    #[error("invalid spec: {0}")]
    InvalidSpec(String),
}

/// Where in the filter chain to insert the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginPhase {
    /// Authentication (before authn filters).
    Authn,
    /// Authorization (between authn and authz native filters).
    Authz,
    /// Stats sink (after policy decisions).
    Stats,
    /// Default ordering — appended to the chain.
    Unspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginVerificationKey {
    Sha256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmPluginSpec {
    /// HTTPS or OCI URL of the .wasm module.
    pub url: String,
    /// Hex SHA-256 of the wasm bytecode for integrity. Required when
    /// `url` is HTTPS (not enforced for OCI references with their own
    /// digest in the tag).
    pub sha256: Option<String>,
    /// Phase to insert the plugin at.
    pub phase: PluginPhase,
    /// Smaller priority sorts earlier within the same phase.
    pub priority: i32,
    /// Workload selector — matches workloads whose pod labels are a
    /// superset of this map.
    pub selector: HashMap<String, String>,
    /// Optional plugin configuration JSON passed verbatim to the
    /// envoy filter `configuration` field.
    pub plugin_config: Option<serde_json::Value>,
    /// Plugin image-pull / sidecar-config flag.
    pub image_pull_policy: String,
}

impl WasmPluginSpec {
    pub fn validate(&self) -> Result<(), WasmPluginError> {
        if self.url.is_empty() {
            return Err(WasmPluginError::InvalidSpec("url must not be empty".into()));
        }
        if !(self.url.starts_with("https://") || self.url.starts_with("oci://")) {
            return Err(WasmPluginError::InvalidSpec(
                "url must be https:// or oci://".into(),
            ));
        }
        if self.url.starts_with("https://") && self.sha256.is_none() {
            return Err(WasmPluginError::InvalidSpec(
                "sha256 is required for https:// URLs".into(),
            ));
        }
        if let Some(h) = &self.sha256 {
            if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(WasmPluginError::InvalidSpec(
                    "sha256 must be a 64-char lowercase hex string".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WasmPluginPhase {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmPluginStatus {
    pub phase: WasmPluginPhase,
    pub observed_generation: u64,
    pub last_updated: DateTime<Utc>,
    pub message: String,
    /// Workloads (namespaced names) that matched the selector on
    /// the most recent reconcile.
    pub matched_workloads: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmPlugin {
    pub name: String,
    pub namespace: String,
    pub spec: WasmPluginSpec,
    pub status: WasmPluginStatus,
    pub generation: u64,
}

/// One filter to insert in the envoy chain. Caller (xDS proxy)
/// translates these into actual `envoy.filters.http.wasm`
/// configurations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterChainEntry {
    pub plugin_name: String,
    pub plugin_namespace: String,
    pub url: String,
    pub sha256: Option<String>,
    pub phase: PluginPhase,
    pub priority: i32,
    pub plugin_config_json: Option<String>,
}

pub struct WasmPluginManager {
    plugins: Arc<RwLock<HashMap<String, WasmPlugin>>>,
}

impl WasmPluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn key(namespace: &str, name: &str) -> String {
        format!("{namespace}/{name}")
    }

    pub fn create(
        &self,
        namespace: impl Into<String>,
        name: impl Into<String>,
        spec: WasmPluginSpec,
    ) -> Result<WasmPlugin, WasmPluginError> {
        spec.validate()?;
        let namespace = namespace.into();
        let name = name.into();
        let k = Self::key(&namespace, &name);
        let mut plugins = self.plugins.write().unwrap();
        if plugins.contains_key(&k) {
            return Err(WasmPluginError::AlreadyExists(k));
        }
        let plugin = WasmPlugin {
            name,
            namespace,
            spec,
            status: WasmPluginStatus {
                phase: WasmPluginPhase::Pending,
                observed_generation: 0,
                last_updated: Utc::now(),
                message: "created".into(),
                matched_workloads: Vec::new(),
            },
            generation: 1,
        };
        plugins.insert(k, plugin.clone());
        Ok(plugin)
    }

    pub fn get(&self, namespace: &str, name: &str) -> Result<WasmPlugin, WasmPluginError> {
        self.plugins
            .read()
            .unwrap()
            .get(&Self::key(namespace, name))
            .cloned()
            .ok_or_else(|| WasmPluginError::NotFound(Self::key(namespace, name)))
    }

    pub fn list(&self) -> Vec<WasmPlugin> {
        self.plugins.read().unwrap().values().cloned().collect()
    }

    pub fn list_namespace(&self, namespace: &str) -> Vec<WasmPlugin> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.namespace == namespace)
            .cloned()
            .collect()
    }

    pub fn update_spec(
        &self,
        namespace: &str,
        name: &str,
        spec: WasmPluginSpec,
    ) -> Result<WasmPlugin, WasmPluginError> {
        spec.validate()?;
        let k = Self::key(namespace, name);
        let mut plugins = self.plugins.write().unwrap();
        let p = plugins
            .get_mut(&k)
            .ok_or_else(|| WasmPluginError::NotFound(k.clone()))?;
        p.spec = spec;
        p.generation += 1;
        Ok(p.clone())
    }

    pub fn delete(&self, namespace: &str, name: &str) -> Result<(), WasmPluginError> {
        let k = Self::key(namespace, name);
        self.plugins
            .write()
            .unwrap()
            .remove(&k)
            .map(|_| ())
            .ok_or_else(|| WasmPluginError::NotFound(k))
    }

    /// Reconcile against a workload-label catalog and refresh status.
    /// `workloads` is `(namespaced_name, labels)` pairs.
    pub fn reconcile(
        &self,
        namespace: &str,
        name: &str,
        workloads: &[(String, HashMap<String, String>)],
    ) -> Result<WasmPlugin, WasmPluginError> {
        let k = Self::key(namespace, name);
        let mut plugins = self.plugins.write().unwrap();
        let p = plugins
            .get_mut(&k)
            .ok_or_else(|| WasmPluginError::NotFound(k))?;
        let mut matched: Vec<String> = workloads
            .iter()
            .filter(|(_, labels)| selector_matches(&p.spec.selector, labels))
            .map(|(n, _)| n.clone())
            .collect();
        matched.sort();
        p.status.matched_workloads = matched;
        p.status.observed_generation = p.generation;
        p.status.last_updated = Utc::now();
        p.status.phase = WasmPluginPhase::Ready;
        p.status.message = format!(
            "matched {} workload(s)",
            p.status.matched_workloads.len()
        );
        Ok(p.clone())
    }

    /// Project the active filter chain across all plugins that match
    /// a given workload's labels. Ordered by (phase, priority, name)
    /// so envoy sees a stable order.
    pub fn project_chain(
        &self,
        workload_labels: &HashMap<String, String>,
    ) -> Vec<FilterChainEntry> {
        let plugins = self.plugins.read().unwrap();
        let mut entries: Vec<FilterChainEntry> = plugins
            .values()
            .filter(|p| selector_matches(&p.spec.selector, workload_labels))
            .map(|p| FilterChainEntry {
                plugin_name: p.name.clone(),
                plugin_namespace: p.namespace.clone(),
                url: p.spec.url.clone(),
                sha256: p.spec.sha256.clone(),
                phase: p.spec.phase,
                priority: p.spec.priority,
                plugin_config_json: p
                    .spec
                    .plugin_config
                    .as_ref()
                    .map(|v| v.to_string()),
            })
            .collect();
        entries.sort_by(|a, b| {
            (a.phase as u8)
                .cmp(&(b.phase as u8))
                .then(a.priority.cmp(&b.priority))
                .then(a.plugin_name.cmp(&b.plugin_name))
        });
        entries
    }
}

impl Default for WasmPluginManager {
    fn default() -> Self {
        Self::new()
    }
}

fn selector_matches(
    selector: &HashMap<String, String>,
    labels: &HashMap<String, String>,
) -> bool {
    if selector.is_empty() {
        return true; // empty selector means "all workloads"
    }
    selector.iter().all(|(k, v)| labels.get(k) == Some(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(url: &str, sha: Option<&str>) -> WasmPluginSpec {
        WasmPluginSpec {
            url: url.into(),
            sha256: sha.map(|s| s.into()),
            phase: PluginPhase::Authn,
            priority: 10,
            selector: HashMap::new(),
            plugin_config: None,
            image_pull_policy: "IfNotPresent".into(),
        }
    }

    fn sha() -> &'static str {
        // 64 hex chars
        "0011223344556677889900aabbccddeeff00112233445566778899aabbccddee"
    }

    #[test]
    fn create_stores_plugin() {
        let m = WasmPluginManager::new();
        let p = m.create("ns", "p1", spec("https://w.example/m.wasm", Some(sha()))).unwrap();
        assert_eq!(p.namespace, "ns");
        assert_eq!(p.status.phase, WasmPluginPhase::Pending);
    }

    #[test]
    fn create_duplicate_refused() {
        let m = WasmPluginManager::new();
        m.create("ns", "p", spec("https://x", Some(sha()))).unwrap();
        let err = m.create("ns", "p", spec("https://x", Some(sha()))).unwrap_err();
        assert!(matches!(err, WasmPluginError::AlreadyExists(_)));
    }

    #[test]
    fn https_without_sha_rejected() {
        let m = WasmPluginManager::new();
        let err = m.create("ns", "p", spec("https://x", None)).unwrap_err();
        assert!(matches!(err, WasmPluginError::InvalidSpec(_)));
    }

    #[test]
    fn oci_url_does_not_require_sha() {
        let m = WasmPluginManager::new();
        m.create("ns", "p", spec("oci://repo/img:v1", None)).unwrap();
    }

    #[test]
    fn bad_url_scheme_rejected() {
        let m = WasmPluginManager::new();
        let err = m.create("ns", "p", spec("http://x", None)).unwrap_err();
        assert!(matches!(err, WasmPluginError::InvalidSpec(_)));
    }

    #[test]
    fn invalid_sha_format_rejected() {
        let mut s = spec("https://x", Some("zz"));
        assert!(matches!(s.validate(), Err(WasmPluginError::InvalidSpec(_))));
        s.sha256 = Some("a".repeat(63));
        assert!(matches!(s.validate(), Err(WasmPluginError::InvalidSpec(_))));
    }

    #[test]
    fn reconcile_marks_ready_and_records_matches() {
        let m = WasmPluginManager::new();
        let mut s = spec("oci://r:1", None);
        s.selector.insert("app".into(), "echo".into());
        m.create("ns", "p", s).unwrap();
        let workloads = vec![
            ("ns/echo-1".to_string(), [("app", "echo")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
            ("ns/other-1".to_string(), [("app", "other")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
        ];
        let p = m.reconcile("ns", "p", &workloads).unwrap();
        assert_eq!(p.status.phase, WasmPluginPhase::Ready);
        assert_eq!(p.status.matched_workloads, vec!["ns/echo-1".to_string()]);
    }

    #[test]
    fn empty_selector_matches_everything() {
        let m = WasmPluginManager::new();
        m.create("ns", "p", spec("oci://r:1", None)).unwrap();
        let labels: HashMap<String, String> = [("app", "anything")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let chain = m.project_chain(&labels);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn project_chain_orders_by_phase_then_priority() {
        let m = WasmPluginManager::new();
        let mut a = spec("oci://r:1", None); a.phase = PluginPhase::Authz; a.priority = 5;
        let mut b = spec("oci://r:1", None); b.phase = PluginPhase::Authn; b.priority = 10;
        let mut c = spec("oci://r:1", None); c.phase = PluginPhase::Authn; c.priority = 1;
        m.create("ns", "a", a).unwrap();
        m.create("ns", "b", b).unwrap();
        m.create("ns", "c", c).unwrap();
        let chain = m.project_chain(&HashMap::new());
        assert_eq!(chain.len(), 3);
        // Authn first, then Authz; within Authn priority 1 before 10.
        assert_eq!(chain[0].plugin_name, "c");
        assert_eq!(chain[1].plugin_name, "b");
        assert_eq!(chain[2].plugin_name, "a");
    }

    #[test]
    fn update_spec_bumps_generation() {
        let m = WasmPluginManager::new();
        m.create("ns", "p", spec("oci://r:1", None)).unwrap();
        let mut s = spec("oci://r:2", None);
        s.priority = 99;
        let p = m.update_spec("ns", "p", s).unwrap();
        assert_eq!(p.generation, 2);
        assert_eq!(p.spec.priority, 99);
    }

    #[test]
    fn delete_removes_plugin() {
        let m = WasmPluginManager::new();
        m.create("ns", "p", spec("oci://r:1", None)).unwrap();
        m.delete("ns", "p").unwrap();
        assert!(matches!(m.get("ns", "p").unwrap_err(), WasmPluginError::NotFound(_)));
    }

    #[test]
    fn list_namespace_filters() {
        let m = WasmPluginManager::new();
        m.create("ns-a", "p", spec("oci://r:1", None)).unwrap();
        m.create("ns-b", "p", spec("oci://r:1", None)).unwrap();
        assert_eq!(m.list_namespace("ns-a").len(), 1);
        assert_eq!(m.list().len(), 2);
    }

    #[test]
    fn project_chain_excludes_non_matching_selectors() {
        let m = WasmPluginManager::new();
        let mut s = spec("oci://r:1", None);
        s.selector.insert("app".into(), "echo".into());
        m.create("ns", "p", s).unwrap();
        let labels: HashMap<String, String> = [("app", "other")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert!(m.project_chain(&labels).is_empty());
    }
}
