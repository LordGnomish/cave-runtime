//! Module-specific wrap registry.
//!
//! Each `wrap` describes how a single backend module (cave-pg, cave-cache,
//! cave-docdb, …) integrates into the portal: its display name, its
//! upstream lineage, the dashboards/alerts it owns, and the live-data
//! endpoints the portal calls to populate registry/cards.
//!
//! Loaded into [`PortalState::parity_cache`] via [`registry::list_all`]
//! so the registry UI can render every backend uniformly.

pub mod postgres;
pub mod cache;
pub mod docdb;
pub mod blob;
pub mod search;
pub mod iceberg;
pub mod datafusion;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wrap {
    /// Crate name, e.g. "cave-pg".
    pub module: String,
    /// Friendly UI title, e.g. "PostgreSQL (CloudNativePG)".
    pub title: String,
    /// `org/repo` of the upstream we replicate.
    pub upstream: String,
    /// Pinned upstream commit (matches parity.manifest.toml).
    pub upstream_commit: String,
    /// Short paragraph for registry card.
    pub description: String,
    /// Path (relative to repo root) of the Grafana dashboard JSON.
    pub dashboard_path: String,
    /// Path (relative to repo root) of the Prometheus alert rule YAML.
    pub alerts_path: String,
    /// HTTP endpoint that returns live data for the registry card.
    pub live_data_endpoint: String,
}

pub struct WrapRegistry {
    items: Vec<Wrap>,
}

impl WrapRegistry {
    pub fn new() -> Self {
        Self {
            items: vec![
                postgres::wrap(),
                cache::wrap(),
                docdb::wrap(),
                blob::wrap(),
                search::wrap(),
                iceberg::wrap(),
                datafusion::wrap(),
            ],
        }
    }

    pub fn list_all(&self) -> &[Wrap] {
        &self.items
    }

    pub fn get(&self, module: &str) -> Option<&Wrap> {
        self.items.iter().find(|w| w.module == module)
    }
}

impl Default for WrapRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lists_all_seven_data_persistence_wraps() {
        let r = WrapRegistry::new();
        let names: Vec<&str> = r.list_all().iter().map(|w| w.module.as_str()).collect();
        for expected in [
            "cave-pg",
            "cave-cache",
            "cave-docdb",
            "cave-blob",
            "cave-search",
            "cave-iceberg",
            "cave-datafusion",
        ] {
            assert!(names.contains(&expected), "missing wrap: {}", expected);
        }
        assert_eq!(r.list_all().len(), 7);
    }

    #[test]
    fn registry_get_returns_correct_wrap() {
        let r = WrapRegistry::new();
        let pg = r.get("cave-pg").expect("cave-pg wrap registered");
        assert_eq!(pg.upstream.split('/').next().unwrap(), "postgres");
    }
}
