// SPDX-License-Identifier: AGPL-3.0-or-later
//! Service discovery registry.
//!
//! Maintains a live map of namespace/service → endpoints.
//! Supports subset filtering, health filtering, and locality metadata.

use crate::models::{Endpoint, HealthStatus, Locality, ServiceMeta};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::{debug, info};

#[derive(Debug)]
struct ServiceRecord {
    meta: ServiceMeta,
    endpoints: Vec<Endpoint>,
}

/// Thread-safe service registry keyed by "namespace/name".
#[derive(Debug, Clone)]
pub struct ServiceRegistry {
    inner: Arc<RwLock<HashMap<String, ServiceRecord>>>,
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())) }
    }

    fn svc_key(namespace: &str, name: &str) -> String {
        format!("{namespace}/{name}")
    }

    /// Register or update a service and upsert one endpoint.
    pub fn register(&self, meta: ServiceMeta, endpoint: Endpoint) {
        let key = Self::svc_key(&meta.namespace, &meta.name);
        let mut map = self.inner.write().unwrap();
        let record = map.entry(key.clone()).or_insert_with(|| {
            info!(service = %key, "Service registered");
            ServiceRecord { meta: meta.clone(), endpoints: vec![] }
        });
        let addr = endpoint.address.clone();
        let port = endpoint.port;
        if let Some(e) = record.endpoints.iter_mut().find(|e| e.address == addr && e.port == port)
        {
            *e = endpoint;
        } else {
            debug!(service = %key, addr = %addr, "Endpoint added");
            record.endpoints.push(endpoint);
        }
    }

    /// Remove a specific endpoint from a service.
    pub fn deregister(&self, namespace: &str, service_name: &str, addr: &str, port: u16) {
        let key = Self::svc_key(namespace, service_name);
        let mut map = self.inner.write().unwrap();
        if let Some(record) = map.get_mut(&key) {
            record.endpoints.retain(|e| !(e.address == addr && e.port == port));
            debug!(service = %key, addr = %addr, "Endpoint deregistered");
        }
    }

    /// Update the health status of a specific endpoint.
    pub fn update_health(
        &self,
        namespace: &str,
        service_name: &str,
        addr: &str,
        port: u16,
        status: HealthStatus,
    ) {
        let key = Self::svc_key(namespace, service_name);
        let mut map = self.inner.write().unwrap();
        if let Some(record) = map.get_mut(&key) {
            if let Some(e) =
                record.endpoints.iter_mut().find(|e| e.address == addr && e.port == port)
            {
                e.health = status;
                e.last_checked = chrono::Utc::now();
            }
        }
    }

    /// Resolve healthy (or Unknown) endpoints for a service.
    pub fn resolve(&self, service_name: &str) -> Vec<Endpoint> {
        self.resolve_impl(service_name, true, None)
    }

    /// Resolve all endpoints (including unhealthy).
    pub fn resolve_all(&self, service_name: &str) -> Vec<Endpoint> {
        self.resolve_impl(service_name, false, None)
    }

    /// Resolve endpoints filtered by label subset.
    pub fn resolve_subset(
        &self,
        service_name: &str,
        subset_labels: &HashMap<String, String>,
    ) -> Vec<Endpoint> {
        self.resolve_impl(service_name, true, Some(subset_labels))
    }

    /// Resolve endpoints in a specific locality (for locality-aware LB).
    pub fn resolve_locality(
        &self,
        service_name: &str,
        locality: &Locality,
    ) -> Vec<Endpoint> {
        let all = self.resolve(service_name);
        let in_region: Vec<_> = all
            .iter()
            .filter(|e| {
                e.locality.as_ref().map(|l| l.region == locality.region).unwrap_or(false)
            })
            .cloned()
            .collect();
        if in_region.is_empty() { all } else { in_region }
    }

    fn resolve_impl(
        &self,
        service_name: &str,
        healthy_only: bool,
        subset_labels: Option<&HashMap<String, String>>,
    ) -> Vec<Endpoint> {
        let map = self.inner.read().unwrap();
        let record = if let Some(r) = map.get(service_name) {
            Some(r)
        } else {
            map.values().find(|r| r.meta.name == service_name)
        };

        match record {
            None => vec![],
            Some(r) => r
                .endpoints
                .iter()
                .filter(|e| {
                    if healthy_only
                        && e.health == HealthStatus::Unhealthy
                    {
                        return false;
                    }
                    if let Some(sel) = subset_labels {
                        if !sel.iter().all(|(k, v)| e.labels.get(k).map(|vv| vv == v).unwrap_or(false)) {
                            return false;
                        }
                    }
                    true
                })
                .cloned()
                .collect(),
        }
    }

    /// List all registered service metadata.
    pub fn list_services(&self) -> Vec<ServiceMeta> {
        let map = self.inner.read().unwrap();
        map.values().map(|r| r.meta.clone()).collect()
    }

    /// Get a service's metadata.
    pub fn get_service(&self, namespace: &str, service_name: &str) -> Option<ServiceMeta> {
        let key = Self::svc_key(namespace, service_name);
        let map = self.inner.read().unwrap();
        map.get(&key).map(|r| r.meta.clone())
    }
}
