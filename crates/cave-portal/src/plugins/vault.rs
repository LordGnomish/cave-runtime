// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vault wrap — native secret-engine UI.
//!
//! Replaces the OpenBao / Vault web console. Tenants list and read their own
//! secret engines through cave-portal-api; writes happen through this view's
//! native form. **No** redirect to Vault's UI exists.

use super::ViewPersona;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    KvV1,
    KvV2,
    Pki,
    Database,
    Transit,
    Aws,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretEngine {
    pub mount: String,
    pub kind: EngineKind,
    pub tenant: String,
    pub description: String,
    pub default_lease_ttl_secs: u64,
    pub max_lease_ttl_secs: u64,
    pub sealed: bool,
}

impl SecretEngine {
    pub fn new(mount: impl Into<String>, kind: EngineKind, tenant: impl Into<String>) -> Self {
        Self {
            mount: mount.into(),
            kind,
            tenant: tenant.into(),
            description: String::new(),
            default_lease_ttl_secs: 3600,
            max_lease_ttl_secs: 86400,
            sealed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretEntry {
    pub mount: String,
    pub path: String,
    pub version: u64,
    pub deleted: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VaultError {
    #[error("engine {0:?} sealed")]
    Sealed(String),
    #[error("path traversal blocked: {0:?}")]
    PathTraversal(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("engine {0:?} not found")]
    NotFound(String),
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("entry already deleted")]
    AlreadyDeleted,
}

pub fn validate_path(path: &str) -> Result<(), VaultError> {
    if path.is_empty() {
        return Err(VaultError::InvalidPath("empty".into()));
    }
    if path.len() > 256 {
        return Err(VaultError::InvalidPath("too long".into()));
    }
    if path.contains("..") {
        return Err(VaultError::PathTraversal(path.into()));
    }
    if path.starts_with('/') || path.contains("//") {
        return Err(VaultError::InvalidPath(path.into()));
    }
    for ch in path.chars() {
        let ok = ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' || ch == '.';
        if !ok {
            return Err(VaultError::InvalidPath(format!("char {:?}", ch)));
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct VaultPlugin {
    engines: Vec<SecretEngine>,
    entries: Vec<SecretEntry>,
}

impl VaultPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mount(&mut self, e: SecretEngine) {
        if let Some(idx) = self
            .engines
            .iter()
            .position(|x| x.mount == e.mount && x.tenant == e.tenant)
        {
            self.engines[idx] = e;
        } else {
            self.engines.push(e);
        }
    }

    pub fn unmount(&mut self, tenant: &str, mount: &str) -> Result<(), VaultError> {
        let idx = self
            .engines
            .iter()
            .position(|e| e.tenant == tenant && e.mount == mount)
            .ok_or_else(|| VaultError::NotFound(mount.into()))?;
        self.engines.remove(idx);
        self.entries.retain(|x| !(x.mount == mount));
        Ok(())
    }

    pub fn engine(&self, tenant: &str, mount: &str) -> Option<&SecretEngine> {
        self.engines.iter().find(|e| e.tenant == tenant && e.mount == mount)
    }

    pub fn list_engines(&self, tenant: &str) -> Vec<&SecretEngine> {
        let mut out: Vec<&SecretEngine> = self.engines.iter().filter(|e| e.tenant == tenant).collect();
        out.sort_by(|a, b| a.mount.cmp(&b.mount));
        out
    }

    pub fn put(
        &mut self,
        persona: ViewPersona,
        tenant: &str,
        mount: &str,
        path: &str,
    ) -> Result<&SecretEntry, VaultError> {
        if !matches!(persona, ViewPersona::Tenant | ViewPersona::Admin) {
            return Err(VaultError::Forbidden("operator cannot write secrets"));
        }
        validate_path(path)?;
        let engine = self
            .engines
            .iter()
            .find(|e| e.tenant == tenant && e.mount == mount)
            .ok_or_else(|| VaultError::NotFound(mount.into()))?;
        if engine.sealed {
            return Err(VaultError::Sealed(mount.into()));
        }
        let next_version = self
            .entries
            .iter()
            .filter(|x| x.mount == mount && x.path == path)
            .map(|x| x.version)
            .max()
            .unwrap_or(0)
            + 1;
        self.entries.push(SecretEntry {
            mount: mount.into(),
            path: path.into(),
            version: next_version,
            deleted: false,
        });
        Ok(self.entries.last().unwrap())
    }

    pub fn list_entries(
        &self,
        persona: ViewPersona,
        tenant: &str,
        mount: &str,
    ) -> Result<Vec<&SecretEntry>, VaultError> {
        let _ = persona; // tenant + admin can list; operator audit-list also OK
        let _ = self
            .engines
            .iter()
            .find(|e| e.tenant == tenant && e.mount == mount)
            .ok_or_else(|| VaultError::NotFound(mount.into()))?;
        let mut out: Vec<&SecretEntry> =
            self.entries.iter().filter(|x| x.mount == mount).collect();
        out.sort_by(|a, b| a.path.cmp(&b.path).then(a.version.cmp(&b.version)));
        Ok(out)
    }

    pub fn delete(
        &mut self,
        persona: ViewPersona,
        tenant: &str,
        mount: &str,
        path: &str,
    ) -> Result<(), VaultError> {
        if !matches!(persona, ViewPersona::Tenant | ViewPersona::Admin) {
            return Err(VaultError::Forbidden("operator cannot delete"));
        }
        let engine = self
            .engines
            .iter()
            .find(|e| e.tenant == tenant && e.mount == mount)
            .ok_or_else(|| VaultError::NotFound(mount.into()))?;
        if engine.sealed {
            return Err(VaultError::Sealed(mount.into()));
        }
        let mut found = false;
        for entry in self.entries.iter_mut() {
            if entry.mount == mount && entry.path == path && !entry.deleted {
                entry.deleted = true;
                found = true;
            }
        }
        if !found {
            return Err(VaultError::AlreadyDeleted);
        }
        Ok(())
    }

    pub fn seal(&mut self, tenant: &str, mount: &str) -> Result<(), VaultError> {
        let engine = self
            .engines
            .iter_mut()
            .find(|e| e.tenant == tenant && e.mount == mount)
            .ok_or_else(|| VaultError::NotFound(mount.into()))?;
        engine.sealed = true;
        Ok(())
    }

    pub fn unseal(&mut self, tenant: &str, mount: &str) -> Result<(), VaultError> {
        let engine = self
            .engines
            .iter_mut()
            .find(|e| e.tenant == tenant && e.mount == mount)
            .ok_or_else(|| VaultError::NotFound(mount.into()))?;
        engine.sealed = false;
        Ok(())
    }

    pub fn count_engines(&self) -> usize {
        self.engines.len()
    }

    pub fn count_entries(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(mount: &str, tenant: &str) -> SecretEngine {
        SecretEngine::new(mount, EngineKind::KvV2, tenant)
    }

    #[test]
    fn validate_path_accepts_normal() {
        assert!(validate_path("app/db/password").is_ok());
        assert!(validate_path("simple").is_ok());
    }

    #[test]
    fn validate_path_rejects_empty() {
        let err = validate_path("").unwrap_err();
        assert!(matches!(err, VaultError::InvalidPath(_)));
    }

    #[test]
    fn validate_path_rejects_too_long() {
        let p = "a".repeat(257);
        let err = validate_path(&p).unwrap_err();
        assert!(matches!(err, VaultError::InvalidPath(_)));
    }

    #[test]
    fn validate_path_rejects_dot_dot() {
        let err = validate_path("a/../etc").unwrap_err();
        assert!(matches!(err, VaultError::PathTraversal(_)));
    }

    #[test]
    fn validate_path_rejects_leading_slash() {
        let err = validate_path("/abs").unwrap_err();
        assert!(matches!(err, VaultError::InvalidPath(_)));
    }

    #[test]
    fn validate_path_rejects_double_slash() {
        let err = validate_path("a//b").unwrap_err();
        assert!(matches!(err, VaultError::InvalidPath(_)));
    }

    #[test]
    fn validate_path_rejects_bad_char() {
        let err = validate_path("a b").unwrap_err();
        assert!(matches!(err, VaultError::InvalidPath(_)));
    }

    #[test]
    fn mount_inserts_engine() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        assert_eq!(v.count_engines(), 1);
    }

    #[test]
    fn mount_replaces_same_mount_same_tenant() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        let mut e2 = engine("kv", "acme");
        e2.description = "updated".into();
        v.mount(e2);
        assert_eq!(v.count_engines(), 1);
        assert_eq!(v.engine("acme", "kv").unwrap().description, "updated");
    }

    #[test]
    fn mount_separate_tenants_dont_collide() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        v.mount(engine("kv", "globex"));
        assert_eq!(v.count_engines(), 2);
    }

    #[test]
    fn unmount_removes_engine_and_entries() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        v.put(ViewPersona::Tenant, "acme", "kv", "p").unwrap();
        v.unmount("acme", "kv").unwrap();
        assert_eq!(v.count_engines(), 0);
        assert_eq!(v.count_entries(), 0);
    }

    #[test]
    fn unmount_unknown_errors() {
        let mut v = VaultPlugin::new();
        let err = v.unmount("acme", "ghost").unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }

    #[test]
    fn put_creates_v1_then_v2() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        v.put(ViewPersona::Tenant, "acme", "kv", "p").unwrap();
        let e2 = v.put(ViewPersona::Tenant, "acme", "kv", "p").unwrap();
        assert_eq!(e2.version, 2);
    }

    #[test]
    fn put_separate_paths_each_v1() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        let a = v.put(ViewPersona::Tenant, "acme", "kv", "a").unwrap().clone();
        let b = v.put(ViewPersona::Tenant, "acme", "kv", "b").unwrap().clone();
        assert_eq!(a.version, 1);
        assert_eq!(b.version, 1);
    }

    #[test]
    fn put_operator_denied() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        let err = v.put(ViewPersona::Operator, "acme", "kv", "p").unwrap_err();
        assert!(matches!(err, VaultError::Forbidden(_)));
    }

    #[test]
    fn put_unknown_engine() {
        let mut v = VaultPlugin::new();
        let err = v.put(ViewPersona::Tenant, "acme", "kv", "p").unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }

    #[test]
    fn put_sealed_engine_rejected() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        v.seal("acme", "kv").unwrap();
        let err = v.put(ViewPersona::Tenant, "acme", "kv", "p").unwrap_err();
        assert!(matches!(err, VaultError::Sealed(_)));
    }

    #[test]
    fn put_path_traversal_blocked() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        let err = v.put(ViewPersona::Tenant, "acme", "kv", "a/../b").unwrap_err();
        assert!(matches!(err, VaultError::PathTraversal(_)));
    }

    #[test]
    fn list_entries_returns_sorted() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        for p in ["zeta", "alpha", "mu"] {
            v.put(ViewPersona::Tenant, "acme", "kv", p).unwrap();
        }
        let out = v.list_entries(ViewPersona::Tenant, "acme", "kv").unwrap();
        let paths: Vec<&str> = out.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn list_entries_unknown_engine_errors() {
        let v = VaultPlugin::new();
        let err = v.list_entries(ViewPersona::Tenant, "acme", "kv").unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }

    #[test]
    fn delete_marks_deleted() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        v.put(ViewPersona::Tenant, "acme", "kv", "p").unwrap();
        v.delete(ViewPersona::Tenant, "acme", "kv", "p").unwrap();
        let entries = v.list_entries(ViewPersona::Tenant, "acme", "kv").unwrap();
        assert!(entries.iter().any(|e| e.path == "p" && e.deleted));
    }

    #[test]
    fn delete_already_deleted_errors() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        v.put(ViewPersona::Tenant, "acme", "kv", "p").unwrap();
        v.delete(ViewPersona::Tenant, "acme", "kv", "p").unwrap();
        let err = v.delete(ViewPersona::Tenant, "acme", "kv", "p").unwrap_err();
        assert_eq!(err, VaultError::AlreadyDeleted);
    }

    #[test]
    fn delete_operator_denied() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        v.put(ViewPersona::Tenant, "acme", "kv", "p").unwrap();
        let err = v.delete(ViewPersona::Operator, "acme", "kv", "p").unwrap_err();
        assert!(matches!(err, VaultError::Forbidden(_)));
    }

    #[test]
    fn seal_blocks_writes() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        v.seal("acme", "kv").unwrap();
        assert!(v.put(ViewPersona::Tenant, "acme", "kv", "p").is_err());
    }

    #[test]
    fn unseal_restores_writes() {
        let mut v = VaultPlugin::new();
        v.mount(engine("kv", "acme"));
        v.seal("acme", "kv").unwrap();
        v.unseal("acme", "kv").unwrap();
        assert!(v.put(ViewPersona::Tenant, "acme", "kv", "p").is_ok());
    }

    #[test]
    fn list_engines_sorted_per_tenant() {
        let mut v = VaultPlugin::new();
        v.mount(SecretEngine::new("zeta", EngineKind::KvV2, "acme"));
        v.mount(SecretEngine::new("alpha", EngineKind::KvV1, "acme"));
        v.mount(SecretEngine::new("mu", EngineKind::Pki, "globex"));
        let acme: Vec<&str> = v.list_engines("acme").iter().map(|e| e.mount.as_str()).collect();
        assert_eq!(acme, vec!["alpha", "zeta"]);
    }

    #[test]
    fn engine_kind_serializes_snake_case() {
        let s = serde_json::to_string(&EngineKind::KvV2).unwrap();
        assert_eq!(s, "\"kv_v2\"");
    }
}
