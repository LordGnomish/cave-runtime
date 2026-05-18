// SPDX-License-Identifier: AGPL-3.0-or-later
//! Maps GC controller — periodic sweep across BPF map families.
//!
//! Mirrors `pkg/maps/gc/gc.go`. The agent runs a periodic sweep of
//! conntrack, NAT, ipcache, policy, and lb maps to evict stale entries
//! that the kernel-side GC didn't reap (e.g. CT entries whose endpoints
//! are gone). This module models the dispatch surface + per-map
//! statistics report.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GcTarget {
    Conntrack,
    Nat,
    Ipcache,
    Policy,
    Lb,
    Auth,
}

impl GcTarget {
    pub fn label(self) -> &'static str {
        match self {
            GcTarget::Conntrack => "ct",
            GcTarget::Nat => "nat",
            GcTarget::Ipcache => "ipcache",
            GcTarget::Policy => "policy",
            GcTarget::Lb => "lb",
            GcTarget::Auth => "auth",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcSweepReport {
    pub target: GcTarget,
    pub scanned: u64,
    pub deleted: u64,
    pub duration_us: u64,
    pub timestamp_ns: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MapsGcError {
    #[error("target {0:?} already scheduled")]
    AlreadyScheduled(GcTarget),
    #[error("target {0:?} not scheduled")]
    NotScheduled(GcTarget),
    #[error("tenant {tenant} cannot mutate maps-gc owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct MapsGcController {
    pub tenant: TenantId,
    /// Scheduled GC interval in nanoseconds, per target.
    schedules: BTreeMap<GcTarget, u64>,
    /// Per-target last-run timestamp.
    last_run: BTreeMap<GcTarget, u64>,
    /// History of completed sweeps (latest at the back).
    history: Vec<GcSweepReport>,
}

impl MapsGcController {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            schedules: BTreeMap::new(),
            last_run: BTreeMap::new(),
            history: Vec::new(),
        }
    }

    pub fn schedule(&mut self, target: GcTarget, interval_seconds: u64) -> Result<(), MapsGcError> {
        if self.schedules.contains_key(&target) {
            return Err(MapsGcError::AlreadyScheduled(target));
        }
        self.schedules.insert(target, interval_seconds * 1_000_000_000);
        Ok(())
    }

    pub fn unschedule(&mut self, target: GcTarget) -> Result<(), MapsGcError> {
        self.schedules.remove(&target).ok_or(MapsGcError::NotScheduled(target))?;
        self.last_run.remove(&target);
        Ok(())
    }

    pub fn schedule_count(&self) -> usize {
        self.schedules.len()
    }

    /// Compute which targets are due for a sweep at `now_ns`.
    pub fn due(&self, now_ns: u64) -> Vec<GcTarget> {
        let mut out = Vec::new();
        for (target, interval) in &self.schedules {
            let due = match self.last_run.get(target) {
                None => true, // never run → due immediately
                Some(last) => now_ns.saturating_sub(*last) >= *interval,
            };
            if due {
                out.push(*target);
            }
        }
        out
    }

    /// Record a completed sweep + advance the last-run pointer.
    pub fn record(&mut self, report: GcSweepReport) {
        self.last_run.insert(report.target, report.timestamp_ns);
        self.history.push(report);
    }

    pub fn last_report_for(&self, target: GcTarget) -> Option<&GcSweepReport> {
        self.history.iter().rev().find(|r| r.target == target)
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Aggregate scanned/deleted across all reports for a target.
    pub fn totals_for(&self, target: GcTarget) -> (u64, u64) {
        let mut scanned = 0u64;
        let mut deleted = 0u64;
        for r in &self.history {
            if r.target == target {
                scanned += r.scanned;
                deleted += r.deleted;
            }
        }
        (scanned, deleted)
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/maps/gc/gc.go", "Controller");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn ctrl(tenant: TenantId) -> MapsGcController {
        MapsGcController::new(tenant)
    }

    fn report(target: GcTarget, scanned: u64, deleted: u64, ts: u64) -> GcSweepReport {
        GcSweepReport { target, scanned, deleted, duration_us: 100, timestamp_ns: ts }
    }

    // ── Target labels ──────────────────────────────────────────────────────

    #[test]
    fn target_labels_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Target.Label", "tenant-mg-l");
        assert_eq!(GcTarget::Conntrack.label(), "ct");
        assert_eq!(GcTarget::Nat.label(), "nat");
        assert_eq!(GcTarget::Ipcache.label(), "ipcache");
        assert_eq!(GcTarget::Policy.label(), "policy");
        assert_eq!(GcTarget::Lb.label(), "lb");
        assert_eq!(GcTarget::Auth.label(), "auth");
    }

    // ── Schedule ───────────────────────────────────────────────────────────

    #[test]
    fn schedule_records_target() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Schedule", "tenant-mg-s");
        let mut c = ctrl(tenant);
        c.schedule(GcTarget::Conntrack, 60).unwrap();
        assert_eq!(c.schedule_count(), 1);
    }

    #[test]
    fn schedule_duplicate_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Schedule.Duplicate", "tenant-mg-sd");
        let mut c = ctrl(tenant);
        c.schedule(GcTarget::Conntrack, 60).unwrap();
        let err = c.schedule(GcTarget::Conntrack, 30).unwrap_err();
        assert!(matches!(err, MapsGcError::AlreadyScheduled(_)));
    }

    #[test]
    fn unschedule_drops_target() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Unschedule", "tenant-mg-u");
        let mut c = ctrl(tenant);
        c.schedule(GcTarget::Conntrack, 60).unwrap();
        c.unschedule(GcTarget::Conntrack).unwrap();
        assert_eq!(c.schedule_count(), 0);
    }

    #[test]
    fn unschedule_unknown_returns_not_scheduled() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Unschedule.NotScheduled", "tenant-mg-uns");
        let mut c = ctrl(tenant);
        let err = c.unschedule(GcTarget::Conntrack).unwrap_err();
        assert!(matches!(err, MapsGcError::NotScheduled(_)));
    }

    // ── Due ────────────────────────────────────────────────────────────────

    #[test]
    fn due_returns_target_after_interval_elapsed() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Due", "tenant-mg-d");
        let mut c = ctrl(tenant);
        c.schedule(GcTarget::Conntrack, 60).unwrap();
        c.record(report(GcTarget::Conntrack, 100, 5, 0));
        let due = c.due(60_000_000_000 + 1);
        assert_eq!(due, vec![GcTarget::Conntrack]);
    }

    #[test]
    fn due_empty_within_interval() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Due.NotYet", "tenant-mg-dny");
        let mut c = ctrl(tenant);
        c.schedule(GcTarget::Conntrack, 60).unwrap();
        c.record(report(GcTarget::Conntrack, 100, 5, 0));
        let due = c.due(30_000_000_000);
        assert!(due.is_empty());
    }

    #[test]
    fn due_first_run_immediate() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Due.FirstRun", "tenant-mg-df");
        let mut c = ctrl(tenant);
        c.schedule(GcTarget::Conntrack, 60).unwrap();
        let due = c.due(0);
        assert_eq!(due, vec![GcTarget::Conntrack]);
    }

    // ── Record / lookup ────────────────────────────────────────────────────

    #[test]
    fn record_advances_last_run() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Record.LastRun", "tenant-mg-rlr");
        let mut c = ctrl(tenant);
        c.schedule(GcTarget::Conntrack, 60).unwrap();
        c.record(report(GcTarget::Conntrack, 100, 5, 1000));
        let due = c.due(5_000_000_000); // 5s, less than 60s.
        assert!(due.is_empty());
    }

    #[test]
    fn last_report_for_returns_most_recent() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "LastReport", "tenant-mg-lr");
        let mut c = ctrl(tenant);
        c.record(report(GcTarget::Conntrack, 100, 5, 100));
        c.record(report(GcTarget::Conntrack, 200, 10, 200));
        let r = c.last_report_for(GcTarget::Conntrack).unwrap();
        assert_eq!(r.scanned, 200);
        assert_eq!(r.timestamp_ns, 200);
    }

    #[test]
    fn last_report_for_unknown_target_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "LastReport.NotFound", "tenant-mg-lrnf");
        let c = ctrl(tenant);
        assert!(c.last_report_for(GcTarget::Auth).is_none());
    }

    // ── Totals ─────────────────────────────────────────────────────────────

    #[test]
    fn totals_for_aggregates_across_history() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Totals", "tenant-mg-t");
        let mut c = ctrl(tenant);
        c.record(report(GcTarget::Conntrack, 100, 5, 100));
        c.record(report(GcTarget::Conntrack, 200, 10, 200));
        c.record(report(GcTarget::Nat, 50, 1, 100));
        let (scanned, deleted) = c.totals_for(GcTarget::Conntrack);
        assert_eq!(scanned, 300);
        assert_eq!(deleted, 15);
    }

    #[test]
    fn totals_for_target_with_no_history_returns_zero() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Totals.Zero", "tenant-mg-tz");
        let c = ctrl(tenant);
        assert_eq!(c.totals_for(GcTarget::Conntrack), (0, 0));
    }

    #[test]
    fn history_len_tracks_records() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "History.Len", "tenant-mg-hl");
        let mut c = ctrl(tenant);
        for i in 0..5u64 {
            c.record(report(GcTarget::Conntrack, 100, 5, i * 100));
        }
        assert_eq!(c.history_len(), 5);
    }

    // ── Multi-target ───────────────────────────────────────────────────────

    #[test]
    fn multiple_targets_have_independent_schedules() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "MultiTarget", "tenant-mg-mt");
        let mut c = ctrl(tenant);
        c.schedule(GcTarget::Conntrack, 60).unwrap();
        c.schedule(GcTarget::Nat, 30).unwrap();
        c.record(report(GcTarget::Conntrack, 1, 0, 0));
        c.record(report(GcTarget::Nat, 1, 0, 0));
        // After 31s, only NAT is due (conntrack interval is 60s).
        let due = c.due(31_000_000_000);
        assert_eq!(due, vec![GcTarget::Nat]);
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn gc_target_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Target.Serde", "tenant-mg-tserde");
        for t in [
            GcTarget::Conntrack, GcTarget::Nat, GcTarget::Ipcache,
            GcTarget::Policy, GcTarget::Lb, GcTarget::Auth,
        ] {
            let s = serde_json::to_string(&t).unwrap();
            let back: GcTarget = serde_json::from_str(&s).unwrap();
            assert_eq!(back, t);
        }
    }

    #[test]
    fn sweep_report_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Report.Serde", "tenant-mg-rserde");
        let r = report(GcTarget::Ipcache, 50, 2, 1000);
        let s = serde_json::to_string(&r).unwrap();
        let back: GcSweepReport = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    // ── End-to-end ─────────────────────────────────────────────────────────

    #[test]
    fn full_lifecycle_schedule_run_record() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/gc/gc.go", "Lifecycle", "tenant-mg-lc");
        let mut c = ctrl(tenant);
        c.schedule(GcTarget::Conntrack, 60).unwrap();
        let now = 0u64;
        let due = c.due(now);
        assert_eq!(due, vec![GcTarget::Conntrack]);
        c.record(report(GcTarget::Conntrack, 100, 5, now));
        // Immediately re-check: not due.
        let due_after = c.due(now + 1);
        assert!(due_after.is_empty());
    }
}
