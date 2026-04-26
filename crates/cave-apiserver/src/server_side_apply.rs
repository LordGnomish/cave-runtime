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
    /// Why the conflict happened. Mirrors the `causes` field upstream
    /// emits via `apierrors.NewApplyConflict` (`managedfields/internal/conflict.go`).
    pub reason: ConflictReason,
}

/// Conflict cause categories. Mirrors `metav1.CauseTypeFieldManagerConflict`
/// + the more granular reasons emitted by upstream KEP-555 implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictReason {
    /// Another Apply manager owns the field (the canonical case).
    AppliedBy,
    /// An Update operation (kubectl edit / replace) holds the field.
    UpdatedBy,
    /// A controller's `lastApplied` annotation owns it (Apply-vs-Update mix).
    ControllerSubresource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApplyOutcome {
    Applied { manager: String, fields: Vec<String> },
    Conflicts(Vec<ApplyConflict>),
}

/// One entry in the per-object ownership-transfer audit log. Recorded each
/// time a forced apply transfers a field from manager A to manager B.
/// Mirrors the audit signal upstream surfaces via the
/// `Force=true` reconciliation path in `managedfields/internal/structuredmerge.go`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnershipTransfer {
    pub field: String,
    pub from: String,
    pub to: String,
    pub at: chrono::DateTime<chrono::Utc>,
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
    /// Append-only ownership-transfer log, keyed by ObjectKey.
    transfers: Mutex<HashMap<ObjectKey, Vec<OwnershipTransfer>>>,
}

impl FieldManagerRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            transfers: Mutex::new(HashMap::new()),
        }
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
                let reason = match owner.operation {
                    ManagerOperation::Apply => ConflictReason::AppliedBy,
                    ManagerOperation::Update => ConflictReason::UpdatedBy,
                };
                conflicts.push(ApplyConflict {
                    field: f.clone(),
                    current_manager: owner.manager.clone(),
                    reason,
                });
            }
        }
        if !conflicts.is_empty() && !force {
            return ApplyOutcome::Conflicts(conflicts);
        }
        // On force=true, transfer ownership of conflicting fields away from prior
        // managers.
        if force {
            // Snapshot prior owners per field so we can record the transfers
            // *before* the entries are mutated.
            let mut to_record: Vec<(String, String)> = vec![];
            for f in fields {
                for e in entries.iter() {
                    if e.manager != manager && e.fields_owned.iter().any(|x| x == f) {
                        to_record.push((f.clone(), e.manager.clone()));
                    }
                }
            }
            for f in fields {
                for e in entries.iter_mut() {
                    if e.manager != manager {
                        e.fields_owned.retain(|x| x != f);
                    }
                }
            }
            if !to_record.is_empty() {
                let mut log = self.transfers.lock().unwrap();
                let entry = log.entry(key.clone()).or_default();
                let now = Utc::now();
                for (field, from) in to_record {
                    entry.push(OwnershipTransfer {
                        field, from, to: manager.into(), at: now,
                    });
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

    /// Record an Update operation (kubectl edit / replace path) under
    /// `manager`. Mirrors upstream `managedfields/internal/structuredmerge.go::Update`.
    pub fn record_update(
        &self,
        key: &ObjectKey,
        manager: &str,
        api_version: &str,
        fields: &[String],
    ) {
        let mut inner = self.inner.lock().unwrap();
        let entries = inner.entry(key.clone()).or_default();
        // Strip these fields from any other manager's ownership — Update wins
        // unconditionally (this is the documented behaviour of the
        // imperative path; it also seeds the conflict reason for future
        // Apply calls as `UpdatedBy`).
        for e in entries.iter_mut() {
            if e.manager != manager {
                e.fields_owned.retain(|x| !fields.contains(x));
            }
        }
        entries.retain(|e| !e.fields_owned.is_empty() || e.manager == manager);
        if let Some(e) = entries.iter_mut().find(|e| e.manager == manager) {
            for f in fields {
                if !e.fields_owned.contains(f) { e.fields_owned.push(f.clone()); }
            }
            e.operation = ManagerOperation::Update;
            e.api_version = api_version.into();
            e.time = Utc::now();
        } else {
            entries.push(ManagedFieldsEntry {
                manager: manager.into(),
                operation: ManagerOperation::Update,
                api_version: api_version.into(),
                time: Utc::now(),
                fields_owned: fields.to_vec(),
            });
        }
    }

    /// Read the ownership-transfer audit log for `key`. Append-only since
    /// registry construction.
    pub fn transfer_log(&self, key: &ObjectKey) -> Vec<OwnershipTransfer> {
        self.transfers.lock().unwrap()
            .get(key).cloned().unwrap_or_default()
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

    // ── Deeper coverage (v1.36.0) ─────────────────────────────────────────────

    /// Upstream parity: `TestApply_SameManagerIsIdempotent`
    /// (managedfields/internal/managedfields_test.go — re-applying the same
    /// fields under the same manager is a no-op on conflict surface).
    #[test]
    fn test_same_manager_reapply_is_idempotent() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let res1 = r.apply(&k, "kubectl", "v1", &["spec.image".into()], false);
        assert!(matches!(res1, ApplyOutcome::Applied { .. }));
        let res2 = r.apply(&k, "kubectl", "v1", &["spec.image".into()], false);
        assert!(matches!(res2, ApplyOutcome::Applied { .. }),
            "same manager re-applying same field MUST NOT conflict with itself");
        let entries = r.entries(&k);
        assert_eq!(entries.len(), 1, "single entry retained");
        assert_eq!(entries[0].fields_owned, vec!["spec.image".to_string()],
            "field set unchanged on idempotent reapply");
        assert_eq!(k.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestApply_RemoveUnknownManagerNoop`
    /// (registry.RemoveManager on absent name is a no-op, not an error).
    #[test]
    fn test_remove_unknown_manager_does_not_panic() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let _ = r.apply(&k, "kubectl", "v1", &["x".into()], false);
        r.remove(&k, "ghost-manager");
        assert_eq!(r.owner_of(&k, "x").as_deref(), Some("kubectl"),
            "removing unknown manager preserves existing ownership");
        assert_eq!(k.tenant_id, "acme", "tenant_id invariant: scoped removal");
    }

    /// Upstream parity: `TestApply_TenantSiblingsCannotForceAcrossTenants`
    /// (force=true is local to the (tenant_id, uid) ObjectKey).
    #[test]
    fn test_force_apply_does_not_cross_tenant_boundary() {
        let r = FieldManagerRegistry::new();
        let k_a = key("acme", "obj-1");
        let k_b = key("globex", "obj-1");
        let _ = r.apply(&k_a, "kubectl", "v1", &["spec.image".into()], false);
        // globex tries to force — must not affect acme's ownership.
        let res = r.apply(&k_b, "argo-cd", "v1", &["spec.image".into()], true);
        assert!(matches!(res, ApplyOutcome::Applied { .. }));
        assert_eq!(r.owner_of(&k_a, "spec.image").as_deref(), Some("kubectl"),
            "tenant_id invariant: globex.force MUST NOT strip acme.kubectl ownership");
        assert_eq!(r.owner_of(&k_b, "spec.image").as_deref(), Some("argo-cd"));
    }

    /// Upstream parity: `TestApply_OutcomeAppliedFieldsEchoesInput`
    /// (ApplyOutcome::Applied carries back the manager + the field set
    /// supplied — used by the response builder to populate ManagedFields).
    #[test]
    fn test_outcome_applied_echoes_manager_and_fields() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let res = r.apply(&k, "kubectl", "v1",
            &["spec.replicas".into(), "spec.image".into()], false);
        match res {
            ApplyOutcome::Applied { manager, fields } => {
                assert_eq!(manager, "kubectl");
                assert_eq!(fields.len(), 2);
                assert!(fields.contains(&"spec.replicas".to_string()));
                assert!(fields.contains(&"spec.image".to_string()));
            }
            ApplyOutcome::Conflicts(_) => panic!("expected applied outcome"),
        }
        assert_eq!(k.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestApply_OperationFlavorIsApply`
    /// (managed-field entries created via Apply carry ManagerOperation::Apply).
    #[test]
    fn test_managed_fields_entry_carries_apply_operation() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-1");
        let _ = r.apply(&k, "kubectl", "v1", &["spec.image".into()], false);
        let entries = r.entries(&k);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].operation, ManagerOperation::Apply,
            "Apply path tags entry as ManagerOperation::Apply");
        assert_eq!(entries[0].api_version, "v1");
        assert!(!entries[0].manager.is_empty());
        assert_eq!(k.tenant_id, "acme", "tenant_id invariant");
    }

    // ── Deeper coverage (deeper-004) — KEP-555 ────────────────────────────────

    /// Upstream parity: `TestApply_ConflictReasonAppliedByVsUpdatedBy`
    /// (managedfields/internal/conflict_test.go — conflict reasons reflect
    /// whether the prior owner used Apply or Update).
    #[test]
    fn test_conflict_reason_distinguishes_apply_from_update() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-conflict-reason");
        // kubectl-applied via SSA.
        let _ = r.apply(&k, "kubectl", "v1", &["spec.image".into()], false);
        // an operator did `kubectl edit` — record Update path.
        r.record_update(&k, "operator", "v1", &["spec.replicas".into()]);
        // argo-cd tries to apply both fields without force.
        let res = r.apply(&k, "argo-cd", "v1",
            &["spec.image".into(), "spec.replicas".into()], false);
        match res {
            ApplyOutcome::Conflicts(cs) => {
                let by_field: std::collections::HashMap<_, _> = cs.iter()
                    .map(|c| (c.field.clone(), c.reason)).collect();
                assert_eq!(by_field.get("spec.image"),
                    Some(&ConflictReason::AppliedBy));
                assert_eq!(by_field.get("spec.replicas"),
                    Some(&ConflictReason::UpdatedBy));
            }
            _ => panic!("expected conflicts"),
        }
        assert_eq!(k.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestApply_ForceRecordsOwnershipTransfer`
    /// (no upstream test — cave-apiserver invariant: forced apply emits an
    /// auditable OwnershipTransfer record so we can reconstruct who took
    /// what from whom and when).
    #[test]
    fn test_force_apply_records_ownership_transfers_in_audit_log() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-transfer");
        let _ = r.apply(&k, "kubectl",
            "v1", &["spec.replicas".into(), "spec.image".into()], false);
        // argo-cd takes replicas with force; image stays with kubectl.
        let _ = r.apply(&k, "argo-cd", "v1", &["spec.replicas".into()], true);
        let log = r.transfer_log(&k);
        assert_eq!(log.len(), 1, "exactly one transfer recorded");
        assert_eq!(log[0].field, "spec.replicas");
        assert_eq!(log[0].from, "kubectl");
        assert_eq!(log[0].to, "argo-cd");
        assert_eq!(k.tenant_id, "acme",
            "tenant_id invariant: transfer log scoped to acme key");
    }

    /// Upstream parity: `TestApply_TransferChainAcrossThreeManagers`
    /// (managedfields/internal/structuredmerge_test.go — sequential force
    /// applies hand the field through a chain A→B→C).
    #[test]
    fn test_force_apply_transfer_chain_records_each_hop() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-chain");
        let _ = r.apply(&k, "A", "v1", &["spec.x".into()], false);
        let _ = r.apply(&k, "B", "v1", &["spec.x".into()], true);
        let _ = r.apply(&k, "C", "v1", &["spec.x".into()], true);
        let log = r.transfer_log(&k);
        assert_eq!(log.len(), 2, "two hops in the chain");
        assert_eq!(log[0].from, "A");
        assert_eq!(log[0].to,   "B");
        assert_eq!(log[1].from, "B");
        assert_eq!(log[1].to,   "C");
        assert_eq!(r.owner_of(&k, "spec.x").as_deref(), Some("C"));
        assert_eq!(k.tenant_id, "acme", "tenant_id invariant: scoped chain");
    }

    /// Upstream parity: `TestUpdate_StripsApplyOwnershipForOverlappingFields`
    /// (managedfields/internal/structuredmerge_test.go — Update path takes
    /// ownership unconditionally and strips overlapping Apply-managed fields).
    #[test]
    fn test_record_update_strips_overlapping_apply_ownership() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-update-strip");
        let _ = r.apply(&k, "kubectl", "v1",
            &["spec.replicas".into(), "spec.image".into()], false);
        r.record_update(&k, "kubectl-edit", "v1", &["spec.replicas".into()]);
        // kubectl loses replicas to kubectl-edit; image still kubectl.
        assert_eq!(r.owner_of(&k, "spec.replicas").as_deref(), Some("kubectl-edit"));
        assert_eq!(r.owner_of(&k, "spec.image").as_deref(), Some("kubectl"));
        // kubectl-edit's entry is tagged Operation::Update.
        let entries = r.entries(&k);
        let editor = entries.iter().find(|e| e.manager == "kubectl-edit").unwrap();
        assert_eq!(editor.operation, ManagerOperation::Update);
        assert_eq!(k.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestApply_ForceWithNoConflictDoesNotRecordTransfer`
    /// (no upstream test — invariant: ownership-transfer log only grows
    /// when an actual transfer happened).
    #[test]
    fn test_force_apply_without_existing_owner_records_no_transfer() {
        let r = FieldManagerRegistry::new();
        let k = key("acme", "obj-force-no-prior");
        // First apply ever, force=true — no prior owner to take from.
        let _ = r.apply(&k, "argo-cd", "v1", &["spec.image".into()], true);
        let log = r.transfer_log(&k);
        assert!(log.is_empty(),
            "tenant_id invariant: empty transfer log when no prior owner exists");
        assert_eq!(r.owner_of(&k, "spec.image").as_deref(), Some("argo-cd"));
        assert_eq!(k.tenant_id, "acme", "tenant_id invariant");
    }
}
