// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant / namespace registry shared by the Kafka and Pulsar wire layers.
//!
//! Per ADR-RUNTIME-STREAMING-CONSOLIDATION-001, addressing is Pulsar-native
//! (`persistent://tenant/ns/topic`); the Kafka wire path translates a flat
//! Kafka topic into the canonical 3-tuple by deriving tenant + namespace
//! from the leading prefix segments of the topic name (or assigning the
//! `public/default` defaults if the name has none).

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::RwLock;

use crate::error::{StreamsError, StreamsResult};

/// Default tenant when the wire layer cannot infer one.  Mirrors Pulsar's
/// `pulsar.client.PulsarClient.DEFAULT_TENANT`.
pub const DEFAULT_TENANT: &str = "public";

/// Default namespace when the wire layer cannot infer one.  Mirrors Pulsar's
/// `pulsar.client.PulsarClient.DEFAULT_NAMESPACE`.
pub const DEFAULT_NAMESPACE: &str = "default";

/// Tenant configuration (admin-managed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub name: String,
    /// Roles allowed to administer this tenant (Pulsar `admin_roles`).
    pub admin_roles: HashSet<String>,
    /// Cluster names this tenant is allowed to dispatch to.
    pub allowed_clusters: HashSet<String>,
}

impl Tenant {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            admin_roles: HashSet::new(),
            allowed_clusters: HashSet::new(),
        }
    }
}

/// Namespace configuration scoped to a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub tenant: String,
    pub name: String,
    /// Replication clusters (Pulsar `replicationClusters`).
    pub replication_clusters: HashSet<String>,
    /// Per-namespace message TTL in seconds (0 = none).
    pub message_ttl_secs: u32,
    /// Per-namespace per-topic retention in MB (0 = unlimited).
    pub retention_mb: u64,
}

impl Namespace {
    pub fn new(tenant: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            tenant: tenant.into(),
            name: name.into(),
            replication_clusters: HashSet::new(),
            message_ttl_secs: 0,
            retention_mb: 0,
        }
    }

    /// Fully-qualified name `tenant/namespace`.
    pub fn fqn(&self) -> String {
        format!("{}/{}", self.tenant, self.name)
    }
}

/// In-memory tenant + namespace registry.  Lock-free reads via DashMap;
/// the global default-allocation policy is guarded by a single `RwLock`.
pub struct TenantRegistry {
    tenants: DashMap<String, Tenant>,
    /// Key = `tenant/namespace`.
    namespaces: DashMap<String, Namespace>,
    /// When `true`, a `get_or_create_default` call will auto-create the
    /// `public/default` tenant + namespace on demand.  Pulsar enables this
    /// in standalone mode and disables in production.
    autocreate_default: RwLock<bool>,
}

impl TenantRegistry {
    pub fn new() -> Self {
        Self {
            tenants: DashMap::new(),
            namespaces: DashMap::new(),
            autocreate_default: RwLock::new(true),
        }
    }

    pub fn create_tenant(&self, t: Tenant) -> StreamsResult<()> {
        if self.tenants.contains_key(&t.name) {
            return Err(StreamsError::Internal(format!(
                "tenant already exists: {}",
                t.name
            )));
        }
        self.tenants.insert(t.name.clone(), t);
        Ok(())
    }

    pub fn delete_tenant(&self, name: &str) -> StreamsResult<()> {
        // Disallow delete while namespaces survive.
        let in_use = self.namespaces.iter().any(|e| e.value().tenant == name);
        if in_use {
            return Err(StreamsError::Internal(format!(
                "tenant {name} still has namespaces"
            )));
        }
        self.tenants
            .remove(name)
            .ok_or_else(|| StreamsError::Internal(format!("tenant not found: {name}")))?;
        Ok(())
    }

    pub fn get_tenant(&self, name: &str) -> Option<Tenant> {
        self.tenants.get(name).map(|r| r.clone())
    }

    pub fn list_tenants(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tenants.iter().map(|e| e.key().clone()).collect();
        v.sort();
        v
    }

    pub fn create_namespace(&self, ns: Namespace) -> StreamsResult<()> {
        if !self.tenants.contains_key(&ns.tenant) {
            return Err(StreamsError::Internal(format!(
                "tenant not found: {}",
                ns.tenant
            )));
        }
        let key = ns.fqn();
        if self.namespaces.contains_key(&key) {
            return Err(StreamsError::Internal(format!(
                "namespace already exists: {key}"
            )));
        }
        self.namespaces.insert(key, ns);
        Ok(())
    }

    pub fn delete_namespace(&self, fqn: &str) -> StreamsResult<()> {
        self.namespaces
            .remove(fqn)
            .ok_or_else(|| StreamsError::Internal(format!("namespace not found: {fqn}")))?;
        Ok(())
    }

    pub fn get_namespace(&self, fqn: &str) -> Option<Namespace> {
        self.namespaces.get(fqn).map(|r| r.clone())
    }

    pub fn list_namespaces(&self, tenant: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .namespaces
            .iter()
            .filter(|e| e.value().tenant == tenant)
            .map(|e| e.value().name.clone())
            .collect();
        v.sort();
        v
    }

    pub fn set_autocreate_default(&self, on: bool) {
        *self.autocreate_default.write().unwrap() = on;
    }

    pub fn autocreate_default(&self) -> bool {
        *self.autocreate_default.read().unwrap()
    }

    /// Look up `(tenant, namespace)` and create the public/default pair on
    /// demand if `autocreate_default` is on (matches Pulsar standalone).
    pub fn ensure_namespace(&self, tenant: &str, namespace: &str) -> StreamsResult<Namespace> {
        let fqn = format!("{tenant}/{namespace}");
        if let Some(ns) = self.get_namespace(&fqn) {
            return Ok(ns);
        }
        if !self.autocreate_default() {
            return Err(StreamsError::Internal(format!(
                "namespace not found and autocreate disabled: {fqn}"
            )));
        }
        if !self.tenants.contains_key(tenant) {
            self.create_tenant(Tenant::new(tenant))?;
        }
        let ns = Namespace::new(tenant, namespace);
        self.create_namespace(ns.clone())?;
        Ok(ns)
    }
}

impl Default for TenantRegistry {
    fn default() -> Self {
        let r = Self::new();
        // Pre-seed the default tenant + namespace so Kafka clients that send
        // raw topic names without explicit tenant addressing keep working.
        let _ = r.create_tenant(Tenant::new(DEFAULT_TENANT));
        let _ = r.create_namespace(Namespace::new(DEFAULT_TENANT, DEFAULT_NAMESPACE));
        r
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tenant / namespace registry tests
// feat/cave-streams-kafka-pulsar-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_create_and_get() {
        // cite: pulsar 4.2.0 pulsar-broker/.../TenantsBase.java#createTenant
        let tenant_id = "tenant-001";
        let reg = TenantRegistry::new();
        let mut t = Tenant::new(tenant_id);
        t.admin_roles.insert("admin".into());
        t.allowed_clusters.insert("standalone".into());
        reg.create_tenant(t).unwrap();
        let got = reg.get_tenant(tenant_id).unwrap();
        assert_eq!(got.name, tenant_id);
        assert!(got.admin_roles.contains("admin"));
    }

    #[test]
    fn test_tenant_create_duplicate_rejected() {
        // cite: pulsar 4.2.0 .../TenantsBase.java (409 ConflictException)
        let tenant_id = "tenant-002";
        let reg = TenantRegistry::new();
        reg.create_tenant(Tenant::new(tenant_id)).unwrap();
        let err = reg.create_tenant(Tenant::new(tenant_id));
        assert!(err.is_err());
    }

    #[test]
    fn test_tenant_delete_blocked_by_namespaces() {
        // cite: pulsar 4.2.0 .../TenantsBase.java#deleteTenant (PreconditionFailed)
        let tenant_id = "tenant-003";
        let reg = TenantRegistry::new();
        reg.create_tenant(Tenant::new(tenant_id)).unwrap();
        reg.create_namespace(Namespace::new(tenant_id, "ns"))
            .unwrap();
        let err = reg.delete_tenant(tenant_id);
        assert!(err.is_err(), "delete must fail while ns exists");
    }

    #[test]
    fn test_namespace_create_requires_existing_tenant() {
        // cite: pulsar 4.2.0 .../NamespacesBase.java (NotFound when tenant missing)
        let _tenant_id = "tenant-004";
        let reg = TenantRegistry::new();
        let err = reg.create_namespace(Namespace::new("ghost-tenant", "ns"));
        assert!(err.is_err());
    }

    #[test]
    fn test_ensure_namespace_autocreate_default() {
        // cite: pulsar 4.2.0 standalone Mode (allowAutoTopicCreation=true)
        let tenant_id = "tenant-005";
        let reg = TenantRegistry::new();
        // autocreate is on by default
        let ns = reg.ensure_namespace(tenant_id, "ns").unwrap();
        assert_eq!(ns.tenant, tenant_id);
        assert_eq!(ns.name, "ns");
        // tenant was created on demand
        assert!(reg.get_tenant(tenant_id).is_some());
    }

    #[test]
    fn test_ensure_namespace_autocreate_off_rejects() {
        // cite: pulsar 4.2.0 broker.conf allowAutoTopicCreation=false
        let tenant_id = "tenant-006";
        let reg = TenantRegistry::new();
        reg.set_autocreate_default(false);
        let err = reg.ensure_namespace(tenant_id, "ns");
        assert!(err.is_err());
    }

    #[test]
    fn test_list_namespaces_per_tenant() {
        // cite: pulsar 4.2.0 .../NamespacesBase.java#getNamespaces
        let tenant_id = "tenant-007";
        let reg = TenantRegistry::new();
        reg.create_tenant(Tenant::new(tenant_id)).unwrap();
        reg.create_namespace(Namespace::new(tenant_id, "alpha"))
            .unwrap();
        reg.create_namespace(Namespace::new(tenant_id, "beta"))
            .unwrap();
        // Other tenant — should not bleed into the listing.
        reg.create_tenant(Tenant::new("other")).unwrap();
        reg.create_namespace(Namespace::new("other", "x")).unwrap();
        let mut ns = reg.list_namespaces(tenant_id);
        ns.sort();
        assert_eq!(ns, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn test_default_registry_seeds_public_default() {
        // cite: pulsar 4.2.0 PulsarStandalone.java (auto-seeds public/default)
        let _tenant_id = "tenant-008";
        let reg = TenantRegistry::default();
        assert!(reg.get_tenant(DEFAULT_TENANT).is_some());
        assert!(
            reg.get_namespace(&format!("{}/{}", DEFAULT_TENANT, DEFAULT_NAMESPACE))
                .is_some()
        );
    }
}
