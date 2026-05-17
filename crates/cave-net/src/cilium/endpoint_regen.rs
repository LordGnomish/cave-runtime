// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Endpoint regeneration controller — per-endpoint BPF program rebuild.
//!
//! Mirrors `pkg/endpoint/policy.go::Endpoint.Regenerate` and the work
//! queue in `pkg/endpoint/regeneration_queue.go`. When a policy or
//! identity event affects an endpoint, the agent enqueues a
//! regeneration request; the controller dedupes, prioritises, and
//! executes the rebuild that recompiles the per-endpoint BPF object.
//!
//! Semantics (faithful to upstream):
//!
//! * Multiple `RegenRequest`s for the same endpoint coalesce into a
//!   single in-flight task — the request with the higher
//!   `RegenLevel` wins (e.g. PolicyRecompute > Datapath > Maps).
//! * The work queue processes one request per endpoint at a time;
//!   while a regeneration is in flight, follow-up enqueues are stored
//!   and applied after completion.
//! * The controller records the last result per endpoint
//!   (`Pending`, `Success`, `Failure(reason)`) so observability
//!   (`cilium endpoint regenerations`) can surface stuck endpoints.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RegenLevel {
    /// Reload the BPF maps only — cheapest. Mirrors `RegenerateWithoutDatapath`.
    Maps = 0,
    /// Recompile the datapath without touching the policy. Mirrors
    /// `RegenerateWithDatapathRebuild`.
    Datapath = 1,
    /// Full policy recompute + datapath rebuild. Mirrors
    /// `RegenerateWithDatapathRewrite`.
    PolicyRecompute = 2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegenRequest {
    pub endpoint_id: u64,
    pub level: RegenLevel,
    pub reason: String,
    pub enqueued_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegenStatus {
    Pending,
    InFlight { started_ns: u64, level: RegenLevel },
    Success { completed_ns: u64, level: RegenLevel, duration_ns: u64 },
    Failure { failed_ns: u64, level: RegenLevel, reason: String },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegenError {
    #[error("endpoint id {0} has no in-flight regeneration")]
    NotInFlight(u64),
    #[error("endpoint id {0} already in flight at level {1:?}")]
    AlreadyInFlight(u64, RegenLevel),
    #[error("endpoint id {0} not found")]
    NotFound(u64),
    #[error("tenant {tenant} cannot mutate regen queue owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct RegenController {
    pub tenant: TenantId,
    /// Per-endpoint pending request — coalesced.
    pending: HashMap<u64, RegenRequest>,
    /// Order of endpoints waiting to be processed.
    queue: VecDeque<u64>,
    /// Per-endpoint in-flight task.
    in_flight: HashMap<u64, RegenRequest>,
    /// Last status per endpoint (success/failure). Pending/InFlight
    /// are visible via the other maps directly.
    history: BTreeMap<u64, RegenStatus>,
    /// Counters for observability.
    pub completed: u64,
    pub failed: u64,
}

impl RegenController {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            pending: HashMap::new(),
            queue: VecDeque::new(),
            in_flight: HashMap::new(),
            history: BTreeMap::new(),
            completed: 0,
            failed: 0,
        }
    }

    /// Enqueue a request. Coalesces with any existing pending request
    /// for the same endpoint, keeping the higher `level`. Returns the
    /// resulting request.
    pub fn enqueue(&mut self, req: RegenRequest) -> RegenRequest {
        let merged = match self.pending.remove(&req.endpoint_id) {
            Some(existing) => RegenRequest {
                endpoint_id: req.endpoint_id,
                level: existing.level.max(req.level),
                reason: req.reason.clone(),
                enqueued_ns: req.enqueued_ns,
            },
            None => req.clone(),
        };
        // Keep the queue ordering — only push if not already there.
        if !self.queue.contains(&merged.endpoint_id) {
            self.queue.push_back(merged.endpoint_id);
        }
        self.pending.insert(merged.endpoint_id, merged.clone());
        self.history.insert(merged.endpoint_id, RegenStatus::Pending);
        merged
    }

    /// Pop the next request to process. Returns None if the queue is empty.
    /// Endpoints with an active in-flight task are skipped.
    pub fn pop_for_processing(&mut self, now_ns: u64) -> Option<RegenRequest> {
        while let Some(eid) = self.queue.pop_front() {
            if self.in_flight.contains_key(&eid) {
                // Skip; will be retried after the in-flight completes.
                self.queue.push_back(eid);
                return None;
            }
            if let Some(req) = self.pending.remove(&eid) {
                self.in_flight.insert(eid, req.clone());
                self.history.insert(eid, RegenStatus::InFlight { started_ns: now_ns, level: req.level });
                return Some(req);
            }
        }
        None
    }

    /// Mark an in-flight regeneration as successful.
    pub fn complete(&mut self, endpoint_id: u64, completed_ns: u64) -> Result<(), RegenError> {
        let req = self.in_flight.remove(&endpoint_id).ok_or(RegenError::NotInFlight(endpoint_id))?;
        self.history.insert(endpoint_id, RegenStatus::Success {
            completed_ns,
            level: req.level,
            duration_ns: completed_ns.saturating_sub(req.enqueued_ns),
        });
        self.completed += 1;
        // If a follow-up was enqueued during the in-flight, push it onto the queue.
        if self.pending.contains_key(&endpoint_id) && !self.queue.contains(&endpoint_id) {
            self.queue.push_back(endpoint_id);
        }
        Ok(())
    }

    /// Mark an in-flight regeneration as failed.
    pub fn fail(&mut self, endpoint_id: u64, failed_ns: u64, reason: impl Into<String>) -> Result<(), RegenError> {
        let req = self.in_flight.remove(&endpoint_id).ok_or(RegenError::NotInFlight(endpoint_id))?;
        self.history.insert(endpoint_id, RegenStatus::Failure {
            failed_ns, level: req.level, reason: reason.into(),
        });
        self.failed += 1;
        if self.pending.contains_key(&endpoint_id) && !self.queue.contains(&endpoint_id) {
            self.queue.push_back(endpoint_id);
        }
        Ok(())
    }

    pub fn status(&self, endpoint_id: u64) -> Option<&RegenStatus> {
        self.history.get(&endpoint_id)
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }

    pub fn queue_depth(&self) -> usize {
        self.queue.len()
    }

    /// Stuck endpoints: in-flight for longer than `threshold_ns`.
    pub fn stuck_endpoints(&self, now_ns: u64, threshold_ns: u64) -> BTreeSet<u64> {
        self.history.iter()
            .filter_map(|(eid, status)| match status {
                RegenStatus::InFlight { started_ns, .. } if now_ns.saturating_sub(*started_ns) >= threshold_ns => Some(*eid),
                _ => None,
            })
            .collect()
    }

    /// Forget an endpoint entirely (e.g. when the pod is deleted).
    pub fn forget(&mut self, endpoint_id: u64) -> Result<(), RegenError> {
        let mut found = false;
        if self.pending.remove(&endpoint_id).is_some() {
            found = true;
        }
        if self.in_flight.remove(&endpoint_id).is_some() {
            found = true;
        }
        if self.history.remove(&endpoint_id).is_some() {
            found = true;
        }
        self.queue.retain(|e| *e != endpoint_id);
        if !found {
            return Err(RegenError::NotFound(endpoint_id));
        }
        Ok(())
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/endpoint/policy.go", "Endpoint.Regenerate");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn ctrl(tenant: TenantId) -> RegenController {
        RegenController::new(tenant)
    }

    fn req(eid: u64, level: RegenLevel, ns: u64, reason: &str) -> RegenRequest {
        RegenRequest { endpoint_id: eid, level, reason: reason.into(), enqueued_ns: ns }
    }

    // ── RegenLevel ordering ─────────────────────────────────────────────────

    #[test]
    fn regen_level_ordering_maps_lt_datapath_lt_policy() {
        let (_c, _t) = cilium_test_ctx!("pkg/endpoint/policy.go", "RegenLevel.Order", "tenant-rg-ord");
        assert!(RegenLevel::Maps < RegenLevel::Datapath);
        assert!(RegenLevel::Datapath < RegenLevel::PolicyRecompute);
    }

    // ── Enqueue ─────────────────────────────────────────────────────────────

    #[test]
    fn enqueue_records_pending_and_queue_position() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Enqueue", "tenant-rg-eq");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "policy-update"));
        assert_eq!(c.pending_count(), 1);
        assert_eq!(c.queue_depth(), 1);
        assert!(matches!(c.status(1), Some(RegenStatus::Pending)));
    }

    #[test]
    fn enqueue_coalesces_to_higher_level() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Enqueue.Coalesce", "tenant-rg-coal");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "first"));
        let merged = c.enqueue(req(1, RegenLevel::PolicyRecompute, 200, "policy"));
        assert_eq!(merged.level, RegenLevel::PolicyRecompute);
        assert_eq!(c.pending_count(), 1);
        assert_eq!(c.queue_depth(), 1);
    }

    #[test]
    fn enqueue_lower_level_does_not_downgrade() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Enqueue.NoDowngrade", "tenant-rg-cdn");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::PolicyRecompute, 100, "high"));
        let merged = c.enqueue(req(1, RegenLevel::Maps, 200, "low"));
        assert_eq!(merged.level, RegenLevel::PolicyRecompute);
    }

    #[test]
    fn enqueue_distinct_endpoints_create_distinct_queue_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Enqueue.Distinct", "tenant-rg-dist");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        c.enqueue(req(2, RegenLevel::Maps, 100, "y"));
        c.enqueue(req(3, RegenLevel::Maps, 100, "z"));
        assert_eq!(c.queue_depth(), 3);
    }

    // ── Pop / process ───────────────────────────────────────────────────────

    #[test]
    fn pop_returns_first_request_and_marks_in_flight() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Pop.InFlight", "tenant-rg-pif");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        let popped = c.pop_for_processing(150).unwrap();
        assert_eq!(popped.endpoint_id, 1);
        assert_eq!(c.in_flight_count(), 1);
        assert_eq!(c.pending_count(), 0);
    }

    #[test]
    fn pop_empty_queue_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Pop.Empty", "tenant-rg-pe");
        let mut c = ctrl(tenant);
        assert!(c.pop_for_processing(100).is_none());
    }

    #[test]
    fn pop_skips_in_flight_endpoint() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Pop.SkipInFlight", "tenant-rg-pskip");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        let _ = c.pop_for_processing(150).unwrap();
        // Re-enqueue while in flight.
        c.enqueue(req(1, RegenLevel::Maps, 200, "y"));
        // Popping should not return the in-flight endpoint again.
        let next = c.pop_for_processing(200);
        assert!(next.is_none());
    }

    // ── Complete ────────────────────────────────────────────────────────────

    #[test]
    fn complete_records_success_and_duration() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Complete", "tenant-rg-cmp");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Datapath, 100, "x"));
        let _ = c.pop_for_processing(150).unwrap();
        c.complete(1, 200).unwrap();
        match c.status(1).unwrap() {
            RegenStatus::Success { duration_ns, level, .. } => {
                assert_eq!(*duration_ns, 100);
                assert_eq!(*level, RegenLevel::Datapath);
            }
            other => panic!("expected Success, got {other:?}"),
        }
        assert_eq!(c.completed, 1);
    }

    #[test]
    fn complete_for_unknown_endpoint_returns_not_in_flight() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Complete.NotInFlight", "tenant-rg-cnf");
        let mut c = ctrl(tenant);
        let err = c.complete(99, 100).unwrap_err();
        assert_eq!(err, RegenError::NotInFlight(99));
    }

    #[test]
    fn complete_followed_by_pending_re_enqueues() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Complete.PendingRequeue", "tenant-rg-creq");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        let _ = c.pop_for_processing(150).unwrap();
        // While in flight, enqueue a follow-up.
        c.enqueue(req(1, RegenLevel::PolicyRecompute, 200, "policy"));
        c.complete(1, 250).unwrap();
        // The follow-up should now be re-queued.
        assert_eq!(c.queue_depth(), 1);
        assert_eq!(c.pending_count(), 1);
    }

    // ── Fail ────────────────────────────────────────────────────────────────

    #[test]
    fn fail_records_failure_with_reason() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Fail", "tenant-rg-fail");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Datapath, 100, "x"));
        let _ = c.pop_for_processing(150).unwrap();
        c.fail(1, 200, "verifier error").unwrap();
        match c.status(1).unwrap() {
            RegenStatus::Failure { reason, .. } => assert_eq!(reason, "verifier error"),
            other => panic!("expected Failure, got {other:?}"),
        }
        assert_eq!(c.failed, 1);
    }

    #[test]
    fn fail_for_unknown_endpoint_returns_not_in_flight() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Fail.NotInFlight", "tenant-rg-fnf");
        let mut c = ctrl(tenant);
        let err = c.fail(99, 100, "x").unwrap_err();
        assert_eq!(err, RegenError::NotInFlight(99));
    }

    // ── Stuck endpoints ─────────────────────────────────────────────────────

    #[test]
    fn stuck_endpoints_returns_those_past_threshold() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Stuck", "tenant-rg-stk");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        c.enqueue(req(2, RegenLevel::Maps, 100, "y"));
        let _ = c.pop_for_processing(150);
        let _ = c.pop_for_processing(200);
        let stuck = c.stuck_endpoints(1000, 500);
        assert_eq!(stuck.len(), 2);
        assert!(stuck.contains(&1));
        assert!(stuck.contains(&2));
    }

    #[test]
    fn stuck_endpoints_excludes_recent_in_flight() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Stuck.NotYet", "tenant-rg-stkny");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        let _ = c.pop_for_processing(150);
        let stuck = c.stuck_endpoints(200, 1000);
        assert!(stuck.is_empty());
    }

    #[test]
    fn stuck_endpoints_excludes_completed() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Stuck.Excludes", "tenant-rg-stkc");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        let _ = c.pop_for_processing(150);
        c.complete(1, 200).unwrap();
        let stuck = c.stuck_endpoints(10_000, 100);
        assert!(stuck.is_empty());
    }

    // ── Forget ──────────────────────────────────────────────────────────────

    #[test]
    fn forget_pending_endpoint_drops_from_queue() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Forget", "tenant-rg-fgt");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        c.forget(1).unwrap();
        assert_eq!(c.pending_count(), 0);
        assert_eq!(c.queue_depth(), 0);
        assert!(c.status(1).is_none());
    }

    #[test]
    fn forget_in_flight_endpoint_drops_state() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Forget.InFlight", "tenant-rg-fgtif");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        let _ = c.pop_for_processing(150);
        c.forget(1).unwrap();
        assert_eq!(c.in_flight_count(), 0);
    }

    #[test]
    fn forget_unknown_endpoint_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Forget.NotFound", "tenant-rg-fgtnf");
        let mut c = ctrl(tenant);
        let err = c.forget(99).unwrap_err();
        assert_eq!(err, RegenError::NotFound(99));
    }

    // ── Multi-endpoint pipelining ───────────────────────────────────────────

    #[test]
    fn multiple_endpoints_processed_in_order() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Pipeline", "tenant-rg-pipe");
        let mut c = ctrl(tenant);
        c.enqueue(req(1, RegenLevel::Maps, 100, "x"));
        c.enqueue(req(2, RegenLevel::Maps, 100, "y"));
        c.enqueue(req(3, RegenLevel::Maps, 100, "z"));
        let a = c.pop_for_processing(101).unwrap();
        let b = c.pop_for_processing(102).unwrap();
        let cc = c.pop_for_processing(103).unwrap();
        assert_eq!((a.endpoint_id, b.endpoint_id, cc.endpoint_id), (1, 2, 3));
    }

    #[test]
    fn complete_increments_completed_counter() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Counter.Complete", "tenant-rg-cnt");
        let mut c = ctrl(tenant);
        for i in 1..=3u64 {
            c.enqueue(req(i, RegenLevel::Maps, 100, "x"));
            let _ = c.pop_for_processing(i * 10);
            c.complete(i, i * 10 + 5).unwrap();
        }
        assert_eq!(c.completed, 3);
        assert_eq!(c.failed, 0);
    }

    #[test]
    fn fail_increments_failed_counter() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Counter.Fail", "tenant-rg-cntf");
        let mut c = ctrl(tenant);
        for i in 1..=2u64 {
            c.enqueue(req(i, RegenLevel::Maps, 100, "x"));
            let _ = c.pop_for_processing(i * 10);
            c.fail(i, i * 10 + 5, "x").unwrap();
        }
        assert_eq!(c.failed, 2);
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn regen_request_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Request.Serde", "tenant-rg-rserde");
        let r = req(1, RegenLevel::PolicyRecompute, 100, "policy");
        let s = serde_json::to_string(&r).unwrap();
        let back: RegenRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn regen_status_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Status.Serde", "tenant-rg-sserde");
        let st = RegenStatus::Success { completed_ns: 100, level: RegenLevel::Datapath, duration_ns: 50 };
        let s = serde_json::to_string(&st).unwrap();
        let back: RegenStatus = serde_json::from_str(&s).unwrap();
        assert_eq!(back, st);
    }

    #[test]
    fn regen_level_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/endpoint/regeneration_queue.go", "Level.Serde", "tenant-rg-lserde");
        for l in [RegenLevel::Maps, RegenLevel::Datapath, RegenLevel::PolicyRecompute] {
            let s = serde_json::to_string(&l).unwrap();
            let back: RegenLevel = serde_json::from_str(&s).unwrap();
            assert_eq!(back, l);
        }
    }
}
