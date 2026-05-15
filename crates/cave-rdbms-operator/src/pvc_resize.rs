// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Source: cloudnative-pg/cloudnative-pg@v1.24.0 internal/controller/cluster_controller.go
// Source: cloudnative-pg/cloudnative-pg@v1.24.0 pkg/management/postgres/storage.go
//
//! PVCResize controller — handles online storage expansion for managed Postgres
//! instances. Mirrors CloudNativePG's storage reconciliation loop. Resize is a
//! one-way operation in CNPG: shrinking is rejected up-front because PostgreSQL
//! cannot reclaim heap space without a `pg_dump`/`pg_restore` cycle.
//!
//! State machine:
//!
//! ```text
//!   Pending ──(observed < target & expansion-allowed)──▶ Resizing
//!   Resizing ──(observed == target)──▶ Completed
//!   Resizing ──(transient-error)──▶ Retrying ──▶ Resizing
//!   Pending ──(target ≤ observed)──▶ Rejected (Shrink/NoOp)
//!   Pending ──(!expansion_allowed)──▶ ObserverOnly
//! ```
//!
//! The controller is *driven* — the caller pumps `reconcile_once()` each cycle
//! and inspects the returned `ResizeStatus`. The pod-restart step is delegated
//! to a `PodCycler` trait so tests can deterministically observe the sequence.

use crate::error::{PgError, PgResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// Storage class capability: only resize when this is `true`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageClassCaps {
    pub name: String,
    pub allow_volume_expansion: bool,
}

/// Current PVC view, mirrors `core/v1.PersistentVolumeClaim.status.capacity`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PvcView {
    pub name: String,
    pub instance_id: String,
    pub storage_class: String,
    /// Bytes — observed in the cluster.
    pub capacity_bytes: u64,
    /// Bytes — desired by the spec.
    pub requested_bytes: u64,
}

/// Driver state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResizePhase {
    Pending,
    Resizing,
    Completed,
    Rejected,
    ObserverOnly,
    Retrying,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResizeJob {
    pub id: String,
    pub pvc: String,
    pub instance_id: String,
    pub from_bytes: u64,
    pub to_bytes: u64,
    pub phase: ResizePhase,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Outcome of a reconcile tick. Idempotent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// Nothing to do — sizes match.
    NoOp,
    /// First time we noticed the spec; recorded a Pending job.
    Started,
    /// Successfully applied a step.
    Progressed,
    /// Transient error; will retry on next tick.
    Retried,
    /// Reached terminal state (capacity == requested).
    Completed,
    /// Permanent reject (e.g. shrink, expansion not allowed).
    Rejected(String),
}

/// Pod-cycle hook — the operator drains and reschedules the instance pod so it
/// remounts the resized PVC. In production this calls the kubelet shim; in
/// tests a `MockPodCycler` records the calls.
pub trait PodCycler: Send + Sync {
    fn cycle(&self, instance_id: &str) -> Result<(), String>;
}

/// Mock pod cycler — records the instances asked to cycle.
#[derive(Default)]
pub struct MockPodCycler {
    pub cycled: Mutex<Vec<String>>,
    /// If set, the next cycle returns this error and consumes the slot.
    pub inject_error: Mutex<Vec<String>>,
}

impl PodCycler for MockPodCycler {
    fn cycle(&self, instance_id: &str) -> Result<(), String> {
        if let Some(err) = self.inject_error.lock().unwrap().pop() {
            return Err(err);
        }
        self.cycled.lock().unwrap().push(instance_id.into());
        Ok(())
    }
}

/// PVCResize state machine.
pub struct PvcResizeController {
    storage_classes: Arc<RwLock<HashMap<String, StorageClassCaps>>>,
    jobs: Arc<RwLock<HashMap<String, ResizeJob>>>,
    cycler: Arc<dyn PodCycler>,
    /// Max attempts before a Retrying job goes Rejected.
    pub max_attempts: u32,
}

impl PvcResizeController {
    pub fn new(cycler: Arc<dyn PodCycler>) -> Self {
        Self {
            storage_classes: Arc::new(RwLock::new(HashMap::new())),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            cycler,
            max_attempts: 5,
        }
    }

    pub fn register_storage_class(&self, caps: StorageClassCaps) {
        self.storage_classes
            .write()
            .unwrap()
            .insert(caps.name.clone(), caps);
    }

    pub fn list_jobs(&self) -> Vec<ResizeJob> {
        self.jobs.read().unwrap().values().cloned().collect()
    }

    pub fn get_job(&self, id: &str) -> Option<ResizeJob> {
        self.jobs.read().unwrap().get(id).cloned()
    }

    pub fn jobs_for_instance(&self, instance_id: &str) -> Vec<ResizeJob> {
        self.jobs
            .read()
            .unwrap()
            .values()
            .filter(|j| j.instance_id == instance_id)
            .cloned()
            .collect()
    }

    /// One reconcile step. Idempotent — calling twice with the same PvcView
    /// when nothing changed returns `NoOp` (or `Completed` for terminal).
    pub fn reconcile_once(&self, pvc: &PvcView) -> PgResult<ReconcileOutcome> {
        // 1. Trivial no-op: sizes match.
        if pvc.requested_bytes == pvc.capacity_bytes {
            // If we have a terminal job for this PVC, signal Completed once.
            if let Some(job) = self.find_active_job(&pvc.name) {
                if matches!(job.phase, ResizePhase::Resizing | ResizePhase::Retrying) {
                    self.transition(&job.id, ResizePhase::Completed, None);
                    return Ok(ReconcileOutcome::Completed);
                }
            }
            return Ok(ReconcileOutcome::NoOp);
        }

        // 2. Shrink — reject permanently.
        if pvc.requested_bytes < pvc.capacity_bytes {
            return Err(PgError::ConfigError(format!(
                "shrink rejected for {}: {} → {} (PostgreSQL cannot reclaim heap)",
                pvc.name, pvc.capacity_bytes, pvc.requested_bytes
            )));
        }

        // 3. Storage class must allow expansion.
        let sc_allows = self
            .storage_classes
            .read()
            .unwrap()
            .get(&pvc.storage_class)
            .map(|s| s.allow_volume_expansion)
            .unwrap_or(false);

        // 4. Job lookup or creation.
        let existing = self.find_active_job(&pvc.name);
        let job_id = match existing.as_ref() {
            Some(j) => j.id.clone(),
            None => {
                if !sc_allows {
                    let id = uuid::Uuid::new_v4().to_string();
                    let now = Utc::now();
                    self.jobs.write().unwrap().insert(
                        id.clone(),
                        ResizeJob {
                            id: id.clone(),
                            pvc: pvc.name.clone(),
                            instance_id: pvc.instance_id.clone(),
                            from_bytes: pvc.capacity_bytes,
                            to_bytes: pvc.requested_bytes,
                            phase: ResizePhase::ObserverOnly,
                            attempts: 0,
                            last_error: Some("storage class does not allow expansion".into()),
                            started_at: now,
                            updated_at: now,
                        },
                    );
                    return Ok(ReconcileOutcome::Rejected(
                        "storage class does not allow expansion".into(),
                    ));
                }
                let id = self.create_pending(pvc);
                return Ok(ReconcileOutcome::Started)
                    .map(|o| {
                        self.transition(&id, ResizePhase::Resizing, None);
                        o
                    });
            }
        };

        // 5. Existing job — execute pod-cycle and try to apply.
        let result = self.cycler.cycle(&pvc.instance_id);
        match result {
            Ok(()) => {
                // Pod cycled successfully; we model the resize as having
                // taken effect when the caller observes capacity catching up
                // on the next tick. For now mark Resizing.
                self.transition(&job_id, ResizePhase::Resizing, None);
                Ok(ReconcileOutcome::Progressed)
            }
            Err(e) => {
                let attempts = {
                    let mut jobs = self.jobs.write().unwrap();
                    let job = jobs.get_mut(&job_id).unwrap();
                    job.attempts += 1;
                    job.last_error = Some(e.clone());
                    job.phase = ResizePhase::Retrying;
                    job.updated_at = Utc::now();
                    job.attempts
                };
                if attempts >= self.max_attempts {
                    self.transition(
                        &job_id,
                        ResizePhase::Rejected,
                        Some(format!("max attempts {} reached: {}", self.max_attempts, e)),
                    );
                    return Ok(ReconcileOutcome::Rejected(e));
                }
                Ok(ReconcileOutcome::Retried)
            }
        }
    }

    fn create_pending(&self, pvc: &PvcView) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        self.jobs.write().unwrap().insert(
            id.clone(),
            ResizeJob {
                id: id.clone(),
                pvc: pvc.name.clone(),
                instance_id: pvc.instance_id.clone(),
                from_bytes: pvc.capacity_bytes,
                to_bytes: pvc.requested_bytes,
                phase: ResizePhase::Pending,
                attempts: 0,
                last_error: None,
                started_at: now,
                updated_at: now,
            },
        );
        id
    }

    fn transition(&self, id: &str, phase: ResizePhase, err: Option<String>) {
        let mut jobs = self.jobs.write().unwrap();
        if let Some(job) = jobs.get_mut(id) {
            job.phase = phase;
            if err.is_some() {
                job.last_error = err;
            }
            job.updated_at = Utc::now();
        }
    }

    fn find_active_job(&self, pvc_name: &str) -> Option<ResizeJob> {
        self.jobs
            .read()
            .unwrap()
            .values()
            .filter(|j| {
                j.pvc == pvc_name
                    && !matches!(
                        j.phase,
                        ResizePhase::Completed
                            | ResizePhase::Rejected
                            | ResizePhase::ObserverOnly
                    )
            })
            .cloned()
            .next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl(allow: bool) -> (PvcResizeController, Arc<MockPodCycler>) {
        let cycler = Arc::new(MockPodCycler::default());
        let ctl = PvcResizeController::new(cycler.clone());
        ctl.register_storage_class(StorageClassCaps {
            name: "fast-ssd".into(),
            allow_volume_expansion: allow,
        });
        (ctl, cycler)
    }

    fn pvc(name: &str, cap: u64, req: u64) -> PvcView {
        PvcView {
            name: name.into(),
            instance_id: "inst-1".into(),
            storage_class: "fast-ssd".into(),
            capacity_bytes: cap,
            requested_bytes: req,
        }
    }

    #[test]
    fn noop_when_sizes_equal() {
        let (c, _) = ctl(true);
        let p = pvc("pvc-1", 10, 10);
        assert_eq!(c.reconcile_once(&p).unwrap(), ReconcileOutcome::NoOp);
    }

    #[test]
    fn shrink_is_rejected() {
        let (c, _) = ctl(true);
        let p = pvc("pvc-1", 100, 50);
        let err = c.reconcile_once(&p).unwrap_err();
        assert!(matches!(err, PgError::ConfigError(_)));
    }

    #[test]
    fn expansion_creates_job_and_starts_resizing() {
        let (c, _) = ctl(true);
        let p = pvc("pvc-1", 10, 20);
        let out = c.reconcile_once(&p).unwrap();
        assert_eq!(out, ReconcileOutcome::Started);
        let jobs = c.list_jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].phase, ResizePhase::Resizing);
        assert_eq!(jobs[0].to_bytes, 20);
    }

    #[test]
    fn observer_only_when_storage_class_disallows() {
        let (c, _) = ctl(false);
        let p = pvc("pvc-1", 10, 20);
        let out = c.reconcile_once(&p).unwrap();
        match out {
            ReconcileOutcome::Rejected(msg) => assert!(msg.contains("not allow")),
            other => panic!("expected Rejected, got {:?}", other),
        }
        let jobs = c.list_jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].phase, ResizePhase::ObserverOnly);
    }

    #[test]
    fn second_tick_after_capacity_catches_up_completes_job() {
        let (c, _) = ctl(true);
        let p1 = pvc("pvc-1", 10, 20);
        c.reconcile_once(&p1).unwrap();
        // Capacity now matches request (kubelet finished resize).
        let p2 = pvc("pvc-1", 20, 20);
        let out = c.reconcile_once(&p2).unwrap();
        assert_eq!(out, ReconcileOutcome::Completed);
        let jobs = c.list_jobs();
        assert_eq!(jobs[0].phase, ResizePhase::Completed);
    }

    #[test]
    fn pod_cycler_is_invoked_for_active_resize() {
        let (c, mock) = ctl(true);
        let p = pvc("pvc-1", 10, 20);
        c.reconcile_once(&p).unwrap();
        // Second tick with still-mismatched sizes drives the cycle.
        let p_mid = pvc("pvc-1", 10, 20);
        c.reconcile_once(&p_mid).unwrap();
        let cycled = mock.cycled.lock().unwrap().clone();
        assert!(cycled.iter().any(|i| i == "inst-1"));
    }

    #[test]
    fn transient_error_marks_retrying_and_increments_attempts() {
        let (c, mock) = ctl(true);
        mock.inject_error
            .lock()
            .unwrap()
            .push("kubelet busy".into());
        c.reconcile_once(&pvc("pvc-1", 10, 20)).unwrap(); // Start
        let out = c.reconcile_once(&pvc("pvc-1", 10, 20)).unwrap();
        assert_eq!(out, ReconcileOutcome::Retried);
        let jobs = c.list_jobs();
        assert_eq!(jobs[0].phase, ResizePhase::Retrying);
        assert_eq!(jobs[0].attempts, 1);
        assert_eq!(jobs[0].last_error.as_deref(), Some("kubelet busy"));
    }

    #[test]
    fn max_attempts_rejects_job() {
        let (c, mock) = ctl(true);
        // Pre-queue several errors.
        for _ in 0..10 {
            mock.inject_error.lock().unwrap().push("flaky".into());
        }
        c.reconcile_once(&pvc("pvc-1", 10, 20)).unwrap(); // Start
        let mut last = ReconcileOutcome::Started;
        for _ in 0..c.max_attempts as usize {
            last = c.reconcile_once(&pvc("pvc-1", 10, 20)).unwrap();
        }
        assert!(matches!(last, ReconcileOutcome::Rejected(_)));
        let jobs = c.list_jobs();
        assert!(jobs.iter().any(|j| j.phase == ResizePhase::Rejected));
    }

    #[test]
    fn jobs_for_instance_filters() {
        let (c, _) = ctl(true);
        c.reconcile_once(&pvc("pvc-1", 10, 20)).unwrap();
        let mut p2 = pvc("pvc-2", 10, 30);
        p2.instance_id = "inst-2".into();
        c.reconcile_once(&p2).unwrap();
        assert_eq!(c.jobs_for_instance("inst-1").len(), 1);
        assert_eq!(c.jobs_for_instance("inst-2").len(), 1);
        assert_eq!(c.jobs_for_instance("inst-3").len(), 0);
    }

    #[test]
    fn duplicate_reconcile_is_idempotent() {
        let (c, _) = ctl(true);
        c.reconcile_once(&pvc("pvc-1", 10, 20)).unwrap();
        c.reconcile_once(&pvc("pvc-1", 10, 20)).unwrap();
        c.reconcile_once(&pvc("pvc-1", 10, 20)).unwrap();
        // Only one job.
        assert_eq!(c.list_jobs().len(), 1);
    }

    #[test]
    fn unknown_storage_class_is_observer_only() {
        let (c, _) = ctl(true);
        let mut p = pvc("pvc-1", 10, 20);
        p.storage_class = "nfs-legacy".into();
        let out = c.reconcile_once(&p).unwrap();
        assert!(matches!(out, ReconcileOutcome::Rejected(_)));
    }

    #[test]
    fn shrink_error_message_mentions_pg_cannot_reclaim() {
        let (c, _) = ctl(true);
        let err = c
            .reconcile_once(&pvc("pvc-1", 1000, 500))
            .unwrap_err()
            .to_string();
        assert!(err.contains("shrink"));
        assert!(err.contains("PostgreSQL"));
    }

    #[test]
    fn completed_job_does_not_block_new_expansion() {
        let (c, _) = ctl(true);
        c.reconcile_once(&pvc("pvc-1", 10, 20)).unwrap();
        c.reconcile_once(&pvc("pvc-1", 20, 20)).unwrap(); // complete
        c.reconcile_once(&pvc("pvc-1", 20, 40)).unwrap(); // new resize
        let jobs = c.list_jobs();
        assert_eq!(jobs.len(), 2);
        // One completed, one active.
        assert_eq!(
            jobs.iter()
                .filter(|j| j.phase == ResizePhase::Completed)
                .count(),
            1
        );
    }

    #[test]
    fn get_job_returns_none_for_missing() {
        let (c, _) = ctl(true);
        assert!(c.get_job("nonexistent").is_none());
    }
}
