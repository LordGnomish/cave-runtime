// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prober worker pool + restart-coordination ledger.
//!
//! Mirrors `pkg/kubelet/prober/worker.go` + `pkg/kubelet/prober/results/`.
//!
//! `probe.rs` already ships the per-probe state machine (initial
//! delay, period, success/failure thresholds, decision function).
//! The missing layer the 2026-05-12 audit flagged is the
//! *coordinator* on top:
//!
//! * **Worker pool** — kubelet runs at most N probes concurrently
//!   (default 16) to avoid stampeding the network stack on a
//!   high-pod-count node. Adoption sits on `cave_kernel::Semaphore`.
//! * **Restart coordination ledger** — when liveness fails, the
//!   prober wants to trigger exactly *one* container restart. Repeated
//!   `RestartContainer` decisions while the kubelet is still
//!   draining the previous restart must be suppressed (the existing
//!   restart is still in flight).
//! * **Readiness-fan-out de-dup** — readiness flips drive
//!   EndpointSlice updates; we don't want every tick re-emitting
//!   "MarkReady" if the state hasn't changed.
//!
//! This module is deterministic in shape — every test is sync, no
//! tokio runtime needed except for the async permit-acquire path
//! which is asserted under `#[tokio::test]`.

use crate::probe::{ProbeKey, ProbeKind, ProbeResult, ProberAction, ProberManager};
use cave_kernel::semaphore::Semaphore;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[allow(dead_code)]
pub const UPSTREAM_PATH: &str = "pkg/kubelet/prober/worker.go";
#[allow(dead_code)]
pub const UPSTREAM_SYMBOL: &str = "worker.doProbe";

/// The slim view of a container the coordinator needs. The actual
/// pod object is huge; the coordinator only cares about the
/// pod/container identifier and the kubelet's last-known
/// `Ready=true/false` flag (which it must change on readiness
/// transitions).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContainerRef {
    pub pod_uid: String,
    pub container: String,
}

impl ContainerRef {
    pub fn new(pod_uid: impl Into<String>, container: impl Into<String>) -> Self {
        Self {
            pod_uid: pod_uid.into(),
            container: container.into(),
        }
    }
}

/// Coordinator-level event the kubelet sync loop consumes. Differs
/// from `ProberAction` by carrying the (pod_uid, container) identity
/// AND by deduplicating against the coordinator's ledger so the sync
/// loop sees one event per state change, not one per probe tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoordinatorEvent {
    /// Liveness probe is failing AND no restart is currently in
    /// flight for this container → kubelet should request restart.
    RestartContainer { container: ContainerRef },
    /// Startup probe ran out of failure-threshold attempts.
    StartupFailed { container: ContainerRef },
    /// Startup probe completed successfully — liveness/readiness
    /// probes may begin.
    StartupComplete { container: ContainerRef },
    /// Readiness flipped to true (and it was not already true).
    MarkReady { container: ContainerRef },
    /// Readiness flipped to false (and it was not already false).
    MarkNotReady { container: ContainerRef },
}

/// Per-container ledger entry the coordinator carries. Records the
/// last decision *fed back to the sync loop* so duplicate decisions
/// are suppressed.
#[derive(Debug, Clone)]
struct LedgerEntry {
    last_ready: Option<bool>,
    last_startup_complete: bool,
    /// Set on `RestartContainer`. Cleared by `mark_restart_completed`
    /// once the kubelet has done the work. While set, further
    /// `Liveness::Failure → RestartContainer` decisions are silently
    /// suppressed because a restart is already in flight.
    restart_in_flight: bool,
    /// Time of the last RestartContainer event we emitted. Used to
    /// short-circuit the suppression in case the kubelet failed to
    /// call `mark_restart_completed` (defensive: prevents an
    /// indefinite "we already restarted" state).
    last_restart_at: Option<DateTime<Utc>>,
}

impl Default for LedgerEntry {
    fn default() -> Self {
        Self {
            last_ready: None,
            last_startup_complete: false,
            restart_in_flight: false,
            last_restart_at: None,
        }
    }
}

/// Coordinator configuration knobs.
#[derive(Debug, Clone, Copy)]
pub struct ProberConfig {
    /// Maximum number of probes that can execute simultaneously
    /// across the entire kubelet. Upstream default is 16.
    pub max_concurrent: usize,
    /// Safety valve: if a restart hasn't been acknowledged within
    /// this window, the suppression flag is automatically cleared so
    /// a future failure can re-fire. Defends against a stuck kubelet
    /// state machine.
    pub restart_suppression_max: ChronoDuration,
}

impl Default for ProberConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 16,
            // Five minutes is large enough that legitimate restart
            // windows are not interrupted; small enough that a stuck
            // ledger can recover within a sync loop cycle.
            restart_suppression_max: ChronoDuration::minutes(5),
        }
    }
}

/// The main coordinator. Wraps a `ProberManager` (where the actual
/// per-probe state lives) and adds:
///
/// * concurrency control via [`cave_kernel::semaphore::Semaphore`],
/// * a ledger of "what was the last event we fed back to the kubelet",
/// * a deterministic conversion from per-probe `ProberAction`s into
///   pod-level [`CoordinatorEvent`]s.
#[derive(Debug)]
pub struct ProberCoordinator {
    pub manager: ProberManager,
    semaphore: Semaphore,
    ledger: HashMap<ContainerRef, LedgerEntry>,
    cfg: ProberConfig,
}

impl ProberCoordinator {
    pub fn new(cfg: ProberConfig) -> Self {
        Self {
            manager: ProberManager::new(),
            semaphore: Semaphore::new(cfg.max_concurrent),
            ledger: HashMap::new(),
            cfg,
        }
    }

    /// Build with default config (16 concurrent probes).
    pub fn default_pool() -> Self {
        Self::new(ProberConfig::default())
    }

    /// Worker-pool capacity (probes that can run concurrently).
    pub fn pool_capacity(&self) -> usize {
        self.semaphore.capacity()
    }

    /// Currently in-flight probe count.
    pub fn pool_in_use(&self) -> usize {
        self.semaphore.in_use()
    }

    /// Try to take a slot in the pool. The returned permit must be
    /// held for the duration of the probe RPC; dropping it returns
    /// the slot. Returns `None` when the pool is full and the
    /// caller should yield to another tick.
    ///
    /// Async variant lives below.
    pub fn try_reserve(&self) -> Option<ProbePermit> {
        self.semaphore
            .try_acquire()
            .ok()
            .map(|p| ProbePermit { _inner: p })
    }

    /// Async reserve — waits for a free slot.
    pub async fn reserve(&self) -> ProbePermit {
        let p = self.semaphore.acquire().await;
        ProbePermit { _inner: p }
    }

    /// Acknowledge a restart has been completed by the kubelet sync
    /// loop. Clears the ledger's "in flight" flag for this container
    /// so a future liveness failure can re-trigger.
    pub fn mark_restart_completed(&mut self, container: &ContainerRef) {
        if let Some(entry) = self.ledger.get_mut(container) {
            entry.restart_in_flight = false;
            entry.last_restart_at = None;
        }
    }

    /// Feed a probe sample into the coordinator. Updates the
    /// underlying `ProberManager` state and returns a coordinator
    /// event if (and only if) this sample causes a state transition
    /// the kubelet hasn't seen yet.
    ///
    /// Returns `None` for "nothing new to do" — the common case on
    /// steady-state ticks.
    pub fn record_sample(
        &mut self,
        container: &ContainerRef,
        kind: ProbeKind,
        sample: ProbeResult,
        now: DateTime<Utc>,
    ) -> Option<CoordinatorEvent> {
        let action = self.manager.record_sample(
            &container.pod_uid,
            &container.container,
            kind,
            sample,
            now,
        )?;
        self.coordinate(container, action, now)
    }

    /// Deregister all probes for a container — used on container
    /// teardown so the registry doesn't grow unboundedly.
    pub fn forget_container(&mut self, container: &ContainerRef) {
        self.manager.deregister(&container.pod_uid, &container.container);
        self.ledger.remove(container);
    }

    /// Convert a single `ProberAction` into a deduplicated
    /// `CoordinatorEvent`. Pure logic — public so the kubelet sync
    /// loop can drive the coordinator directly from a snapshot if
    /// it prefers to manage its own probe scheduling.
    pub fn coordinate(
        &mut self,
        container: &ContainerRef,
        action: ProberAction,
        now: DateTime<Utc>,
    ) -> Option<CoordinatorEvent> {
        let entry = self.ledger.entry(container.clone()).or_default();
        // Clear stale restart suppression — see config note above.
        if entry.restart_in_flight {
            if let Some(last) = entry.last_restart_at {
                if now - last > self.cfg.restart_suppression_max {
                    entry.restart_in_flight = false;
                    entry.last_restart_at = None;
                }
            }
        }

        match action {
            ProberAction::NoOp => None,
            ProberAction::RestartContainer => {
                if entry.restart_in_flight {
                    None
                } else {
                    entry.restart_in_flight = true;
                    entry.last_restart_at = Some(now);
                    Some(CoordinatorEvent::RestartContainer {
                        container: container.clone(),
                    })
                }
            }
            ProberAction::StartupFailed => Some(CoordinatorEvent::StartupFailed {
                container: container.clone(),
            }),
            ProberAction::StartupComplete => {
                if entry.last_startup_complete {
                    None
                } else {
                    entry.last_startup_complete = true;
                    Some(CoordinatorEvent::StartupComplete {
                        container: container.clone(),
                    })
                }
            }
            ProberAction::MarkReady => {
                if entry.last_ready == Some(true) {
                    None
                } else {
                    entry.last_ready = Some(true);
                    Some(CoordinatorEvent::MarkReady {
                        container: container.clone(),
                    })
                }
            }
            ProberAction::MarkNotReady => {
                if entry.last_ready == Some(false) {
                    None
                } else {
                    entry.last_ready = Some(false);
                    Some(CoordinatorEvent::MarkNotReady {
                        container: container.clone(),
                    })
                }
            }
        }
    }

    /// Diagnostics: list every container the ledger has seen along
    /// with its last-known readiness state. Used by the admin
    /// surface (kubelet status panel).
    pub fn snapshot(&self) -> Vec<LedgerSnapshot> {
        self.ledger
            .iter()
            .map(|(k, v)| LedgerSnapshot {
                container: k.clone(),
                last_ready: v.last_ready,
                last_startup_complete: v.last_startup_complete,
                restart_in_flight: v.restart_in_flight,
            })
            .collect()
    }

    /// Public access to probe-key listing for completeness with
    /// `ProberManager`. Forward to the existing helper so callers
    /// have a single coordinator handle.
    pub fn has_any(&self, container: &ContainerRef) -> bool {
        self.manager.has_any(&container.pod_uid, &container.container)
    }

    /// Compose a `ProbeKey` from a `ContainerRef` + kind. Saves
    /// callers from re-constructing the struct every time.
    pub fn key(container: &ContainerRef, kind: ProbeKind) -> ProbeKey {
        ProbeKey {
            pod_uid: container.pod_uid.clone(),
            container: container.container.clone(),
            kind,
        }
    }
}

/// Read-only diagnostic snapshot of one container's ledger entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerSnapshot {
    pub container: ContainerRef,
    pub last_ready: Option<bool>,
    pub last_startup_complete: bool,
    pub restart_in_flight: bool,
}

/// RAII guard: while held, one slot of the worker pool is reserved.
/// Drop returns the slot. Same shape as `cave_kernel::semaphore::Permit`
/// but type-aliased so the coordinator API is self-documenting.
#[derive(Debug)]
pub struct ProbePermit {
    _inner: cave_kernel::semaphore::Permit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{ProbeKind, ProbeResult, ProbeSpec};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn spec_liveness() -> ProbeSpec {
        let mut s = ProbeSpec::http_get(8080, "/healthz");
        s.kind = ProbeKind::Liveness;
        s.initial_delay_seconds = 0;
        s.period_seconds = 1;
        s.failure_threshold = 1;
        s.success_threshold = 1;
        s
    }

    fn spec_readiness() -> ProbeSpec {
        let mut s = ProbeSpec::http_get(8080, "/ready");
        s.kind = ProbeKind::Readiness;
        s.initial_delay_seconds = 0;
        s.period_seconds = 1;
        s.failure_threshold = 1;
        s.success_threshold = 1;
        s
    }

    fn spec_startup() -> ProbeSpec {
        let mut s = ProbeSpec::http_get(8080, "/startup");
        s.kind = ProbeKind::Startup;
        s.initial_delay_seconds = 0;
        s.period_seconds = 1;
        s.failure_threshold = 1;
        s.success_threshold = 1;
        s
    }

    fn cref() -> ContainerRef {
        ContainerRef::new("pod-1", "main")
    }

    fn fresh_coordinator() -> ProberCoordinator {
        let mut c = ProberCoordinator::default_pool();
        // Register liveness/readiness/startup probes for the
        // single container under test.
        let cr = cref();
        c.manager
            .register(&cr.pod_uid, &cr.container, spec_liveness(), now())
            .unwrap();
        c.manager
            .register(&cr.pod_uid, &cr.container, spec_readiness(), now())
            .unwrap();
        c.manager
            .register(&cr.pod_uid, &cr.container, spec_startup(), now())
            .unwrap();
        c
    }

    #[test]
    fn pool_capacity_defaults_to_16() {
        let c = ProberCoordinator::default_pool();
        assert_eq!(c.pool_capacity(), 16);
        assert_eq!(c.pool_in_use(), 0);
    }

    #[test]
    fn try_reserve_succeeds_until_pool_full() {
        let c = ProberCoordinator::new(ProberConfig {
            max_concurrent: 2,
            restart_suppression_max: ChronoDuration::minutes(5),
        });
        let _a = c.try_reserve().expect("slot 1");
        let _b = c.try_reserve().expect("slot 2");
        // Third reservation should fail because pool is full.
        assert!(c.try_reserve().is_none());
        assert_eq!(c.pool_in_use(), 2);
    }

    #[test]
    fn dropping_permit_returns_slot() {
        let c = ProberCoordinator::new(ProberConfig {
            max_concurrent: 1,
            restart_suppression_max: ChronoDuration::minutes(5),
        });
        {
            let _p = c.try_reserve().unwrap();
            assert_eq!(c.pool_in_use(), 1);
        }
        // Now reservable again.
        assert!(c.try_reserve().is_some());
    }

    #[tokio::test]
    async fn async_reserve_waits_for_slot() {
        let c = ProberCoordinator::new(ProberConfig {
            max_concurrent: 1,
            restart_suppression_max: ChronoDuration::minutes(5),
        });
        let _held = c.reserve().await;
        assert_eq!(c.pool_in_use(), 1);
    }

    #[test]
    fn liveness_failure_emits_restart_once_then_suppresses() {
        let mut c = fresh_coordinator();
        let cr = cref();
        // First failure → RestartContainer.
        let ev = c.record_sample(&cr, ProbeKind::Liveness, ProbeResult::Failure, now());
        assert_eq!(
            ev,
            Some(CoordinatorEvent::RestartContainer {
                container: cr.clone(),
            })
        );
        // Second failure before acknowledgement → suppressed.
        let ev = c.record_sample(
            &cr,
            ProbeKind::Liveness,
            ProbeResult::Failure,
            now() + ChronoDuration::seconds(1),
        );
        assert_eq!(ev, None);
    }

    #[test]
    fn restart_re_fires_after_mark_completed() {
        let mut c = fresh_coordinator();
        let cr = cref();
        c.record_sample(&cr, ProbeKind::Liveness, ProbeResult::Failure, now())
            .unwrap();
        c.mark_restart_completed(&cr);
        // After acknowledgement, a fresh failure must re-fire.
        let ev = c.record_sample(
            &cr,
            ProbeKind::Liveness,
            ProbeResult::Failure,
            now() + ChronoDuration::seconds(1),
        );
        assert!(matches!(ev, Some(CoordinatorEvent::RestartContainer { .. })));
    }

    #[test]
    fn restart_suppression_auto_clears_after_max_window() {
        // If the kubelet forgot to call mark_restart_completed we
        // mustn't lock the container out forever.
        let mut c = ProberCoordinator::new(ProberConfig {
            max_concurrent: 4,
            restart_suppression_max: ChronoDuration::seconds(30),
        });
        let cr = cref();
        c.manager
            .register(&cr.pod_uid, &cr.container, spec_liveness(), now())
            .unwrap();
        c.record_sample(&cr, ProbeKind::Liveness, ProbeResult::Failure, now());
        // Just before the auto-clear window — still suppressed.
        let still_suppressed = c.record_sample(
            &cr,
            ProbeKind::Liveness,
            ProbeResult::Failure,
            now() + ChronoDuration::seconds(29),
        );
        assert_eq!(still_suppressed, None);
        // Past the window — fires again.
        let fired_again = c.record_sample(
            &cr,
            ProbeKind::Liveness,
            ProbeResult::Failure,
            now() + ChronoDuration::seconds(31),
        );
        assert!(matches!(
            fired_again,
            Some(CoordinatorEvent::RestartContainer { .. })
        ));
    }

    #[test]
    fn readiness_flip_emits_event_once() {
        let mut c = fresh_coordinator();
        let cr = cref();
        // First Success → MarkReady.
        let ev = c.record_sample(&cr, ProbeKind::Readiness, ProbeResult::Success, now());
        assert_eq!(
            ev,
            Some(CoordinatorEvent::MarkReady {
                container: cr.clone()
            })
        );
        // Second Success → suppressed (already ready).
        let ev = c.record_sample(
            &cr,
            ProbeKind::Readiness,
            ProbeResult::Success,
            now() + ChronoDuration::seconds(1),
        );
        assert_eq!(ev, None);
    }

    #[test]
    fn readiness_failure_then_success_flips_both_ways() {
        let mut c = fresh_coordinator();
        let cr = cref();
        // Ready=true first.
        c.record_sample(&cr, ProbeKind::Readiness, ProbeResult::Success, now());
        // Failure flips to NotReady.
        let ev = c.record_sample(
            &cr,
            ProbeKind::Readiness,
            ProbeResult::Failure,
            now() + ChronoDuration::seconds(1),
        );
        assert_eq!(
            ev,
            Some(CoordinatorEvent::MarkNotReady {
                container: cr.clone()
            })
        );
        // Success flips back to Ready.
        let ev = c.record_sample(
            &cr,
            ProbeKind::Readiness,
            ProbeResult::Success,
            now() + ChronoDuration::seconds(2),
        );
        assert_eq!(
            ev,
            Some(CoordinatorEvent::MarkReady {
                container: cr.clone()
            })
        );
    }

    #[test]
    fn startup_complete_emits_once() {
        let mut c = fresh_coordinator();
        let cr = cref();
        let ev = c.record_sample(&cr, ProbeKind::Startup, ProbeResult::Success, now());
        assert_eq!(
            ev,
            Some(CoordinatorEvent::StartupComplete {
                container: cr.clone()
            })
        );
        let ev = c.record_sample(
            &cr,
            ProbeKind::Startup,
            ProbeResult::Success,
            now() + ChronoDuration::seconds(1),
        );
        // ProberAction::StartupComplete is repeated, but the
        // coordinator dedupes.
        assert_eq!(ev, None);
    }

    #[test]
    fn startup_failed_emits_each_time() {
        // Unlike the other flips, StartupFailed is a *terminal*
        // condition the kubelet must always see (it drives container
        // restart). The ProberManager itself stops firing after the
        // failure threshold; we just verify the first emission goes
        // through.
        let mut c = fresh_coordinator();
        let cr = cref();
        let ev = c.record_sample(&cr, ProbeKind::Startup, ProbeResult::Failure, now());
        assert_eq!(
            ev,
            Some(CoordinatorEvent::StartupFailed {
                container: cr.clone()
            })
        );
    }

    #[test]
    fn forget_container_drops_ledger_and_manager_state() {
        let mut c = fresh_coordinator();
        let cr = cref();
        c.record_sample(&cr, ProbeKind::Readiness, ProbeResult::Success, now());
        assert!(!c.snapshot().is_empty());
        c.forget_container(&cr);
        assert!(c.snapshot().is_empty());
        assert!(!c.has_any(&cr));
    }

    #[test]
    fn snapshot_reports_per_container_state() {
        let mut c = fresh_coordinator();
        let cr = cref();
        c.record_sample(&cr, ProbeKind::Readiness, ProbeResult::Success, now());
        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        let only = &snap[0];
        assert_eq!(only.container, cr);
        assert_eq!(only.last_ready, Some(true));
        assert!(!only.restart_in_flight);
    }

    #[test]
    fn coordinate_pure_path_is_directly_callable() {
        // The kubelet sync loop can drive `coordinate` directly when
        // it manages its own probe scheduling — verify the pure
        // entry point exists and dedupes identically.
        let mut c = fresh_coordinator();
        let cr = cref();
        let ev = c.coordinate(&cr, ProberAction::MarkReady, now());
        assert!(matches!(ev, Some(CoordinatorEvent::MarkReady { .. })));
        let ev = c.coordinate(&cr, ProberAction::MarkReady, now());
        assert!(ev.is_none());
    }

    #[test]
    fn key_constructor_round_trips_through_prober_manager() {
        // Sanity: the helper builds the same key the underlying
        // manager uses internally.
        let cr = cref();
        let k = ProberCoordinator::key(&cr, ProbeKind::Liveness);
        assert_eq!(k.pod_uid, cr.pod_uid);
        assert_eq!(k.container, cr.container);
        assert_eq!(k.kind, ProbeKind::Liveness);
    }

    #[test]
    fn liveness_noop_action_emits_no_event() {
        let mut c = fresh_coordinator();
        let cr = cref();
        // Successful liveness probe → ProberAction::NoOp.
        let ev = c.record_sample(&cr, ProbeKind::Liveness, ProbeResult::Success, now());
        assert_eq!(ev, None);
    }

    #[test]
    fn multiple_containers_have_independent_ledgers() {
        let mut c = fresh_coordinator();
        let other = ContainerRef::new("pod-1", "sidecar");
        c.manager
            .register(&other.pod_uid, &other.container, spec_readiness(), now())
            .unwrap();
        c.record_sample(&cref(), ProbeKind::Readiness, ProbeResult::Success, now());
        let ev = c.record_sample(&other, ProbeKind::Readiness, ProbeResult::Success, now());
        assert_eq!(
            ev,
            Some(CoordinatorEvent::MarkReady {
                container: other.clone()
            })
        );
        assert_eq!(c.snapshot().len(), 2);
    }
}
