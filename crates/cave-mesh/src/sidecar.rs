//! Sidecar, EnvoyFilter, and WorkloadGroup resource managers.
//!
//! Three independent managers:
//!   • SidecarManager    — Sidecar resources (workload proxy config)
//!   • EnvoyFilterManager — EnvoyFilter resources (direct Envoy patches)
//!   • WorkloadGroupManager — WorkloadGroup + standalone WorkloadEntry (VM integration)

use crate::models::{EnvoyFilter, Sidecar, WorkloadEntry, WorkloadGroup};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::debug;

// ─────────────────────────────────────────────────────────────
// SidecarManager
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SidecarManager {
    /// Keyed by "namespace/name"
    sidecars: Arc<RwLock<HashMap<String, Sidecar>>>,
}

impl Default for SidecarManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SidecarManager {
    pub fn new() -> Self {
        Self { sidecars: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub fn upsert(&self, sc: Sidecar) {
        let key = format!("{}/{}", sc.namespace, sc.name);
        self.sidecars.write().unwrap().insert(key, sc);
    }

    pub fn remove(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        self.sidecars.write().unwrap().remove(&key);
    }

    pub fn list(&self) -> Vec<Sidecar> {
        self.sidecars.read().unwrap().values().cloned().collect()
    }

    pub fn get(&self, namespace: &str, name: &str) -> Option<Sidecar> {
        let key = format!("{namespace}/{name}");
        self.sidecars.read().unwrap().get(&key).cloned()
    }

    /// Resolve the effective Sidecar for a workload.
    ///
    /// Priority: workload-specific (matching selector) > namespace-wide > None.
    pub fn effective_sidecar(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
    ) -> Option<Sidecar> {
        let map = self.sidecars.read().unwrap();

        let mut workload_match: Option<Sidecar> = None;
        let mut ns_match: Option<Sidecar> = None;

        for sc in map.values() {
            if sc.namespace != namespace {
                continue;
            }
            let is_ns_wide = sc.selector.as_ref().map(|s| s.is_empty()).unwrap_or(true);
            if is_ns_wide {
                ns_match = Some(sc.clone());
            } else if let Some(sel) = &sc.selector {
                let matches = sel.iter().all(|(k, v)| {
                    workload_labels.get(k).map(|vv| vv == v).unwrap_or(false)
                });
                if matches {
                    workload_match = Some(sc.clone());
                }
            }
        }

        workload_match.or(ns_match)
    }

    /// List all hosts accessible from a workload (based on egress listeners).
    pub fn accessible_hosts(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
    ) -> Vec<String> {
        match self.effective_sidecar(namespace, workload_labels) {
            None => vec!["*/*".to_string()], // default: all hosts accessible
            Some(sc) => {
                let mut hosts = vec![];
                for listener in &sc.egress {
                    hosts.extend(listener.hosts.iter().cloned());
                }
                if hosts.is_empty() {
                    hosts.push("*/*".to_string());
                }
                hosts
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// EnvoyFilterManager
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EnvoyFilterManager {
    /// Keyed by "namespace/name"; sorted by priority when retrieved.
    filters: Arc<RwLock<HashMap<String, EnvoyFilter>>>,
}

impl Default for EnvoyFilterManager {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvoyFilterManager {
    pub fn new() -> Self {
        Self { filters: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub fn upsert(&self, ef: EnvoyFilter) {
        let key = format!("{}/{}", ef.namespace, ef.name);
        self.filters.write().unwrap().insert(key, ef);
    }

    pub fn remove(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        self.filters.write().unwrap().remove(&key);
    }

    pub fn list(&self) -> Vec<EnvoyFilter> {
        let mut filters: Vec<EnvoyFilter> =
            self.filters.read().unwrap().values().cloned().collect();
        // Sort by priority (ascending — lower numbers applied first)
        filters.sort_by_key(|f| f.priority);
        filters
    }

    pub fn get(&self, namespace: &str, name: &str) -> Option<EnvoyFilter> {
        let key = format!("{namespace}/{name}");
        self.filters.read().unwrap().get(&key).cloned()
    }

    /// Filters applicable to a workload (namespace + labels), sorted by priority.
    pub fn filters_for_workload(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
    ) -> Vec<EnvoyFilter> {
        let mut matching: Vec<EnvoyFilter> = self
            .filters
            .read()
            .unwrap()
            .values()
            .filter(|ef| {
                ef.namespace == namespace
                    && ef
                        .selector
                        .as_ref()
                        .map(|sel| {
                            sel.iter()
                                .all(|(k, v)| workload_labels.get(k).map(|vv| vv == v).unwrap_or(false))
                        })
                        .unwrap_or(true)
            })
            .cloned()
            .collect();
        matching.sort_by_key(|f| f.priority);
        debug!(
            namespace = %namespace,
            count = matching.len(),
            "EnvoyFilters resolved for workload"
        );
        matching
    }
}

// ─────────────────────────────────────────────────────────────
// WorkloadGroupManager
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WorkloadGroupManager {
    groups: Arc<RwLock<HashMap<String, WorkloadGroup>>>,
    /// Standalone WorkloadEntry resources (keyed by "namespace/name")
    entries: Arc<RwLock<HashMap<String, WorkloadEntry>>>,
}

impl Default for WorkloadGroupManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkloadGroupManager {
    pub fn new() -> Self {
        Self {
            groups: Arc::new(RwLock::new(HashMap::new())),
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ─── WorkloadGroup CRUD ──────────────────────────────────

    pub fn upsert_group(&self, wg: WorkloadGroup) {
        let key = format!("{}/{}", wg.namespace, wg.name);
        self.groups.write().unwrap().insert(key, wg);
    }

    pub fn remove_group(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        self.groups.write().unwrap().remove(&key);
    }

    pub fn list_groups(&self) -> Vec<WorkloadGroup> {
        self.groups.read().unwrap().values().cloned().collect()
    }

    pub fn get_group(&self, namespace: &str, name: &str) -> Option<WorkloadGroup> {
        let key = format!("{namespace}/{name}");
        self.groups.read().unwrap().get(&key).cloned()
    }

    // ─── WorkloadEntry CRUD ──────────────────────────────────

    pub fn upsert_entry(&self, mut entry: WorkloadEntry) {
        let name = entry.name.get_or_insert_with(|| {
            entry.address.replace('.', "-")
        });
        let ns = entry.namespace.clone().unwrap_or_else(|| "default".to_string());
        let key = format!("{ns}/{name}");
        self.entries.write().unwrap().insert(key, entry);
    }

    pub fn remove_entry(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        self.entries.write().unwrap().remove(&key);
    }

    pub fn list_entries(&self) -> Vec<WorkloadEntry> {
        self.entries.read().unwrap().values().cloned().collect()
    }

    pub fn get_entry(&self, namespace: &str, name: &str) -> Option<WorkloadEntry> {
        let key = format!("{namespace}/{name}");
        self.entries.read().unwrap().get(&key).cloned()
    }

    /// Entries belonging to a specific WorkloadGroup (matched by group's selector).
    pub fn entries_for_group(&self, group: &WorkloadGroup) -> Vec<WorkloadEntry> {
        let entries = self.entries.read().unwrap();
        entries
            .values()
            .filter(|e| {
                let en = e.namespace.as_deref().unwrap_or("default");
                if en != group.namespace {
                    return false;
                }
                if let Some(sel) = &group.selector {
                    sel.iter()
                        .all(|(k, v)| e.labels.get(k).map(|vv| vv == v).unwrap_or(false))
                } else {
                    true
                }
            })
            .cloned()
            .collect()
    }

    /// Snapshot of the workload group / entry state.
    pub fn snapshot(&self) -> WorkloadSnapshot {
        WorkloadSnapshot {
            total_groups: self.groups.read().unwrap().len(),
            total_entries: self.entries.read().unwrap().len(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadSnapshot {
    pub total_groups: usize,
    pub total_entries: usize,
}
