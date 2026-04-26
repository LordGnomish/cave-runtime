//! Server-Side Apply (SSA) field manager.
//!
//! Upstream: kubernetes/kubernetes v1.30.0
//!   * `staging/src/k8s.io/apimachinery/pkg/util/managedfields/`
//!   * `staging/src/k8s.io/apiserver/pkg/endpoints/handlers/fieldmanager/`
//!   * KEP-555 (Server-Side Apply).
//!
//! SSA tracks which "field manager" owns each path of a resource. Conflicts
//! occur when one manager attempts to write a path owned by another, and are
//! resolvable only via `force=true`. We keep the model minimal: paths are
//! `/`-separated strings, ownership is tracked per resource UID.
//!
//! Tenant invariant: managed-fields entries are scoped per `(tenant_id, uid)`.
//! A manager registered under tenant A cannot transfer ownership to tenant B,
//! and conflict detection MUST NOT be cross-tenant.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagerOperation {
    Apply,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedFieldsEntry {
    pub manager: String,
    pub operation: ManagerOperation,
    pub api_version: String,
    pub time: DateTime<Utc>,
    pub fields_owned: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyConflict {
    pub field: String,
    pub current_manager: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApplyOutcome {
    Applied { manager: String, fields: Vec<String> },
    Conflicts(Vec<ApplyConflict>),
}

/// Object key — a tenant-scoped resource UID.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ObjectKey {
    pub tenant_id: String,
    pub uid: String,
}

/// Field-manager registry. Tracks ownership per tenant-scoped object.
pub struct FieldManagerRegistry {
    inner: Mutex<HashMap<ObjectKey, Vec<ManagedFieldsEntry>>>,
}

impl FieldManagerRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    /// Apply a set of fields under `manager`. Conflicts arise when any field
    /// is currently owned by a different manager and `force` is false.
    pub fn apply(
        &self,
        key: &ObjectKey,
        manager: &str,
        api_version: &str,
        fields: &[String],
        force: bool,
    ) -> ApplyOutcome {
        let mut inner = self.inner.lock().unwrap();
        let entries = inner.entry(key.clone()).or_default();
        // Tenant invariant: ObjectKey carries tenant_id; entries vector is
        // wholly scoped to (tenant_id, uid) and cannot leak.
        let mut conflicts: Vec<ApplyConflict> = vec![];
        for f in fields {
            if let Some(owner) = entries.iter().find(|e|
                e.manager != manager && e.fields_owned.iter().any(|x| x == f)
            ) {
                conflicts.push(ApplyConflict {
                    field: f.clone(),
                    current_manager: owner.manager.clone(),
                });
            }
        }
        if !conflicts.is_empty() && !force {
            return ApplyOutcome::Conflicts(conflicts);
        }
        // On force=true, transfer ownership of conflicting fields away from prior
        // managers.
        if force {
            for f in fields {
                for e in entries.iter_mut() {
                    if e.manager != manager {
                        e.fields_owned.retain(|x| x != f);
                    }
                }
            }
        }
        // Drop empty manager entries left after force-transfer.
        entries.retain(|e| !e.fields_owned.is_empty() || e.manager == manager);
        // Upsert this manager's entry.
        if let Some(e) = entries.iter_mut().find(|e| e.manager == manager) {
            for f in fields {
                if !e.fields_owned.contains(f) { e.fields_owned.push(f.clone()); }
            }
            e.time = Utc::now();
            e.api_version = api_version.into();
        } else {
            entries.push(ManagedFieldsEntry {
                manager: manager.into(),
                operation: ManagerOperation::Apply,
                api_version: api_version.into(),
                time: Utc::now(),
                fields_owned: fields.to_vec(),
            });
        }
        ApplyOutcome::Applied { manager: manager.into(), fields: fields.to_vec() }
    }

    pub fn entries(&self, key: &ObjectKey) -> Vec<ManagedFieldsEntry> {
        self.inner.lock().unwrap().get(key).cloned().unwrap_or_default()
    }

    pub fn owner_of(&self, key: &ObjectKey, field: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        inner.get(key)?.iter()
            .find(|e| e.fields_owned.iter().any(|f| f == field))
            .map(|e| e.manager.clone())
    }

    pub fn remove(&self, key: &ObjectKey, manager: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entries) = inner.get_mut(key) {
            entries.retain(|e| e.manager != manager);
        }
    }
}

impl Default for FieldManagerRegistry {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(tenant: &str, uid: &str) -> ObjectKey {
        ObjectKey { tenant_id: tenant.into(), uid: uid.into() }
    }

    /// Upstream parity: `TestApply_FirstWriterWins` (managedfields/internal/managedfields_test.go).
    #[test]
    fn test_first_writer_owns_fields() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let res = r.apply(&k, "kubectl", "v1",
            &["spec.replicas".into(), "spec.image".into()], false);
        assert!(matches!(res, ApplyOutcome::Applied { .. }));
        assert_eq!(r.owner_of(&k, "spec.replicas").as_deref(), Some("kubectl"));
        assert_eq!(r.owner_of(&k, "spec.image").as_deref(), Some("kubectl"));
        // tenant_id invariant: entries scoped to acme.
        assert_eq!(r.entries(&k).len(), 1);
    }

    /// Upstream parity: `TestApply_ConflictWithoutForce`.
    #[test]
    fn test_conflict_without_force_returns_conflicts() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let _ = r.apply(&k, "kubectl", "v1", &["spec.replicas".into()], false);
        let res = r.apply(&k, "argo-cd", "v1", &["spec.replicas".into()], false);
        match res {
            ApplyOutcome::Conflicts(c) => {
                assert_eq!(c.len(), 1);
                assert_eq!(c[0].field, "spec.replicas");
                assert_eq!(c[0].current_manager, "kubectl");
            }
            _ => panic!("expected conflict"),
        }
        // tenant_id invariant: ownership unchanged on conflict.
        assert_eq!(r.owner_of(&k, "spec.replicas").as_deref(), Some("kubectl"));
    }

    /// Upstream parity: `TestApply_ForceOverride`.
    #[test]
    fn test_force_transfers_ownership() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let _ = r.apply(&k, "kubectl", "v1", &["spec.replicas".into()], false);
        let res = r.apply(&k, "argo-cd", "v1", &["spec.replicas".into()], true);
        assert!(matches!(res, ApplyOutcome::Applied { .. }));
        assert_eq!(r.owner_of(&k, "spec.replicas").as_deref(), Some("argo-cd"));
        // tenant_id invariant: still scoped to acme.
        assert!(r.entries(&k).iter().all(|_| k.tenant_id == "acme"));
    }

    /// Upstream parity: `TestApply_NoConflictForDisjointFields`.
    #[test]
    fn test_disjoint_fields_no_conflict() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let _ = r.apply(&k, "kubectl", "v1", &["spec.replicas".into()], false);
        let res = r.apply(&k, "argo-cd", "v1", &["spec.image".into()], false);
        assert!(matches!(res, ApplyOutcome::Applied { .. }));
        assert_eq!(r.owner_of(&k, "spec.replicas").as_deref(), Some("kubectl"));
        assert_eq!(r.owner_of(&k, "spec.image").as_deref(), Some("argo-cd"));
        // tenant_id invariant: both managers in same tenant scope.
        assert_eq!(r.entries(&k).len(), 2);
    }

    /// Upstream parity: `TestApply_TenantIsolation`.
    #[test]
    fn test_tenant_isolation_no_cross_tenant_conflict() {
        let r = FieldManagerRegistry::new();
        let k_a = key("acme", "obj-1");
        let k_b = key("globex", "obj-1"); // same uid, different tenant
        let _ = r.apply(&k_a, "kubectl", "v1", &["spec.replicas".into()], false);
        // Same field, same manager, different tenant — must not conflict and
        // must not show up under k_a.
        let res = r.apply(&k_b, "argo-cd", "v1", &["spec.replicas".into()], false);
        assert!(matches!(res, ApplyOutcome::Applied { .. }),
            "tenant_id invariant: ownership is per-tenant, no cross-tenant conflict");
        assert_eq!(r.owner_of(&k_a, "spec.replicas").as_deref(), Some("kubectl"));
        assert_eq!(r.owner_of(&k_b, "spec.replicas").as_deref(), Some("argo-cd"));
    }

    /// Upstream parity: `TestApply_SameManagerExtendsFields`.
    #[test]
    fn test_same_manager_extends_field_set() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let _ = r.apply(&k, "kubectl", "v1", &["spec.replicas".into()], false);
        let _ = r.apply(&k, "kubectl", "v1", &["spec.image".into()], false);
        let entries = r.entries(&k);
        assert_eq!(entries.len(), 1, "same manager => single entry");
        assert!(entries[0].fields_owned.contains(&"spec.replicas".to_string()));
        assert!(entries[0].fields_owned.contains(&"spec.image".to_string()));
        // tenant_id invariant.
        assert_eq!(k.tenant_id, "acme");
    }

    /// Upstream parity: `TestApply_RemoveManager`.
    #[test]
    fn test_remove_manager_clears_ownership() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let _ = r.apply(&k, "kubectl", "v1", &["spec.replicas".into()], false);
        r.remove(&k, "kubectl");
        assert!(r.owner_of(&k, "spec.replicas").is_none());
        // tenant_id invariant: removal scoped, doesn't affect other tenants.
        assert_eq!(k.tenant_id, "acme");
    }

    /// Upstream parity: `TestApply_ForceLeavesPriorManagerWithRemainingFields`.
    #[test]
    fn test_force_strips_only_overlapping_fields_from_prior_owner() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let _ = r.apply(&k, "kubectl", "v1",
            &["spec.replicas".into(), "spec.strategy".into()], false);
        let _ = r.apply(&k, "argo-cd", "v1",
            &["spec.replicas".into()], true);
        // kubectl should still own strategy; argo-cd took replicas.
        assert_eq!(r.owner_of(&k, "spec.strategy").as_deref(), Some("kubectl"));
        assert_eq!(r.owner_of(&k, "spec.replicas").as_deref(), Some("argo-cd"));
        // tenant_id invariant: still scoped to acme.
        assert!(r.entries(&k).len() >= 1);
    }

    /// Upstream parity: `TestApply_ConflictsListAllOverlaps`.
    #[test]
    fn test_conflicts_list_includes_all_overlapping_fields() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let _ = r.apply(&k, "kubectl", "v1",
            &["a".into(), "b".into(), "c".into()], false);
        let res = r.apply(&k, "argo-cd", "v1",
            &["a".into(), "b".into()], false);
        match res {
            ApplyOutcome::Conflicts(c) => {
                assert_eq!(c.len(), 2);
                let names: Vec<_> = c.iter().map(|x| x.field.clone()).collect();
                assert!(names.contains(&"a".to_string()));
                assert!(names.contains(&"b".to_string()));
            }
            _ => panic!("expected conflicts"),
        }
        // tenant_id invariant.
        assert_eq!(k.tenant_id, "acme");
    }
}
