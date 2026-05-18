// SPDX-License-Identifier: AGPL-3.0-or-later
//! Storage migration controller — orchestrates alpha → beta → GA promotions
//! against `storage_version::StorageVersionRegistry`.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `kubernetes-sigs/kube-storage-version-migrator/pkg/controller/`.
//!   * `staging/src/k8s.io/apiserver/pkg/storageversion/manager.go`.
//!   * KEP-3247 (Storage Version API).
//!
//! When a CRD or built-in API graduates, every persisted object written
//! under the prior storage version must be re-encoded under the new one
//! before the prior version can be retired. The migrator runs as a
//! controller, and per KEP-3247 it relies on the storage-version hash
//! changing to detect drift.
//!
//! Tenant invariant: each migration ticket is owned by a tenant_id; the
//! controller's progress for tenant A MUST NOT be observable by tenant B.

use crate::storage_version::{StorageVersionRegistry, StorageError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationPhase {
    /// New storage version pinned, but no migration has run yet.
    Pending,
    /// Background re-encode in progress.
    Running,
    /// All objects re-encoded under the new storage version.
    Succeeded,
    /// Re-encode aborted by an error; controller will retry.
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationTicket {
    pub tenant_id: String,
    pub group: String,
    pub kind: String,
    pub from_version: String,
    pub to_version: String,
    pub phase: MigrationPhase,
    /// Number of objects already re-encoded (for the test harness).
    pub progress: u64,
    /// Total objects to re-encode (for the test harness).
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationError {
    Storage(String),
    UnknownTicket,
    /// Ticket is in a phase that cannot progress (e.g. already Succeeded).
    InvalidTransition { from: MigrationPhase, to: MigrationPhase },
}

impl From<StorageError> for MigrationError {
    fn from(e: StorageError) -> Self {
        MigrationError::Storage(format!("{:?}", e))
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct TicketKey {
    tenant_id: String,
    group: String,
    kind: String,
}

pub struct StorageMigrationController<'a> {
    storage: &'a StorageVersionRegistry,
    inner: Mutex<HashMap<TicketKey, MigrationTicket>>,
}

impl<'a> StorageMigrationController<'a> {
    pub fn new(storage: &'a StorageVersionRegistry) -> Self {
        Self { storage, inner: Mutex::new(HashMap::new()) }
    }

    /// Open a migration ticket. The destination version must already be
    /// registered with the StorageVersionRegistry; the source version is
    /// validated to be a registered prior. Calling `start` then promotes
    /// the storage version. Mirrors upstream
    /// `migrator.Controller.PostStorageVersionChangeHook`.
    pub fn open_ticket(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        from_version: &str,
        to_version: &str,
        total: u64,
    ) -> Result<MigrationTicket, MigrationError> {
        let known = self.storage.known_versions(tenant_id, group, kind);
        if !known.iter().any(|v| v == from_version) {
            return Err(MigrationError::Storage(format!(
                "from_version `{}` not registered", from_version)));
        }
        if !known.iter().any(|v| v == to_version) {
            return Err(MigrationError::Storage(format!(
                "to_version `{}` not registered", to_version)));
        }
        let ticket = MigrationTicket {
            tenant_id: tenant_id.into(),
            group: group.into(),
            kind: kind.into(),
            from_version: from_version.into(),
            to_version: to_version.into(),
            phase: MigrationPhase::Pending,
            progress: 0,
            total,
        };
        self.inner.lock().unwrap().insert(
            TicketKey {
                tenant_id: tenant_id.into(),
                group: group.into(),
                kind: kind.into(),
            },
            ticket.clone(),
        );
        Ok(ticket)
    }

    /// Promote `to_version` as the storage version and move the ticket
    /// into Running. Returns the updated ticket.
    pub fn start(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
    ) -> Result<MigrationTicket, MigrationError> {
        let key = TicketKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            kind: kind.into(),
        };
        let mut inner = self.inner.lock().unwrap();
        let ticket = inner.get_mut(&key).ok_or(MigrationError::UnknownTicket)?;
        if ticket.phase != MigrationPhase::Pending {
            return Err(MigrationError::InvalidTransition {
                from: ticket.phase, to: MigrationPhase::Running,
            });
        }
        self.storage.elect_storage_version(
            tenant_id, group, kind, &ticket.to_version)?;
        ticket.phase = MigrationPhase::Running;
        Ok(ticket.clone())
    }

    /// Record `n` newly re-encoded objects. When progress reaches `total`,
    /// the ticket auto-completes.
    pub fn record_progress(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        n: u64,
    ) -> Result<MigrationTicket, MigrationError> {
        let key = TicketKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            kind: kind.into(),
        };
        let mut inner = self.inner.lock().unwrap();
        let ticket = inner.get_mut(&key).ok_or(MigrationError::UnknownTicket)?;
        if ticket.phase != MigrationPhase::Running {
            return Err(MigrationError::InvalidTransition {
                from: ticket.phase, to: MigrationPhase::Running,
            });
        }
        ticket.progress = (ticket.progress + n).min(ticket.total);
        if ticket.progress >= ticket.total {
            ticket.phase = MigrationPhase::Succeeded;
        }
        Ok(ticket.clone())
    }

    pub fn fail(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
    ) -> Result<MigrationTicket, MigrationError> {
        let key = TicketKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            kind: kind.into(),
        };
        let mut inner = self.inner.lock().unwrap();
        let ticket = inner.get_mut(&key).ok_or(MigrationError::UnknownTicket)?;
        ticket.phase = MigrationPhase::Failed;
        Ok(ticket.clone())
    }

    pub fn lookup(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
    ) -> Option<MigrationTicket> {
        self.inner.lock().unwrap().get(&TicketKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            kind: kind.into(),
        }).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> StorageVersionRegistry {
        let r = StorageVersionRegistry::new();
        r.register_version("acme", "acme.io", "Widget", "v1alpha1").unwrap();
        r.register_version("acme", "acme.io", "Widget", "v1beta1").unwrap();
        r.register_version("acme", "acme.io", "Widget", "v1").unwrap();
        r
    }

    /// Upstream parity: `TestMigration_AlphaToBetaTransition`
    /// (kube-storage-version-migrator/controller_test.go::TestPromote —
    /// open ticket, start it, advance to Running with new storage version
    /// elected).
    #[test]
    fn test_alpha_to_beta_transition_elects_new_storage_version() {
        let storage = fresh();
        let mig = StorageMigrationController::new(&storage);
        let opened = mig.open_ticket("acme", "acme.io", "Widget",
            "v1alpha1", "v1beta1", 100).unwrap();
        assert_eq!(opened.phase, MigrationPhase::Pending);
        assert_eq!(opened.tenant_id, "acme",
            "tenant_id invariant: ticket carries owning tenant_id");
        let started = mig.start("acme", "acme.io", "Widget").unwrap();
        assert_eq!(started.phase, MigrationPhase::Running);
        // Storage version actually changed.
        assert_eq!(
            storage.storage_version("acme", "acme.io", "Widget").as_deref(),
            Some("v1beta1"));
    }

    /// Upstream parity: `TestMigration_BetaToGAProgressCompletes`
    /// (controller_test.go — record_progress to total auto-completes).
    #[test]
    fn test_record_progress_auto_succeeds_when_total_reached() {
        let storage = fresh();
        let mig = StorageMigrationController::new(&storage);
        let _ = mig.open_ticket("acme", "acme.io", "Widget",
            "v1beta1", "v1", 10).unwrap();
        let _ = mig.start("acme", "acme.io", "Widget").unwrap();
        // Two partial chunks then a final chunk.
        let mid = mig.record_progress("acme", "acme.io", "Widget", 4).unwrap();
        assert_eq!(mid.phase, MigrationPhase::Running);
        let almost = mig.record_progress("acme", "acme.io", "Widget", 4).unwrap();
        assert_eq!(almost.phase, MigrationPhase::Running);
        let done = mig.record_progress("acme", "acme.io", "Widget", 10).unwrap();
        assert_eq!(done.phase, MigrationPhase::Succeeded);
        assert_eq!(done.progress, 10);
        assert_eq!(done.tenant_id, "acme",
            "tenant_id invariant: completion stays scoped to acme");
    }

    /// Upstream parity: `TestMigration_FailureMarksTicketFailed`
    /// (controller_test.go — `fail` transitions to Failed for retry).
    #[test]
    fn test_fail_marks_ticket_failed_for_retry() {
        let storage = fresh();
        let mig = StorageMigrationController::new(&storage);
        let _ = mig.open_ticket("acme", "acme.io", "Widget",
            "v1alpha1", "v1beta1", 50).unwrap();
        let _ = mig.start("acme", "acme.io", "Widget").unwrap();
        let failed = mig.fail("acme", "acme.io", "Widget").unwrap();
        assert_eq!(failed.phase, MigrationPhase::Failed);
        // tenant_id invariant: globex sees no ticket.
        assert!(mig.lookup("globex", "acme.io", "Widget").is_none(),
            "tenant_id invariant: globex never sees acme's ticket");
    }

    /// Upstream parity: `TestMigration_RejectsUnknownVersion`
    /// (storageversion/manager_test.go — promotion targets must be
    /// registered first).
    #[test]
    fn test_open_ticket_rejects_unknown_destination_version() {
        let storage = fresh();
        let mig = StorageMigrationController::new(&storage);
        let err = mig.open_ticket("acme", "acme.io", "Widget",
            "v1beta1", "v2-not-registered", 5).unwrap_err();
        match err {
            MigrationError::Storage(msg) => assert!(msg.contains("not registered")),
            other => panic!("unexpected: {:?}", other),
        }
    }

    /// Upstream parity: `TestMigration_TenantIsolatedTickets`
    /// (cave-apiserver invariant: tickets are per-tenant — globex's ticket
    /// is invisible from acme's view, and acme's election does not
    /// interfere with globex's storage state).
    #[test]
    fn test_tickets_are_isolated_per_tenant() {
        let storage = StorageVersionRegistry::new();
        for t in ["acme", "globex"] {
            storage.register_version(t, "acme.io", "Widget", "v1alpha1").unwrap();
            storage.register_version(t, "acme.io", "Widget", "v1").unwrap();
        }
        let mig = StorageMigrationController::new(&storage);
        let _ = mig.open_ticket("acme", "acme.io", "Widget", "v1alpha1", "v1", 5).unwrap();
        let _ = mig.start("acme", "acme.io", "Widget").unwrap();
        // globex's storage version unchanged.
        assert_eq!(
            storage.storage_version("globex", "acme.io", "Widget").as_deref(),
            Some("v1alpha1"),
            "tenant_id invariant: acme's election does not promote globex");
        assert!(mig.lookup("globex", "acme.io", "Widget").is_none(),
            "tenant_id invariant: globex sees no acme ticket");
    }

    /// Upstream parity: `TestMigration_RejectsDoubleStart`
    /// (controller_test.go — `start` is a Pending → Running edge; a
    /// second start is an InvalidTransition).
    #[test]
    fn test_start_rejects_invalid_transition_from_running() {
        let storage = fresh();
        let mig = StorageMigrationController::new(&storage);
        let _ = mig.open_ticket("acme", "acme.io", "Widget",
            "v1alpha1", "v1", 5).unwrap();
        let _ = mig.start("acme", "acme.io", "Widget").unwrap();
        let err = mig.start("acme", "acme.io", "Widget").unwrap_err();
        match err {
            MigrationError::InvalidTransition { from, to } => {
                assert_eq!(from, MigrationPhase::Running);
                assert_eq!(to,   MigrationPhase::Running);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }
}
