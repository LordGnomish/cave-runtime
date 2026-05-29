// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant identity primitives and multi-tenant index registry.
//!
//! `TenantId` is re-exported from `cave_kernel::ns` (sweep-002 F2-G adoption,
//! 2026-05-01) so cave-search shares the canonical DNS-1123-validated newtype
//! used by the rest of the platform. The local `Tenant` aggregate and
//! `TenantRegistry` live here to support per-tenant index isolation.
//!
//! Each tenant has an independent namespace of indices: index names are scoped
//! to the tenant and never bleed across tenant boundaries.
//!
//! upstream: manticoresoftware/manticoresearch — searchd multi-tenant mode
//!           (percolate table + distributed cluster per tenant namespace)

pub use cave_kernel::ns::TenantId;

use std::collections::HashMap;

/// Tenant aggregate: a registered tenant with metadata.
#[derive(Debug, Clone)]
pub struct Tenant {
    /// Canonical DNS-1123 tenant identifier.
    pub id: TenantId,
    /// Display label for the tenant (optional, defaults to the ID string).
    pub display_name: Option<String>,
}

impl Tenant {
    /// Create a new `Tenant` from a `TenantId`.
    pub fn new(id: TenantId) -> Self {
        Tenant {
            id,
            display_name: None,
        }
    }

    /// Create a new `Tenant` with an optional display name.
    pub fn with_display_name(id: TenantId, display_name: impl Into<String>) -> Self {
        Tenant {
            id,
            display_name: Some(display_name.into()),
        }
    }
}

/// In-memory registry of all registered tenants.
///
/// In production this would be backed by cave-etcd or cave-rdbms;
/// the in-memory version is the correct in-process abstraction for
/// single-binary operation.
#[derive(Debug, Default)]
pub struct TenantRegistry {
    tenants: HashMap<String, Tenant>,
}

impl TenantRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        TenantRegistry {
            tenants: HashMap::new(),
        }
    }

    /// Register a tenant. If a tenant with the same ID already exists,
    /// the old entry is replaced.
    pub fn register(&mut self, tenant: Tenant) {
        self.tenants
            .insert(tenant.id.as_str().to_string(), tenant);
    }

    /// Return a reference to the tenant with the given ID, or `None`.
    pub fn get(&self, id: &str) -> Option<&Tenant> {
        self.tenants.get(id)
    }

    /// Remove the tenant with the given ID. No-op if not present.
    pub fn remove(&mut self, id: &str) {
        self.tenants.remove(id);
    }

    /// Return the number of registered tenants.
    pub fn count(&self) -> usize {
        self.tenants.len()
    }

    /// Return a sorted list of all tenant ID strings.
    pub fn list_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.tenants.keys().cloned().collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn tenant_new() {
        let id = TenantId::from_str("abc").unwrap();
        let t = Tenant::new(id);
        assert_eq!(t.id.as_str(), "abc");
    }

    #[test]
    fn registry_register_get_remove() {
        let mut r = TenantRegistry::new();
        let id = TenantId::from_str("test").unwrap();
        r.register(Tenant::new(id));
        assert!(r.get("test").is_some());
        r.remove("test");
        assert!(r.get("test").is_none());
    }
}
