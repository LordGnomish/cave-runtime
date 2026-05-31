// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rollout condition computation — Argo Rollouts parity.
//!
//! Pure-function port of `utils/conditions/conditions.go` (argoproj/argo-rollouts
//! v1.9.0): the boolean predicates the controller uses to decide whether a
//! Rollout is *Complete*, *Healthy*, or has *TimedOut*, plus the condition
//! constructor. These drive `.status.conditions`; the controller owns the live
//! reconcile, this module owns the arithmetic.

use crate::models::RolloutCondition;
use chrono::{DateTime, Duration, Utc};

/// Condition reasons mirrored from upstream `utils/conditions`.
pub const TIMED_OUT_REASON: &str = "ProgressDeadlineExceeded";
/// Reason set on the Progressing condition when a Rollout is aborted.
pub const ROLLOUT_ABORTED_REASON: &str = "RolloutAborted";
/// Reason set on the Progressing condition while a Rollout is paused.
pub const ROLLOUT_PAUSED_REASON: &str = "RolloutPaused";

/// Replica tallies a Rollout's status carries, consumed by the health predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicaCounts {
    pub desired: i32,
    pub updated: i32,
    pub ready: i32,
    pub available: i32,
    pub total: i32,
}

/// `RolloutCompleted` — the stable ReplicaSet hash equals the current pod hash
/// and is non-empty (a never-rolled-out Rollout is not "complete").
pub fn rollout_complete(stable_rs: &str, current_pod_hash: &str) -> bool {
    !stable_rs.is_empty() && stable_rs == current_pod_hash
}

/// Replica-level health shared by both strategies: every replica is updated and
/// available, with no surplus (old) replicas lingering.
pub fn replicas_healthy(c: &ReplicaCounts) -> bool {
    c.updated == c.desired && c.available == c.desired && c.total == c.desired
}

/// Canary `RolloutHealthy`: replicas at desired, every step executed, and the
/// stable ReplicaSet promoted to the current pod hash.
pub fn canary_healthy(
    c: &ReplicaCounts,
    current_step_index: i32,
    step_count: i32,
    stable_rs: &str,
    current_pod_hash: &str,
) -> bool {
    replicas_healthy(c)
        && current_step_index == step_count
        && rollout_complete(stable_rs, current_pod_hash)
}

/// Blue/Green `RolloutHealthy`: replicas at desired, the active selector points
/// at the current pod hash, and — if a preview service is defined — so does the
/// preview selector.
pub fn blue_green_healthy(
    c: &ReplicaCounts,
    active_selector: &str,
    current_pod_hash: &str,
    preview_selector: &str,
    preview_defined: bool,
) -> bool {
    replicas_healthy(c)
        && active_selector == current_pod_hash
        && (!preview_defined || preview_selector == current_pod_hash)
}

/// `RolloutTimedOut`: an explicit timed-out reason short-circuits to true; an
/// aborted/paused Rollout never times out; otherwise the Progressing condition
/// has gone longer than `progress_deadline_seconds` since its last update.
pub fn rollout_timed_out(
    progressing_reason: &str,
    last_update_time: DateTime<Utc>,
    progress_deadline_seconds: i64,
    now: DateTime<Utc>,
) -> bool {
    if progressing_reason == TIMED_OUT_REASON {
        return true;
    }
    if progressing_reason == ROLLOUT_ABORTED_REASON
        || progressing_reason == ROLLOUT_PAUSED_REASON
    {
        return false;
    }
    let deadline = last_update_time + Duration::seconds(progress_deadline_seconds);
    now > deadline
}

/// `newCondition` — construct a `RolloutCondition` stamped at `now`.
pub fn new_rollout_condition(
    condition_type: &str,
    status: &str,
    reason: &str,
    message: &str,
    now: DateTime<Utc>,
) -> RolloutCondition {
    RolloutCondition {
        condition_type: condition_type.to_string(),
        status: status.to_string(),
        reason: reason.to_string(),
        message: message.to_string(),
        last_update_time: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn counts(d: i32) -> ReplicaCounts {
        ReplicaCounts {
            desired: d,
            updated: d,
            ready: d,
            available: d,
            total: d,
        }
    }

    #[test]
    fn complete_requires_nonempty_matching_stable() {
        assert!(rollout_complete("abc123", "abc123"));
        assert!(!rollout_complete("", "")); // empty stable is never complete
        assert!(!rollout_complete("abc123", "def456"));
    }

    #[test]
    fn replicas_healthy_when_all_at_desired() {
        assert!(replicas_healthy(&counts(5)));
        let mut c = counts(5);
        c.available = 4;
        assert!(!replicas_healthy(&c));
        let mut c = counts(5);
        c.total = 7; // old replicas still around
        assert!(!replicas_healthy(&c));
    }

    #[test]
    fn canary_healthy_needs_steps_done_and_stable_promoted() {
        assert!(canary_healthy(&counts(3), 4, 4, "h", "h"));
        assert!(!canary_healthy(&counts(3), 2, 4, "h", "h")); // steps not done
        assert!(!canary_healthy(&counts(3), 4, 4, "old", "h")); // stable not promoted
    }

    #[test]
    fn blue_green_healthy_checks_active_and_optional_preview() {
        // preview not defined → only active selector must match
        assert!(blue_green_healthy(&counts(2), "h", "h", "", false));
        // preview defined and matches
        assert!(blue_green_healthy(&counts(2), "h", "h", "h", true));
        // preview defined but stale
        assert!(!blue_green_healthy(&counts(2), "h", "h", "old", true));
        // active stale
        assert!(!blue_green_healthy(&counts(2), "old", "h", "h", true));
    }

    #[test]
    fn timed_out_honours_reason_and_deadline() {
        let now = Utc::now();
        let last = now - Duration::seconds(120);
        // explicit timed-out reason short-circuits true
        assert!(rollout_timed_out(TIMED_OUT_REASON, last, 600, now));
        // aborted / paused never time out
        assert!(!rollout_timed_out(ROLLOUT_ABORTED_REASON, last, 1, now));
        assert!(!rollout_timed_out(ROLLOUT_PAUSED_REASON, last, 1, now));
        // generic progressing past the deadline
        assert!(rollout_timed_out("ReplicaSetUpdated", last, 60, now));
        // generic progressing within the deadline
        assert!(!rollout_timed_out("ReplicaSetUpdated", last, 600, now));
    }

    #[test]
    fn new_condition_stamps_now() {
        let now = Utc::now();
        let c = new_rollout_condition("Progressing", "True", "NewReplicaSetAvailable", "ok", now);
        assert_eq!(c.condition_type, "Progressing");
        assert_eq!(c.status, "True");
        assert_eq!(c.reason, "NewReplicaSetAvailable");
        assert_eq!(c.message, "ok");
        assert_eq!(c.last_update_time, now);
    }
}
