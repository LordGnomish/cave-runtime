// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Backend datasource plugin SDK.
//!
//! upstream: grafana/grafana — pkg/plugins (backend datasource plugins, gRPC)
//!
//! Upstream plugins are out-of-process binaries that speak gRPC against
//! the Grafana plugin host. The MVP cave-dashboard build statically
//! links datasources via the [`crate::datasource::Datasource`] trait;
//! this module adds the plugin-discovery + plugin-manifest shape so
//! third-party Rust crates can register their own back-ends through
//! the same surface (without forcing a gRPC dependency on the runtime).
//!
//! Three pieces:
//!   * `PluginManifest` — TOML-shaped descriptor with id/type/version/exec.
//!   * `PluginRegistry` — in-proc registry mapping ids to backend factories.
//!   * `BackendPlugin` — trait every loaded plugin must implement.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginType {
    Datasource,
    Panel,
    App,
}

impl PluginType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "datasource" => Some(PluginType::Datasource),
            "panel" => Some(PluginType::Panel),
            "app" => Some(PluginType::App),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub id: String,
    pub kind: PluginType,
    pub name: String,
    pub version: String,
    pub author: String,
    /// Optional executable path (left None when the plugin is in-proc).
    pub executable: Option<String>,
    /// Static signature applied to the manifest body; we don't enforce
    /// it cryptographically here, but the host MAY require it.
    pub signature: Option<String>,
    /// Free-form key=value extra metadata (e.g. `category=tsdb`).
    pub meta: HashMap<String, String>,
}

impl PluginManifest {
    pub fn parse_toml(text: &str) -> Result<Self, String> {
        let mut id = String::new();
        let mut kind: Option<PluginType> = None;
        let mut name = String::new();
        let mut version = String::new();
        let mut author = String::new();
        let mut executable: Option<String> = None;
        let mut signature: Option<String> = None;
        let mut meta: HashMap<String, String> = HashMap::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let (k, v) = match line.split_once('=') {
                Some((k, v)) => (k.trim(), v.trim().trim_matches('"')),
                None => continue,
            };
            match k {
                "id" => id = v.to_string(),
                "type" => {
                    kind = PluginType::from_str(v);
                }
                "name" => name = v.to_string(),
                "version" => version = v.to_string(),
                "author" => author = v.to_string(),
                "executable" => executable = Some(v.to_string()),
                "signature" => signature = Some(v.to_string()),
                other => {
                    meta.insert(other.to_string(), v.to_string());
                }
            }
        }
        if id.is_empty() {
            return Err("plugin manifest missing id".into());
        }
        let kind = kind.ok_or("plugin manifest missing type (datasource|panel|app)")?;
        if version.is_empty() {
            return Err("plugin manifest missing version".into());
        }
        Ok(PluginManifest {
            id,
            kind,
            name,
            version,
            author,
            executable,
            signature,
            meta,
        })
    }
}

/// Lightweight backend plugin interface. Implementors return query
/// results keyed by the original RefId.
pub trait BackendPlugin: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn supports_health_check(&self) -> bool {
        true
    }
    fn health_check(&self) -> HealthCheck;
    fn query(&self, request: &PluginQueryRequest) -> Vec<PluginQueryResponse>;
}

#[derive(Debug, Clone)]
pub struct HealthCheck {
    pub status: HealthStatus,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct PluginQueryRequest {
    pub ref_id: String,
    pub from_unix_ms: i64,
    pub to_unix_ms: i64,
    pub expr: String,
    pub max_data_points: u32,
    pub interval_ms: u32,
}

#[derive(Debug, Clone)]
pub struct PluginQueryResponse {
    pub ref_id: String,
    pub frames: Vec<DataFrame>,
}

#[derive(Default, Debug, Clone)]
pub struct DataFrame {
    pub name: String,
    pub fields: Vec<DataField>,
}

#[derive(Debug, Clone)]
pub struct DataField {
    pub name: String,
    pub values: Vec<f64>,
    pub labels: HashMap<String, String>,
}

/// In-proc plugin registry.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: HashMap<String, Box<dyn BackendPlugin>>,
    manifests: HashMap<String, PluginManifest>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        manifest: PluginManifest,
        plugin: Box<dyn BackendPlugin>,
    ) -> Result<(), String> {
        if manifest.id != plugin.id() {
            return Err(format!(
                "manifest id ({}) does not match plugin id ({})",
                manifest.id,
                plugin.id()
            ));
        }
        if self.plugins.contains_key(&manifest.id) {
            return Err(format!("plugin id `{}` already registered", manifest.id));
        }
        self.manifests.insert(manifest.id.clone(), manifest.clone());
        self.plugins.insert(manifest.id, plugin);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&dyn BackendPlugin> {
        self.plugins.get(id).map(|p| p.as_ref())
    }

    pub fn manifest(&self, id: &str) -> Option<&PluginManifest> {
        self.manifests.get(id)
    }

    pub fn ids_of(&self, kind: PluginType) -> Vec<String> {
        let mut out: Vec<String> = self
            .manifests
            .values()
            .filter(|m| m.kind == kind)
            .map(|m| m.id.clone())
            .collect();
        out.sort();
        out
    }

    pub fn count(&self) -> usize {
        self.plugins.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoPlugin;
    impl BackendPlugin for EchoPlugin {
        fn id(&self) -> &str {
            "test-echo"
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn health_check(&self) -> HealthCheck {
            HealthCheck {
                status: HealthStatus::Ok,
                message: "echo plugin alive".into(),
            }
        }
        fn query(&self, req: &PluginQueryRequest) -> Vec<PluginQueryResponse> {
            vec![PluginQueryResponse {
                ref_id: req.ref_id.clone(),
                frames: vec![DataFrame {
                    name: req.expr.clone(),
                    fields: vec![DataField {
                        name: "value".into(),
                        values: vec![req.from_unix_ms as f64, req.to_unix_ms as f64],
                        labels: HashMap::new(),
                    }],
                }],
            }]
        }
    }

    fn manifest_for(id: &str) -> PluginManifest {
        PluginManifest {
            id: id.into(),
            kind: PluginType::Datasource,
            name: "Echo".into(),
            version: "1.0.0".into(),
            author: "cave".into(),
            executable: None,
            signature: None,
            meta: HashMap::new(),
        }
    }

    #[test]
    fn parse_toml_extracts_required_fields() {
        let text = r#"
        # comment
        id = "test"
        type = "datasource"
        version = "0.1.0"
        name = "Test"
        author = "burak"
        "#;
        let m = PluginManifest::parse_toml(text).unwrap();
        assert_eq!(m.id, "test");
        assert_eq!(m.kind, PluginType::Datasource);
        assert_eq!(m.version, "0.1.0");
        assert_eq!(m.author, "burak");
    }

    #[test]
    fn parse_toml_rejects_missing_id() {
        let text = r#"type = "panel"
                       version = "1""#;
        let err = PluginManifest::parse_toml(text).unwrap_err();
        assert!(err.contains("id"));
    }

    #[test]
    fn parse_toml_rejects_unknown_type() {
        let text = r#"
        id = "x"
        type = "weird"
        version = "1"
        "#;
        let err = PluginManifest::parse_toml(text).unwrap_err();
        assert!(err.contains("type"));
    }

    #[test]
    fn parse_toml_collects_extra_meta() {
        let text = r#"
        id = "x"
        type = "datasource"
        version = "1"
        category = "tsdb"
        "#;
        let m = PluginManifest::parse_toml(text).unwrap();
        assert_eq!(m.meta.get("category").map(String::as_str), Some("tsdb"));
    }

    #[test]
    fn registry_rejects_id_mismatch() {
        let mut r = PluginRegistry::new();
        let err = r
            .register(manifest_for("other"), Box::new(EchoPlugin))
            .unwrap_err();
        assert!(err.contains("manifest id"));
    }

    #[test]
    fn registry_rejects_duplicate_id() {
        let mut r = PluginRegistry::new();
        r.register(manifest_for("test-echo"), Box::new(EchoPlugin))
            .unwrap();
        let err = r
            .register(manifest_for("test-echo"), Box::new(EchoPlugin))
            .unwrap_err();
        assert!(err.contains("already registered"));
    }

    #[test]
    fn registry_returns_plugin_via_get() {
        let mut r = PluginRegistry::new();
        r.register(manifest_for("test-echo"), Box::new(EchoPlugin))
            .unwrap();
        let p = r.get("test-echo").unwrap();
        assert_eq!(p.id(), "test-echo");
        assert_eq!(p.health_check().status, HealthStatus::Ok);
    }

    #[test]
    fn registry_ids_of_filters_by_kind() {
        let mut r = PluginRegistry::new();
        r.register(manifest_for("a"), Box::new(EchoPlugin))
            .unwrap_err();
        // register with matching id only
        let mut m = manifest_for("test-echo");
        m.kind = PluginType::Datasource;
        r.register(m, Box::new(EchoPlugin)).unwrap();
        assert_eq!(
            r.ids_of(PluginType::Datasource),
            vec!["test-echo".to_string()]
        );
        assert!(r.ids_of(PluginType::Panel).is_empty());
    }

    #[test]
    fn plugin_query_round_trip() {
        let mut r = PluginRegistry::new();
        r.register(manifest_for("test-echo"), Box::new(EchoPlugin))
            .unwrap();
        let p = r.get("test-echo").unwrap();
        let req = PluginQueryRequest {
            ref_id: "A".into(),
            from_unix_ms: 100,
            to_unix_ms: 200,
            expr: "up".into(),
            max_data_points: 100,
            interval_ms: 1_000,
        };
        let rsp = p.query(&req);
        assert_eq!(rsp.len(), 1);
        assert_eq!(rsp[0].ref_id, "A");
        assert_eq!(rsp[0].frames[0].name, "up");
        assert_eq!(rsp[0].frames[0].fields[0].values, vec![100.0, 200.0]);
    }
}
