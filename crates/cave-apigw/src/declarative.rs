// SPDX-License-Identifier: AGPL-3.0-or-later
//! decK-style declarative config — import/export the gateway state as YAML/JSON.

use crate::error::AGwResult;
use crate::models::{Consumer, Plugin, Route, Service, Upstream};
use crate::store::GwStore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeclarativeConfig {
    #[serde(default)] pub _format_version: String,
    #[serde(default)] pub routes: Vec<Route>,
    #[serde(default)] pub services: Vec<Service>,
    #[serde(default)] pub upstreams: Vec<Upstream>,
    #[serde(default)] pub consumers: Vec<Consumer>,
    #[serde(default)] pub plugins: Vec<Plugin>,
}

pub fn export_yaml(store: &Arc<GwStore>) -> AGwResult<String> {
    let cfg = DeclarativeConfig {
        _format_version: "3.0".into(),
        routes: store.list_routes(),
        services: store.list_services(),
        upstreams: store.list_upstreams(),
        consumers: store.list_consumers(),
        plugins: store.list_plugins(),
    };
    Ok(serde_yaml::to_string(&cfg)?)
}

pub fn export_json(store: &Arc<GwStore>) -> AGwResult<String> {
    let cfg = DeclarativeConfig {
        _format_version: "3.0".into(),
        routes: store.list_routes(),
        services: store.list_services(),
        upstreams: store.list_upstreams(),
        consumers: store.list_consumers(),
        plugins: store.list_plugins(),
    };
    Ok(serde_json::to_string_pretty(&cfg)?)
}

/// Replace the in-memory store contents with `cfg`. Existing entries with
/// matching IDs are overwritten; missing entries are NOT removed (use
/// `apply_strict` for full replace).
pub fn apply(store: &Arc<GwStore>, cfg: DeclarativeConfig) -> AGwResult<usize> {
    let mut n = 0usize;
    for s in cfg.services { store.upsert_service(s)?; n += 1; }
    for u in cfg.upstreams { store.upsert_upstream(u)?; n += 1; }
    for r in cfg.routes { store.upsert_route(r)?; n += 1; }
    for c in cfg.consumers { store.upsert_consumer(c)?; n += 1; }
    for p in cfg.plugins { store.upsert_plugin(p)?; n += 1; }
    Ok(n)
}

pub fn import_yaml(store: &Arc<GwStore>, yaml: &str) -> AGwResult<usize> {
    let cfg: DeclarativeConfig = serde_yaml::from_str(yaml)?;
    apply(store, cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Route, Service};

    #[test] fn round_trip_yaml() {
        let s = Arc::new(GwStore::new());
        s.upsert_service(Service::new("svc", "h", 80)).unwrap();
        s.upsert_route(Route::new("r")).unwrap();
        let yaml = export_yaml(&s).unwrap();
        assert!(yaml.contains("svc"));
        let s2 = Arc::new(GwStore::new());
        let n = import_yaml(&s2, &yaml).unwrap();
        assert!(n >= 2);
        assert_eq!(s2.list_services().len(), 1);
    }
    #[test] fn export_json_valid() {
        let s = Arc::new(GwStore::new());
        s.upsert_service(Service::new("svc", "h", 80)).unwrap();
        let j = export_json(&s).unwrap();
        let _: serde_json::Value = serde_json::from_str(&j).unwrap();
    }
    #[test] fn apply_empty_no_op() {
        let s = Arc::new(GwStore::new());
        let cfg = DeclarativeConfig::default();
        assert_eq!(apply(&s, cfg).unwrap(), 0);
    }
    #[test] fn import_yaml_parses_minimal() {
        let s = Arc::new(GwStore::new());
        let yaml = "_format_version: '3.0'\nservices: []\n";
        assert_eq!(import_yaml(&s, yaml).unwrap(), 0);
    }
    #[test] fn export_includes_format_version() {
        let s = Arc::new(GwStore::new());
        let y = export_yaml(&s).unwrap();
        assert!(y.contains("3.0"));
    }
}
