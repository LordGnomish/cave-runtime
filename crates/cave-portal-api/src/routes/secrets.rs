// SPDX-License-Identifier: AGPL-3.0-or-later
//! Secrets — list/get/put. Tenant-scoped, default-deny.
//!
//! The `value` of a secret is **never** returned by `list`; only the metadata
//! (name, version, modified_at). Reading the value requires
//! `secrets:read`; writing requires `secrets:write`. Cross-tenant access by
//! the tenant persona is rejected at the guard.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::routes::rbac::{Guard, GuardError, Principal};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMeta {
    pub tenant: String,
    pub name: String,
    pub version: u64,
    pub modified_at: String,
    pub byte_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretValue {
    pub meta: SecretMeta,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutSecretRequest {
    pub tenant: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretsError {
    #[error("guard: {0}")]
    Guard(#[from] GuardError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid name: {0}")]
    InvalidName(String),
    #[error("value too large: {0} bytes (max {1})")]
    TooLarge(usize, usize),
}

const MAX_SECRET_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone)]
struct Stored {
    meta: SecretMeta,
    value: String,
}

pub struct SecretStore {
    inner: Mutex<HashMap<(String, String), Stored>>,
}

impl Default for SecretStore {
    fn default() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }
}

impl SecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn validate_name(name: &str) -> Result<(), SecretsError> {
        if name.is_empty() {
            return Err(SecretsError::InvalidName("empty".into()));
        }
        if name.len() > 128 {
            return Err(SecretsError::InvalidName("too long".into()));
        }
        for ch in name.chars() {
            let ok = ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' || ch == '/';
            if !ok {
                return Err(SecretsError::InvalidName(format!("char {:?}", ch)));
            }
        }
        Ok(())
    }

    pub fn list(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
    ) -> Result<Vec<SecretMeta>, SecretsError> {
        Guard::tenant_only(Some("secrets:read")).authorize(principal, Some(tenant))?;
        let guard = self.inner.lock().unwrap();
        let mut out: Vec<SecretMeta> = guard
            .iter()
            .filter(|((t, _), _)| t == tenant)
            .map(|(_, s)| s.meta.clone())
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn get(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        name: &str,
    ) -> Result<SecretValue, SecretsError> {
        Guard::tenant_only(Some("secrets:read")).authorize(principal, Some(tenant))?;
        let guard = self.inner.lock().unwrap();
        let stored = guard
            .get(&(tenant.into(), name.into()))
            .ok_or_else(|| SecretsError::NotFound(name.into()))?;
        Ok(SecretValue { meta: stored.meta.clone(), value: stored.value.clone() })
    }

    pub fn put(
        &self,
        principal: Option<&Principal>,
        req: PutSecretRequest,
    ) -> Result<SecretMeta, SecretsError> {
        Guard::tenant_only(Some("secrets:write"))
            .authorize(principal, Some(&req.tenant))?;
        Self::validate_name(&req.name)?;
        if req.value.len() > MAX_SECRET_SIZE {
            return Err(SecretsError::TooLarge(req.value.len(), MAX_SECRET_SIZE));
        }
        let key = (req.tenant.clone(), req.name.clone());
        let mut guard = self.inner.lock().unwrap();
        let next_version = guard.get(&key).map(|s| s.meta.version + 1).unwrap_or(1);
        let meta = SecretMeta {
            tenant: req.tenant,
            name: req.name,
            version: next_version,
            modified_at: "1970-01-01T00:00:00Z".into(),
            byte_size: req.value.len(),
        };
        guard.insert(
            key,
            Stored { meta: meta.clone(), value: req.value },
        );
        Ok(meta)
    }

    pub fn delete(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        name: &str,
    ) -> Result<(), SecretsError> {
        Guard::tenant_only(Some("secrets:write")).authorize(principal, Some(tenant))?;
        let mut guard = self.inner.lock().unwrap();
        guard
            .remove(&(tenant.into(), name.into()))
            .ok_or_else(|| SecretsError::NotFound(name.into()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::rbac::Persona;

    fn dev(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant)
            .with_tenant(t)
            .with_role("secrets:read")
            .with_role("secrets:write")
    }
    fn dev_read(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant).with_tenant(t).with_role("secrets:read")
    }
    fn dev_no_role(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant).with_tenant(t)
    }
    fn admin() -> Principal {
        Principal::new("a", Persona::Admin).with_role("secrets:read").with_role("secrets:write")
    }

    fn put_req(t: &str, n: &str, v: &str) -> PutSecretRequest {
        PutSecretRequest { tenant: t.into(), name: n.into(), value: v.into() }
    }

    #[test]
    fn put_anonymous_denied() {
        let s = SecretStore::new();
        let err = s.put(None, put_req("acme", "x", "v")).unwrap_err();
        assert!(matches!(err, SecretsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn put_without_role_denied() {
        let s = SecretStore::new();
        let err = s.put(Some(&dev_no_role("acme")), put_req("acme", "x", "v")).unwrap_err();
        assert!(matches!(err, SecretsError::Guard(GuardError::MissingRole(_))));
    }

    #[test]
    fn put_read_role_cannot_write() {
        let s = SecretStore::new();
        let err = s.put(Some(&dev_read("acme")), put_req("acme", "x", "v")).unwrap_err();
        assert!(matches!(err, SecretsError::Guard(GuardError::MissingRole(r)) if r == "secrets:write"));
    }

    #[test]
    fn put_succeeds_for_dev_with_write() {
        let s = SecretStore::new();
        let m = s.put(Some(&dev("acme")), put_req("acme", "api-key", "abc")).unwrap();
        assert_eq!(m.version, 1);
        assert_eq!(m.byte_size, 3);
    }

    #[test]
    fn put_increments_version() {
        let s = SecretStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k", "v1")).unwrap();
        let m2 = s.put(Some(&dev("acme")), put_req("acme", "k", "v2-bigger")).unwrap();
        assert_eq!(m2.version, 2);
    }

    #[test]
    fn put_cross_tenant_denied() {
        let s = SecretStore::new();
        let err = s.put(Some(&dev("globex")), put_req("acme", "k", "v")).unwrap_err();
        assert!(matches!(err, SecretsError::Guard(GuardError::TenantMismatch { .. })));
    }

    #[test]
    fn put_admin_can_cross_tenant() {
        let s = SecretStore::new();
        assert!(s.put(Some(&admin()), put_req("globex", "k", "v")).is_ok());
        assert!(s.put(Some(&admin()), put_req("acme", "k", "v")).is_ok());
    }

    #[test]
    fn put_invalid_name_empty() {
        let s = SecretStore::new();
        let err = s.put(Some(&dev("acme")), put_req("acme", "", "v")).unwrap_err();
        assert!(matches!(err, SecretsError::InvalidName(_)));
    }

    #[test]
    fn put_invalid_name_too_long() {
        let s = SecretStore::new();
        let n = "a".repeat(129);
        let err = s.put(Some(&dev("acme")), put_req("acme", &n, "v")).unwrap_err();
        assert!(matches!(err, SecretsError::InvalidName(_)));
    }

    #[test]
    fn put_invalid_name_bad_char() {
        let s = SecretStore::new();
        let err = s.put(Some(&dev("acme")), put_req("acme", "key with space", "v")).unwrap_err();
        assert!(matches!(err, SecretsError::InvalidName(_)));
    }

    #[test]
    fn put_accepts_path_like_name() {
        let s = SecretStore::new();
        assert!(s.put(Some(&dev("acme")), put_req("acme", "db/prod/password", "v")).is_ok());
    }

    #[test]
    fn put_too_large_rejected() {
        let s = SecretStore::new();
        let big = "x".repeat(MAX_SECRET_SIZE + 1);
        let err = s.put(Some(&dev("acme")), put_req("acme", "big", &big)).unwrap_err();
        assert!(matches!(err, SecretsError::TooLarge(_, _)));
    }

    #[test]
    fn put_at_size_limit_accepted() {
        let s = SecretStore::new();
        let v = "x".repeat(MAX_SECRET_SIZE);
        let m = s.put(Some(&dev("acme")), put_req("acme", "big", &v)).unwrap();
        assert_eq!(m.byte_size, MAX_SECRET_SIZE);
    }

    #[test]
    fn get_returns_value() {
        let s = SecretStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k", "secret-v")).unwrap();
        let v = s.get(Some(&dev_read("acme")), "acme", "k").unwrap();
        assert_eq!(v.value, "secret-v");
    }

    #[test]
    fn get_not_found() {
        let s = SecretStore::new();
        let err = s.get(Some(&dev("acme")), "acme", "ghost").unwrap_err();
        assert!(matches!(err, SecretsError::NotFound(_)));
    }

    #[test]
    fn get_anonymous_denied() {
        let s = SecretStore::new();
        let err = s.get(None, "acme", "k").unwrap_err();
        assert!(matches!(err, SecretsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn get_cross_tenant_denied_for_dev() {
        let s = SecretStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k", "v")).unwrap();
        let err = s.get(Some(&dev("globex")), "acme", "k").unwrap_err();
        assert!(matches!(err, SecretsError::Guard(GuardError::TenantMismatch { .. })));
    }

    #[test]
    fn list_does_not_leak_value() {
        let s = SecretStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k", "very-secret")).unwrap();
        let metas = s.list(Some(&dev("acme")), "acme").unwrap();
        let json = serde_json::to_string(&metas).unwrap();
        assert!(!json.contains("very-secret"));
    }

    #[test]
    fn list_filters_by_tenant() {
        let s = SecretStore::new();
        s.put(Some(&admin()), put_req("acme", "a", "v")).unwrap();
        s.put(Some(&admin()), put_req("globex", "b", "v")).unwrap();
        let acme = s.list(Some(&admin()), "acme").unwrap();
        assert_eq!(acme.len(), 1);
        assert_eq!(acme[0].name, "a");
    }

    #[test]
    fn list_sorted() {
        let s = SecretStore::new();
        for n in ["zeta", "alpha", "mu"] {
            s.put(Some(&dev("acme")), put_req("acme", n, "v")).unwrap();
        }
        let names: Vec<String> = s.list(Some(&dev("acme")), "acme").unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn delete_removes() {
        let s = SecretStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k", "v")).unwrap();
        s.delete(Some(&dev("acme")), "acme", "k").unwrap();
        assert!(s.get(Some(&dev("acme")), "acme", "k").is_err());
    }

    #[test]
    fn delete_unknown_not_found() {
        let s = SecretStore::new();
        let err = s.delete(Some(&dev("acme")), "acme", "ghost").unwrap_err();
        assert!(matches!(err, SecretsError::NotFound(_)));
    }

    #[test]
    fn delete_requires_write() {
        let s = SecretStore::new();
        s.put(Some(&dev("acme")), put_req("acme", "k", "v")).unwrap();
        let err = s.delete(Some(&dev_read("acme")), "acme", "k").unwrap_err();
        assert!(matches!(err, SecretsError::Guard(GuardError::MissingRole(r)) if r == "secrets:write"));
    }
}
