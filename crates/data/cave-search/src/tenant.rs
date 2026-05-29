// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant identity primitives and multi-tenant index registry.
//!
//! `TenantId` is re-exported from `cave_kernel::ns` (DNS-1123-validated newtype).
//! The `Tenant` aggregate and `TenantRegistry` provide per-tenant namespace
//! isolation: index names are scoped to a tenant and never bleed across boundaries.
//!
//! upstream: manticoresoftware/manticoresearch — searchd multi-tenant mode

pub use cave_kernel::ns::TenantId;

use std::collections::HashMap;

/// Tenant aggregate: a registered tenant with metadata.
#[derive(Debug, Clone)]
pub struct Tenant {
    /// Canonical DNS-1123 tenant identifier.
    pub id: TenantId,
    /// Optional display label (defaults to None).
    pub display_name: Option<String>,
}

impl Tenant {
    /// Create a new `Tenant` from a `TenantId` with no display name.
    pub fn new(id: TenantId) -> Self {
        Tenant { id, display_name: None }
    }

    /// Create a `Tenant` with a display name.
    pub fn with_display_name(id: TenantId, display_name: impl Into<String>) -> Self {
        Tenant { id, display_name: Some(display_name.into()) }
    }
}

/// In-memory registry of all registered tenants.
///
/// In production this would be backed by cave-etcd or cave-rdbms.
#[derive(Debug, Default)]
pub struct TenantRegistry {
    tenants: HashMap<String, Tenant>,
}

impl TenantRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        TenantRegistry { tenants: HashMap::new() }
    }

    /// Register a tenant, replacing any existing entry with the same ID.
    pub fn register(&mut self, tenant: Tenant) {
        self.tenants.insert(tenant.id.as_str().to_string(), tenant);
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
