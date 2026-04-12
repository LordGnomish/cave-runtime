//! Multi-tenant support.
//!
//! Tenancy follows the Grafana/Tempo convention: the `X-Scope-OrgID` HTTP header
//! carries the tenant identifier. Absent header → "default" tenant.

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use serde::{Deserialize, Serialize};

use crate::{Result, TraceError};

pub const DEFAULT_TENANT: &str = "default";
pub const TENANT_HEADER: &str = "x-scope-orgid";

// ─── Tenant config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantConfig {
    pub tenant_id: String,
    /// Maximum traces retained for this tenant.
    pub max_traces: usize,
    /// Retention duration in hours.
    pub retention_hours: u64,
    /// Whether this tenant is allowed to query across tenants.
    pub cross_tenant_query: bool,
}

impl Default for TenantConfig {
    fn default() -> Self {
        TenantConfig {
            tenant_id: DEFAULT_TENANT.into(),
            max_traces: 100_000,
            retention_hours: 72,
            cross_tenant_query: false,
        }
    }
}

// ─── Registry ──────────────────────────────────────────────────────────────

pub struct TenantRegistry {
    configs: RwLock<HashMap<String, TenantConfig>>,
    /// If true, unknown tenants are auto-registered with defaults.
    auto_register: bool,
}

impl Default for TenantRegistry {
    fn default() -> Self {
        let mut configs = HashMap::new();
        configs.insert(DEFAULT_TENANT.into(), TenantConfig::default());
        TenantRegistry {
            configs: RwLock::new(configs),
            auto_register: true,
        }
    }
}

impl TenantRegistry {
    pub fn new(auto_register: bool) -> Self {
        let mut r = TenantRegistry::default();
        r.auto_register = auto_register;
        r
    }

    pub fn register(&self, config: TenantConfig) {
        self.configs.write().unwrap().insert(config.tenant_id.clone(), config);
    }

    pub fn get(&self, tenant_id: &str) -> Result<TenantConfig> {
        let guard = self.configs.read().unwrap();
        if let Some(cfg) = guard.get(tenant_id) {
            return Ok(cfg.clone());
        }
        drop(guard);

        if self.auto_register {
            let cfg = TenantConfig {
                tenant_id: tenant_id.to_owned(),
                ..Default::default()
            };
            self.configs.write().unwrap().insert(tenant_id.to_owned(), cfg.clone());
            Ok(cfg)
        } else {
            Err(TraceError::TenantNotFound(tenant_id.to_owned()))
        }
    }

    pub fn list(&self) -> Vec<String> {
        self.configs.read().unwrap().keys().cloned().collect()
    }

    /// Validate that `requester` may query `target` tenant.
    pub fn check_access(&self, requester: &str, target: &str) -> Result<()> {
        if requester == target {
            return Ok(());
        }
        let guard = self.configs.read().unwrap();
        let cfg = guard
            .get(requester)
            .ok_or_else(|| TraceError::TenantNotFound(requester.to_owned()))?;
        if cfg.cross_tenant_query {
            Ok(())
        } else {
            Err(TraceError::TenantNotFound(format!(
                "tenant '{}' cannot query tenant '{}'", requester, target
            )))
        }
    }
}

// ─── Helpers for axum extractors ──────────────────────────────────────────

/// Extract the tenant ID from an `axum::http::HeaderMap`, falling back to "default".
pub fn tenant_from_headers(headers: &axum::http::HeaderMap) -> String {
    headers
        .get(TENANT_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_TENANT.into())
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_register_unknown_tenant() {
        let reg = TenantRegistry::new(true);
        let cfg = reg.get("new-tenant").unwrap();
        assert_eq!(cfg.tenant_id, "new-tenant");
        assert!(reg.list().contains(&"new-tenant".to_owned()));
    }

    #[test]
    fn reject_unknown_tenant_when_auto_register_off() {
        let reg = TenantRegistry::new(false);
        assert!(reg.get("ghost").is_err());
    }

    #[test]
    fn cross_tenant_access_denied_by_default() {
        let reg = TenantRegistry::new(true);
        reg.get("a").unwrap();
        reg.get("b").unwrap();
        assert!(reg.check_access("a", "b").is_err());
    }

    #[test]
    fn cross_tenant_access_granted_when_enabled() {
        let reg = TenantRegistry::new(true);
        reg.register(TenantConfig {
            tenant_id: "admin".into(),
            cross_tenant_query: true,
            ..Default::default()
        });
        reg.get("other").unwrap();
        assert!(reg.check_access("admin", "other").is_ok());
    }
}
