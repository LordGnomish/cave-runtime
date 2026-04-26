//! Tenant isolation — RBAC and network policy templates.

use crate::models::{NetworkPolicyMode, TenantIsolation};
use dashmap::DashMap;

pub struct TenantRegistry {
    tenants: DashMap<String, TenantIsolation>,
}

impl TenantRegistry {
    pub fn new() -> Self {
        Self { tenants: DashMap::new() }
    }

    pub fn register(&self, tenant_id: &str, namespace_prefix: &str, mode: NetworkPolicyMode) {
        self.tenants.insert(tenant_id.to_owned(), TenantIsolation {
            tenant_id: tenant_id.to_owned(),
            namespace_prefix: namespace_prefix.to_owned(),
            network_policy: mode,
            rbac_template: default_rbac_template(tenant_id),
        });
    }

    pub fn get(&self, tenant_id: &str) -> Option<TenantIsolation> {
        self.tenants.get(tenant_id).map(|r| r.clone())
    }

    pub fn list(&self) -> Vec<TenantIsolation> {
        self.tenants.iter().map(|r| r.value().clone()).collect()
    }
}

fn default_rbac_template(tenant_id: &str) -> String {
    format!(
        "apiVersion: rbac.authorization.k8s.io/v1\nkind: Role\nmetadata:\n  name: vcluster-{tenant_id}\n"
    )
}

impl Default for TenantRegistry {
    fn default() -> Self { Self::new() }
}
