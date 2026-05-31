// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Leader-election decision core — a pure-Rust port of Kubernetes
//! client-go `tools/leaderelection`. Kamaji's manager runs with
//! `LeaderElection: true` so that exactly one replica drives
//! TenantControlPlane reconciliation; this module owns the lease
//! acquire/renew/observe decision (`tryAcquireOrRenew`), the renew
//! deadline check the run loop uses to step down, and the config
//! validation client-go performs at construction. The actual Lease
//! object I/O (coordination.k8s.io) is left to the runtime; this is the
//! deterministic decision layer that is unit-testable on its own.

/// The persisted lease state — mirrors client-go's `LeaderElectionRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderElectionRecord {
    /// Identity of the current holder (empty means unheld).
    pub holder_identity: String,
    /// Duration (seconds) a lease is valid past its last renewal.
    pub lease_duration_seconds: u64,
    /// Unix seconds at which the current holder first acquired the lease.
    pub acquire_time: i64,
    /// Unix seconds of the most recent renewal.
    pub renew_time: i64,
    /// Monotonic count of leadership hand-offs.
    pub leader_transitions: u64,
}

/// Outcome of a single `try_acquire_or_renew` decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// No record existed; we created one and became leader.
    Created,
    /// A prior record existed (unheld or expired); we took the lease.
    Acquired,
    /// We already held the lease and renewed it.
    Renewed,
    /// A valid lease is held by someone else; we did not acquire.
    Lost,
}

/// Decide whether `identity` may acquire or renew the lease at `now`
/// (unix seconds), given the currently-observed record (if any).
///
/// Faithful to client-go `tryAcquireOrRenew`:
/// * no record → create and acquire;
/// * record held by another **and still valid** → lose;
/// * we are the holder → renew, preserving `acquire_time` and the
///   transition count;
/// * otherwise (unheld or expired) → acquire, incrementing the
///   transition count.
///
/// Returns the outcome and the record that should be persisted. On
/// [`Outcome::Lost`] the observed record is returned unchanged.
pub fn try_acquire_or_renew(
    now: i64,
    identity: &str,
    lease_duration_seconds: u64,
    observed: Option<&LeaderElectionRecord>,
) -> (Outcome, LeaderElectionRecord) {
    let mut desired = LeaderElectionRecord {
        holder_identity: identity.to_string(),
        lease_duration_seconds,
        acquire_time: now,
        renew_time: now,
        leader_transitions: 0,
    };

    match observed {
        None => (Outcome::Created, desired),
        Some(old) => {
            let we_are_holder = old.holder_identity == identity;
            let expires_at = old.renew_time + old.lease_duration_seconds as i64;
            let still_valid = now < expires_at;

            if !old.holder_identity.is_empty() && still_valid && !we_are_holder {
                return (Outcome::Lost, old.clone());
            }

            if we_are_holder {
                desired.acquire_time = old.acquire_time;
                desired.leader_transitions = old.leader_transitions;
                (Outcome::Renewed, desired)
            } else {
                desired.leader_transitions = old.leader_transitions + 1;
                (Outcome::Acquired, desired)
            }
        }
    }
}

/// Whether a sitting leader must step down because it has not renewed
/// within `renew_deadline_seconds`. Mirrors the run loop's renew-loop
/// timeout: stepping down only once the deadline is strictly exceeded.
pub fn renew_deadline_exceeded(now: i64, last_renew: i64, renew_deadline_seconds: u64) -> bool {
    now - last_renew > renew_deadline_seconds as i64
}

/// Jitter factor client-go applies to the retry period when validating
/// the renew deadline (`leaderelection.go`).
const JITTER_FACTOR: f64 = 1.2;

/// Validate the timing triad as client-go does at `NewLeaderElector`:
/// `lease_duration > renew_deadline`, `renew_deadline > JITTER * retry`,
/// and every value strictly positive.
pub fn validate_config(
    lease_duration_seconds: u64,
    renew_deadline_seconds: u64,
    retry_period_seconds: u64,
) -> Result<(), String> {
    if lease_duration_seconds == 0 || renew_deadline_seconds == 0 || retry_period_seconds == 0 {
        return Err("leaseDuration, renewDeadline and retryPeriod must all be > 0".to_string());
    }
    if lease_duration_seconds <= renew_deadline_seconds {
        return Err(format!(
            "leaseDuration ({lease_duration_seconds}) must be greater than renewDeadline ({renew_deadline_seconds})"
        ));
    }
    if (renew_deadline_seconds as f64) <= JITTER_FACTOR * retry_period_seconds as f64 {
        return Err(format!(
            "renewDeadline ({renew_deadline_seconds}) must be greater than {JITTER_FACTOR} * retryPeriod ({retry_period_seconds})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(holder: &str, acquire: i64, renew: i64, transitions: u64) -> LeaderElectionRecord {
        LeaderElectionRecord {
            holder_identity: holder.to_string(),
            lease_duration_seconds: 15,
            acquire_time: acquire,
            renew_time: renew,
            leader_transitions: transitions,
        }
    }

    #[test]
    fn acquire_when_no_record_exists() {
        let (outcome, rec) = try_acquire_or_renew(1_000, "me", 15, None);
        assert_eq!(outcome, Outcome::Created);
        assert_eq!(rec.holder_identity, "me");
        assert_eq!(rec.acquire_time, 1_000);
        assert_eq!(rec.renew_time, 1_000);
        assert_eq!(rec.leader_transitions, 0);
    }

    #[test]
    fn acquire_when_existing_lease_expired() {
        // other last renewed at 100, lease 15s -> expires at 115; now=1000.
        let old = record("other", 100, 100, 3);
        let (outcome, rec) = try_acquire_or_renew(1_000, "me", 15, Some(&old));
        assert_eq!(outcome, Outcome::Acquired);
        assert_eq!(rec.holder_identity, "me");
        assert_eq!(rec.acquire_time, 1_000);
        assert_eq!(rec.leader_transitions, 4, "transition count increments on takeover");
    }

    #[test]
    fn renew_when_we_are_holder() {
        let old = record("me", 100, 990, 7);
        let (outcome, rec) = try_acquire_or_renew(1_000, "me", 15, Some(&old));
        assert_eq!(outcome, Outcome::Renewed);
        assert_eq!(rec.acquire_time, 100, "acquire_time preserved across renew");
        assert_eq!(rec.renew_time, 1_000);
        assert_eq!(rec.leader_transitions, 7, "no transition on self-renew");
    }

    #[test]
    fn lost_when_valid_lease_held_by_other() {
        // other renewed at 995, lease 15s -> valid until 1010; now=1000.
        let old = record("other", 100, 995, 2);
        let (outcome, rec) = try_acquire_or_renew(1_000, "me", 15, Some(&old));
        assert_eq!(outcome, Outcome::Lost);
        assert_eq!(rec.holder_identity, "other", "observed record returned unchanged on loss");
    }

    #[test]
    fn renew_deadline_exceeded_detects_stale_leader() {
        assert!(renew_deadline_exceeded(1_000, 989, 10));
        assert!(!renew_deadline_exceeded(1_000, 991, 10));
        assert!(!renew_deadline_exceeded(1_000, 990, 10), "exactly at deadline is not yet exceeded");
    }

    #[test]
    fn validate_config_enforces_client_go_invariants() {
        // good: lease > renew > 1.2*retry, all > 0.
        assert!(validate_config(15, 10, 2).is_ok());
        // lease must exceed renew deadline.
        assert!(validate_config(10, 10, 2).is_err());
        // renew deadline must exceed jitter*retry (1.2 * 5 = 6 >= 6 -> reject).
        assert!(validate_config(15, 6, 5).is_err());
        // zero retry period is invalid.
        assert!(validate_config(15, 10, 0).is_err());
    }
}
