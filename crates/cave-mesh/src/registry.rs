//! Service discovery and registry.
//!
//! Maintains a live map of service name → healthy endpoints.
//! Supports registration, deregistration, and health-status updates.

use crate::models::{Endpoint, HealthStatus, ServiceMeta};
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

/// Thread-safe service registry: service name → endpoints.
#[derive(Debug, Clone)]
pub struct ServiceRegistry {
    // key = "namespace/service-name"
    inner: Arc<RwLock<HashMap<String, ServiceRecord>>>,
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn svc_key(namespace: &str, name: &str) -> String {
        format!("{namespace}/{name}")
    }

    /// Register or update a service and add one endpoint.
    pub fn register(&self, meta: ServiceMeta, endpoint: Endpoint) {
        let key = Self::svc_key(&meta.namespace, &meta.name);
        let mut map = self.inner.write().unwrap();
        let record = map.entry(key.clone()).or_insert_with(|| {
            info!(service = %key, "Service registered");
            ServiceRecord {
                meta: meta.clone(),
                endpoints: vec![],
            }
        });
        // Upsert by address:port
        let addr = endpoint.address.clone();
        let port = endpoint.port;
        if let Some(e) = record
            .endpoints
            .iter_mut()
            .find(|e| e.address == addr && e.port == port)
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
            record
                .endpoints
                .retain(|e| !(e.address == addr && e.port == port));
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
            if let Some(e) = record
                .endpoints
                .iter_mut()
                .find(|e| e.address == addr && e.port == port)
            {
                e.health = status;
                e.last_checked = chrono::Utc::now();
            }
        }
    }

    /// Resolve healthy endpoints for a service (namespace/name or just name).
    pub fn resolve(&self, service_name: &str) -> Vec<Endpoint> {
        self.resolve_impl(service_name, true)
    }

    /// Resolve ALL endpoints (including unhealthy) for a service.
    pub fn resolve_all(&self, service_name: &str) -> Vec<Endpoint> {
        self.resolve_impl(service_name, false)
    }

    fn resolve_impl(&self, service_name: &str, healthy_only: bool) -> Vec<Endpoint> {
        let map = self.inner.read().unwrap();
        // Try exact key first, then search by bare name
        let record = if let Some(r) = map.get(service_name) {
            Some(r)
        } else {
            map.values()
                .find(|r| r.meta.name == service_name)
        };

        match record {
            None => vec![],
            Some(r) => r
                .endpoints
                .iter()
                .filter(|e| {
                    !healthy_only
                        || e.health == HealthStatus::Healthy
                        || e.health == HealthStatus::Unknown
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
