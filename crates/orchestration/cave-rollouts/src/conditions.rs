// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rollout condition computation — Argo Rollouts parity.
//!
//! Pure-function port of `utils/conditions/conditions.go` (argoproj/argo-rollouts
//! v1.9.0): the boolean predicates the controller uses to decide whether a
//! Rollout is *Complete*, *Healthy*, or has *TimedOut*, plus the condition
//! constructor. These drive `.status.conditions`; the controller owns the live
//! reconcile, this module owns the arithmetic.

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
