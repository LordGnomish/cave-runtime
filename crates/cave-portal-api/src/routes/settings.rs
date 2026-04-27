//! Tenant settings — key/value store for portal configuration.
//!
//! Settings are *namespaced* per tenant. Operators can read all tenants
//! (read-only support view); admins can write any tenant; tenant-persona
//! users can read+write their own tenant.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::routes::rbac::{Guard, GuardError, Principal};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setting {
    pub tenant: String,
    pub key: String,
    pub value: String,
    pub modified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutSettingRequest {
    pub tenant: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("guard: {0}")]
    Guard(#[from] GuardError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid key: {0}")]
    InvalidKey(String),
    #[error("value too large: {0} bytes (max {1})")]
    TooLarge(usize, usize),
    #[error("forbidden key: {0}")]
    ForbiddenKey(String),
}

const MAX_VALUE_SIZE: usize = 16 * 1024;

/// Keys whose values may not be set by the tenant persona — only admins.
/// Plumbed here so RBAC stays in one place.
const ADMIN_ONLY_KEYS: &[&str] = &[
    "billing.plan",
    "billing.stripe_customer_id",
    "feature.flags",
    "compliance.tier",
    "limits.max_namespaces",
];

pub struct SettingsStore {
    inner: Mutex<HashMap<(String, String), Setting>>,
}

impl Default for SettingsStore {
    fn default() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }
}

impl SettingsStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn validate_key(key: &str) -> Result<(), SettingsError> {
        if key.is_empty() {
            return Err(SettingsError::InvalidKey("empty".into()));
        }
        if key.len() > 128 {
            return Err(SettingsError::InvalidKey("too long".into()));
        }
        for ch in key.chars() {
            let ok = ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-';
            if !ok {
                return Err(SettingsError::InvalidKey(format!("char {:?}", ch)));
            }
        }
        Ok(())
    }

    pub fn list(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
    ) -> Result<Vec<Setting>, SettingsError> {
        Guard::cross_persona(None).authorize(principal, Some(tenant))?;
        let g = self.inner.lock().unwrap();
        let mut out: Vec<Setting> = g
            .iter()
            .filter(|((t, _), _)| t == tenant)
            .map(|(_, s)| s.clone())
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    pub fn get(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        key: &str,
    ) -> Result<Setting, SettingsError> {
        Guard::cross_persona(None).authorize(principal, Some(tenant))?;
        let g = self.inner.lock().unwrap();
        g.get(&(tenant.into(), key.into()))
            .cloned()
            .ok_or_else(|| SettingsError::NotFound(key.into()))
    }

    pub fn put(
        &self,
        principal: Option<&Principal>,
        req: PutSettingRequest,
    ) -> Result<Setting, SettingsError> {
        Self::validate_key(&req.key)?;
        let admin_only = ADMIN_ONLY_KEYS.iter().any(|k| *k == req.key);
        if admin_only {
            Guard::admin_only().authorize(principal, None)?;
        } else {
            Guard::tenant_only(Some("settings:write"))
                .authorize(principal, Some(&req.tenant))?;
        }
        if req.value.len() > MAX_VALUE_SIZE {
            return Err(SettingsError::TooLarge(req.value.len(), MAX_VALUE_SIZE));
        }
        let key = (req.tenant.clone(), req.key.clone());
        let setting = Setting {
            tenant: req.tenant,
            key: req.key,
            value: req.value,
            modified_at: "1970-01-01T00:00:00Z".into(),
        };
        self.inner.lock().unwrap().insert(key, setting.clone());
        Ok(setting)
    }

    pub fn delete(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        key: &str,
    ) -> Result<(), SettingsError> {
        let admin_only = ADMIN_ONLY_KEYS.iter().any(|k| *k == key);
        if admin_only {
            Guard::admin_only().authorize(principal, None)?;
        } else {
            Guard::tenant_only(Some("settings:write"))
                .authorize(principal, Some(tenant))?;
        }
        self.inner
            .lock()
            .unwrap()
            .remove(&(tenant.into(), key.into()))
            .ok_or_else(|| SettingsError::NotFound(key.into()))?;
        Ok(())
    }

    pub fn is_admin_only_key(key: &str) -> bool {
        ADMIN_ONLY_KEYS.contains(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::rbac::Persona;

    fn admin() -> Principal {
        Principal::new("a", Persona::Admin).with_role("settings:write")
    }
    fn dev(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant).with_tenant(t).with_role("settings:write")
    }
    fn dev_no_role(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant).with_tenant(t)
    }
    fn op() -> Principal { Principal::new("o", Persona::Operator) }

    fn put_req(t: &str, k: &str, v: &str) -> PutSettingRequest {
        PutSettingRequest { tenant: t.into(), key: k.into(), value: v.into() }
    }

    #[test]
    fn put_anonymous_denied() {
        let s = SettingsStore::new();
        let err = s.put(None, put_req("acme", "ui.theme", "dark")).unwrap_err();
        assert!(matches!(err, SettingsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn put_dev_can_set_normal_key() {
        let s = SettingsStore::new();
        let v = s.put(Some(&dev("acme")), put_req("acme", "ui.theme", "dark")).unwrap();
        assert_eq!(v.value, "dark");
    }

    #[test]
    fn put_dev_cannot_set_admin_only_key() {
        let s = SettingsStore::new();
        let err = s.put(Some(&dev("acme")), put_req("acme", "billing.plan", "enterprise")).unwrap_err();
        assert!(matches!(err, SettingsError::Guard(GuardError::PersonaForbidden { .. })));
    }

    #[test]
    fn put_admin_can_set_admin_only_key() {
        let s = SettingsStore::new();
        let v = s.put(Some(&admin()), put_req("acme", "billing.plan", "enterprise")).unwrap();
        assert_eq!(v.value, "enterprise");
    }

    #[test]
    fn put_dev_cross_tenant_denied() {
        let s = SettingsStore::new();
        let err = s.put(Some(&dev("globex")), put_req("acme", "ui.theme", "x")).unwrap_err();
        assert!(matches!(err, SettingsError::Guard(GuardError::TenantMismatch { .. })));
    }

    #[test]
    fn put_dev_no_role_denied() {
        let s = SettingsStore::new();
        let err = s.put(Some(&dev_no_role("acme")), put_req("acme", "ui.theme", "x")).unwrap_err();
        assert!(matches!(err, SettingsError::Guard(GuardError::MissingRole(_))));
    }

    #[test]
    fn put_invalid_key_empty() {
        let s = SettingsStore::new();
        let err = s.put(Some(&dev("acme")), put_req("acme", "", "v")).unwrap_err();
        assert!(matches!(err, SettingsError::InvalidKey(_)));
    }

    #[test]
    fn put_invalid_key_bad_char() {
        let s = SettingsStore::new();
        let err = s.put(Some(&dev("acme")), put_req("acme", "k k", "v")).unwrap_err();
        assert!(matches!(err, SettingsError::InvalidKey(_)));
    }

    #[test]
    fn put_invalid_key_too_long() {
        let s = SettingsStore::new();
        let k = "a".repeat(129);
        let err = s.put(Some(&dev("acme")), put_req("acme", &k, "v")).unwrap_err();
        assert!(matches!(err, SettingsError::InvalidKey(_)));
    }

    #[test]
    fn put_too_large_rejected() {
        let s = SettingsStore::new();
        let v = "x".repeat(MAX_VALUE_SIZE + 1);
        let err = s.put(Some(&dev("acme")), put_req("acme", "k", &v)).unwrap_err();
        assert!(matches!(err, SettingsError::TooLarge(_, _)));
    }

    #[test]
    fn put_overwrite_replaces_value() {
        let s = SettingsStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k", "v1")).unwrap();
        s.put(Some(&dev("acme")), put_req("acme", "k", "v2")).unwrap();
        let v = s.get(Some(&dev("acme")), "acme", "k").unwrap();
        assert_eq!(v.value, "v2");
    }

    #[test]
    fn get_returns_setting() {
        let s = SettingsStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k", "v")).unwrap();
        let v = s.get(Some(&dev("acme")), "acme", "k").unwrap();
        assert_eq!(v.value, "v");
    }

    #[test]
    fn get_not_found() {
        let s = SettingsStore::new();
        let err = s.get(Some(&dev("acme")), "acme", "ghost").unwrap_err();
        assert!(matches!(err, SettingsError::NotFound(_)));
    }

    #[test]
    fn get_operator_can_read_any_tenant() {
        let s = SettingsStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k", "v")).unwrap();
        let v = s.get(Some(&op()), "acme", "k").unwrap();
        assert_eq!(v.value, "v");
    }

    #[test]
    fn list_filters_by_tenant() {
        let s = SettingsStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k1", "v")).unwrap();
        s.put(Some(&dev("globex")), put_req("globex", "k2", "v")).unwrap();
        let acme = s.list(Some(&dev("acme")), "acme").unwrap();
        assert_eq!(acme.len(), 1);
        assert_eq!(acme[0].key, "k1");
    }

    #[test]
    fn list_sorted() {
        let s = SettingsStore::new();
        for k in ["zeta", "alpha", "mu"] {
            s.put(Some(&dev("acme")), put_req("acme", k, "v")).unwrap();
        }
        let keys: Vec<String> = s.list(Some(&dev("acme")), "acme").unwrap().into_iter().map(|s| s.key).collect();
        assert_eq!(keys, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn delete_dev_normal_key() {
        let s = SettingsStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "ui.theme", "x")).unwrap();
        s.delete(Some(&dev("acme")), "acme", "ui.theme").unwrap();
        assert!(s.get(Some(&dev("acme")), "acme", "ui.theme").is_err());
    }

    #[test]
    fn delete_dev_admin_only_denied() {
        let s = SettingsStore::new();
        s.put(Some(&admin()), put_req("acme", "billing.plan", "free")).unwrap();
        let err = s.delete(Some(&dev("acme")), "acme", "billing.plan").unwrap_err();
        assert!(matches!(err, SettingsError::Guard(GuardError::PersonaForbidden { .. })));
    }

    #[test]
    fn delete_admin_admin_only_ok() {
        let s = SettingsStore::new();
        s.put(Some(&admin()), put_req("acme", "billing.plan", "free")).unwrap();
        s.delete(Some(&admin()), "acme", "billing.plan").unwrap();
    }

    #[test]
    fn delete_unknown_not_found() {
        let s = SettingsStore::new();
        let err = s.delete(Some(&dev("acme")), "acme", "ghost").unwrap_err();
        assert!(matches!(err, SettingsError::NotFound(_)));
    }

    #[test]
    fn admin_only_keys_listed() {
        assert!(SettingsStore::is_admin_only_key("billing.plan"));
        assert!(SettingsStore::is_admin_only_key("compliance.tier"));
        assert!(!SettingsStore::is_admin_only_key("ui.theme"));
    }
}
