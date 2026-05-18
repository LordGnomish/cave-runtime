// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tenants CRUD endpoint state.
//!
//! All operations are RBAC-gated via [`crate::routes::rbac::Guard`]. The store
//! is in-memory for now — the production wiring will plug a `cave-db` backend
//! against the same trait surface.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::routes::rbac::{Guard, GuardError, Principal};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenant {
    pub id: String,
    pub display_name: String,
    pub plan: String,
    pub status: TenantStatus,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStatus {
    Active,
    Suspended,
    PendingDelete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTenantRequest {
    pub id: String,
    pub display_name: String,
    pub plan: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateTenantRequest {
    pub display_name: Option<String>,
    pub plan: Option<String>,
    pub status: Option<TenantStatus>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TenantsError {
    #[error("guard: {0}")]
    Guard(#[from] GuardError),
    #[error("tenant {0:?} already exists")]
    AlreadyExists(String),
    #[error("tenant {0:?} not found")]
    NotFound(String),
    #[error("invalid id: {0}")]
    InvalidId(String),
}

pub struct TenantStore {
    inner: Mutex<HashMap<String, Tenant>>,
}

impl Default for TenantStore {
    fn default() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }
}

impl TenantStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn validate_id(id: &str) -> Result<(), TenantsError> {
        if id.is_empty() {
            return Err(TenantsError::InvalidId("empty".into()));
        }
        if id.len() > 64 {
            return Err(TenantsError::InvalidId("too long".into()));
        }
        for ch in id.chars() {
            if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_') {
                return Err(TenantsError::InvalidId(format!("char {:?}", ch)));
            }
        }
        Ok(())
    }

    pub fn list(&self, principal: Option<&Principal>) -> Result<Vec<Tenant>, TenantsError> {
        Guard::operator_only().authorize(principal, None)?;
        let guard = self.inner.lock().unwrap();
        let mut out: Vec<Tenant> = guard.values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    pub fn get(
        &self,
        principal: Option<&Principal>,
        id: &str,
    ) -> Result<Tenant, TenantsError> {
        Guard::cross_persona(None).authorize(principal, Some(id))?;
        let guard = self.inner.lock().unwrap();
        guard
            .get(id)
            .cloned()
            .ok_or_else(|| TenantsError::NotFound(id.to_string()))
    }

    pub fn create(
        &self,
        principal: Option<&Principal>,
        req: CreateTenantRequest,
    ) -> Result<Tenant, TenantsError> {
        Guard::admin_only().authorize(principal, None)?;
        Self::validate_id(&req.id)?;
        let mut guard = self.inner.lock().unwrap();
        if guard.contains_key(&req.id) {
            return Err(TenantsError::AlreadyExists(req.id));
        }
        let tenant = Tenant {
            id: req.id.clone(),
            display_name: req.display_name,
            plan: req.plan,
            status: TenantStatus::Active,
            created_at: "1970-01-01T00:00:00Z".into(),
        };
        guard.insert(req.id, tenant.clone());
        Ok(tenant)
    }

    pub fn update(
        &self,
        principal: Option<&Principal>,
        id: &str,
        req: UpdateTenantRequest,
    ) -> Result<Tenant, TenantsError> {
        Guard::admin_only().authorize(principal, None)?;
        let mut guard = self.inner.lock().unwrap();
        let tenant = guard
            .get_mut(id)
            .ok_or_else(|| TenantsError::NotFound(id.to_string()))?;
        if let Some(d) = req.display_name {
            tenant.display_name = d;
        }
        if let Some(p) = req.plan {
            tenant.plan = p;
        }
        if let Some(s) = req.status {
            tenant.status = s;
        }
        Ok(tenant.clone())
    }

    pub fn delete(
        &self,
        principal: Option<&Principal>,
        id: &str,
    ) -> Result<(), TenantsError> {
        Guard::admin_only().authorize(principal, None)?;
        let mut guard = self.inner.lock().unwrap();
        guard
            .remove(id)
            .ok_or_else(|| TenantsError::NotFound(id.to_string()))?;
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::rbac::Persona;

    fn admin() -> Principal {
        Principal::new("a", Persona::Admin)
    }
    fn operator() -> Principal {
        Principal::new("o", Persona::Operator)
    }
    fn tenant_p(t: &str) -> Principal {
        Principal::new("u", Persona::Tenant).with_tenant(t)
    }

    fn make_create(id: &str) -> CreateTenantRequest {
        CreateTenantRequest {
            id: id.into(),
            display_name: id.to_uppercase(),
            plan: "free".into(),
        }
    }

    #[test]
    fn store_starts_empty() {
        let s = TenantStore::new();
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn create_requires_admin() {
        let s = TenantStore::new();
        let err = s.create(Some(&operator()), make_create("acme")).unwrap_err();
        assert!(matches!(err, TenantsError::Guard(GuardError::PersonaForbidden { .. })));
    }

    #[test]
    fn create_anonymous_denied() {
        let s = TenantStore::new();
        let err = s.create(None, make_create("acme")).unwrap_err();
        assert!(matches!(err, TenantsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn create_succeeds_for_admin() {
        let s = TenantStore::new();
        let t = s.create(Some(&admin()), make_create("acme")).unwrap();
        assert_eq!(t.id, "acme");
        assert_eq!(t.status, TenantStatus::Active);
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn create_rejects_duplicate() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        let err = s.create(Some(&admin()), make_create("acme")).unwrap_err();
        assert!(matches!(err, TenantsError::AlreadyExists(s) if s == "acme"));
    }

    #[test]
    fn create_rejects_invalid_id_empty() {
        let s = TenantStore::new();
        let err = s.create(Some(&admin()), make_create("")).unwrap_err();
        assert!(matches!(err, TenantsError::InvalidId(_)));
    }

    #[test]
    fn create_rejects_invalid_id_uppercase() {
        let s = TenantStore::new();
        let err = s.create(Some(&admin()), make_create("Acme")).unwrap_err();
        assert!(matches!(err, TenantsError::InvalidId(_)));
    }

    #[test]
    fn create_rejects_invalid_id_too_long() {
        let s = TenantStore::new();
        let id = "a".repeat(65);
        let err = s.create(Some(&admin()), make_create(&id)).unwrap_err();
        assert!(matches!(err, TenantsError::InvalidId(_)));
    }

    #[test]
    fn create_rejects_invalid_id_chars() {
        let s = TenantStore::new();
        let err = s.create(Some(&admin()), make_create("a/b")).unwrap_err();
        assert!(matches!(err, TenantsError::InvalidId(_)));
    }

    #[test]
    fn list_requires_operator_or_admin() {
        let s = TenantStore::new();
        let err = s.list(Some(&tenant_p("acme"))).unwrap_err();
        assert!(matches!(err, TenantsError::Guard(GuardError::PersonaForbidden { .. })));
    }

    #[test]
    fn list_anonymous_denied() {
        let s = TenantStore::new();
        let err = s.list(None).unwrap_err();
        assert!(matches!(err, TenantsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn list_returns_sorted() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("zenith")).unwrap();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        s.create(Some(&admin()), make_create("monolith")).unwrap();
        let ids: Vec<String> = s.list(Some(&operator())).unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["acme", "monolith", "zenith"]);
    }

    #[test]
    fn get_returns_tenant() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        let t = s.get(Some(&admin()), "acme").unwrap();
        assert_eq!(t.id, "acme");
    }

    #[test]
    fn get_not_found() {
        let s = TenantStore::new();
        let err = s.get(Some(&admin()), "ghost").unwrap_err();
        assert!(matches!(err, TenantsError::NotFound(s) if s == "ghost"));
    }

    #[test]
    fn get_tenant_persona_can_only_read_own_tenant() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        let err = s.get(Some(&tenant_p("globex")), "acme").unwrap_err();
        assert!(matches!(err, TenantsError::Guard(GuardError::TenantMismatch { .. })));
    }

    #[test]
    fn get_tenant_persona_can_read_self() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        let t = s.get(Some(&tenant_p("acme")), "acme").unwrap();
        assert_eq!(t.id, "acme");
    }

    #[test]
    fn update_requires_admin() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        let err = s.update(Some(&operator()), "acme", UpdateTenantRequest {
            display_name: Some("X".into()),
            plan: None,
            status: None,
        }).unwrap_err();
        assert!(matches!(err, TenantsError::Guard(_)));
    }

    #[test]
    fn update_changes_fields() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        let upd = s.update(Some(&admin()), "acme", UpdateTenantRequest {
            display_name: Some("Acme Inc".into()),
            plan: Some("enterprise".into()),
            status: Some(TenantStatus::Suspended),
        }).unwrap();
        assert_eq!(upd.display_name, "Acme Inc");
        assert_eq!(upd.plan, "enterprise");
        assert_eq!(upd.status, TenantStatus::Suspended);
    }

    #[test]
    fn update_partial_keeps_others() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        let upd = s.update(Some(&admin()), "acme", UpdateTenantRequest {
            display_name: None,
            plan: Some("pro".into()),
            status: None,
        }).unwrap();
        assert_eq!(upd.display_name, "ACME"); // unchanged
        assert_eq!(upd.plan, "pro");
        assert_eq!(upd.status, TenantStatus::Active);
    }

    #[test]
    fn update_unknown_returns_not_found() {
        let s = TenantStore::new();
        let err = s.update(Some(&admin()), "ghost", UpdateTenantRequest {
            display_name: Some("X".into()),
            plan: None,
            status: None,
        }).unwrap_err();
        assert!(matches!(err, TenantsError::NotFound(_)));
    }

    #[test]
    fn delete_requires_admin() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        let err = s.delete(Some(&operator()), "acme").unwrap_err();
        assert!(matches!(err, TenantsError::Guard(_)));
    }

    #[test]
    fn delete_removes_tenant() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("acme")).unwrap();
        s.delete(Some(&admin()), "acme").unwrap();
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn delete_unknown_not_found() {
        let s = TenantStore::new();
        let err = s.delete(Some(&admin()), "ghost").unwrap_err();
        assert!(matches!(err, TenantsError::NotFound(_)));
    }

    #[test]
    fn tenant_status_serializes_snake_case() {
        let s = serde_json::to_string(&TenantStatus::PendingDelete).unwrap();
        assert_eq!(s, "\"pending_delete\"");
    }

    #[test]
    fn tenant_status_deserializes_snake_case() {
        let v: TenantStatus = serde_json::from_str("\"pending_delete\"").unwrap();
        assert_eq!(v, TenantStatus::PendingDelete);
    }

    #[test]
    fn tenant_round_trips_json() {
        let t = Tenant {
            id: "acme".into(),
            display_name: "Acme".into(),
            plan: "free".into(),
            status: TenantStatus::Active,
            created_at: "1970-01-01T00:00:00Z".into(),
        };
        let s = serde_json::to_string(&t).unwrap();
        let back: Tenant = serde_json::from_str(&s).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn create_tenant_request_round_trips() {
        let r = CreateTenantRequest {
            id: "acme".into(),
            display_name: "Acme".into(),
            plan: "free".into(),
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: CreateTenantRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn store_count_tracks_creates_and_deletes() {
        let s = TenantStore::new();
        s.create(Some(&admin()), make_create("a")).unwrap();
        s.create(Some(&admin()), make_create("b")).unwrap();
        assert_eq!(s.count(), 2);
        s.delete(Some(&admin()), "a").unwrap();
        assert_eq!(s.count(), 1);
    }
}
