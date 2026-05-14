// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NodeLease deeper — `pkg/kubelet/nodelease/controller.go` +
//! `pkg/controller/nodelifecycle`.
//!
//! Adds:
//!
//! * Holder identity rotation (controller-manager failover, kubelet
//!   reschedule with same node name → fresh holder identity).
//! * Renewal interval = `lease_duration_sec * 0.25` (upstream kubelet).
//! * Stale-lease detection across multiple kubelet generations
//!   (renewal time < observed_at means a fresher renewal raced past).
//! * LeaseLock leader election helpers — `pkg/leaderelection/leaselock.go`.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub holder_identity: String,
    pub lease_duration_sec: u32,
    pub renew_time_sec: u64,
    /// `acquireTime` mirrors upstream — first time this holder claimed the
    /// lease. Helps detect failovers.
    pub acquire_time_sec: u64,
    /// Each successful renewal bumps this. Stale observers can detect a
    /// concurrent newer renewal by comparing.
    pub lease_transitions: u32,
}

/// Default kubelet renewal cadence — `lease_duration_sec * 0.25`.
pub const RENEWAL_FRACTION: f64 = 0.25;

pub fn renewal_interval_sec(lease_duration_sec: u32) -> u32 {
    ((lease_duration_sec as f64) * RENEWAL_FRACTION).max(1.0) as u32
}

/// True if the local observation of `lease` is stale compared to the
/// most recent renewal we have on file (e.g. seen via watch).
pub fn observation_is_stale(local: &LeaseRecord, latest: &LeaseRecord) -> bool {
    if local.holder_identity != latest.holder_identity {
        return latest.acquire_time_sec >= local.acquire_time_sec;
    }
    latest.lease_transitions > local.lease_transitions
        || latest.renew_time_sec > local.renew_time_sec
}

/// Holder identity rotation: a different identity holding the same
/// (lease name, namespace) tuple after `lease_expired` indicates failover.
pub fn detect_failover(prev: &LeaseRecord, curr: &LeaseRecord) -> bool {
    prev.holder_identity != curr.holder_identity
        && curr.acquire_time_sec >= prev.renew_time_sec + prev.lease_duration_sec as u64
}

// ── LeaseLock leader election ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderElectionConfig {
    pub lease_duration_sec: u32,
    pub renew_deadline_sec: u32,
    pub retry_period_sec: u32,
}

impl Default for LeaderElectionConfig {
    fn default() -> Self {
        // Upstream defaults from `cmd/kube-controller-manager/app/options/options.go`.
        Self {
            lease_duration_sec: 15,
            renew_deadline_sec: 10,
            retry_period_sec: 2,
        }
    }
}

/// Validate the constraint between renew_deadline / lease_duration / retry_period.
/// Mirrors `LeaderElectionConfiguration::Validate`:
///
/// * `lease_duration > renew_deadline`
/// * `renew_deadline > retry_period`
/// * `retry_period > 0`
pub fn validate_leader_config(cfg: &LeaderElectionConfig) -> Result<(), &'static str> {
    if cfg.retry_period_sec == 0 {
        return Err("retry_period must be > 0");
    }
    if cfg.renew_deadline_sec <= cfg.retry_period_sec {
        return Err("renew_deadline must be > retry_period");
    }
    if cfg.lease_duration_sec <= cfg.renew_deadline_sec {
        return Err("lease_duration must be > renew_deadline");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeaderAction {
    /// We hold the lease and should renew it now.
    Renew,
    /// We hold the lease but it isn't time to renew yet.
    Hold,
    /// Lease is held by a different identity and not yet expired.
    Follow,
    /// Lease has expired — try to acquire.
    AcquireAttempt,
}

pub fn leader_step(
    self_identity: &str,
    lease: &LeaseRecord,
    cfg: &LeaderElectionConfig,
    now_sec: u64,
) -> LeaderAction {
    let expire = lease.renew_time_sec + lease.lease_duration_sec as u64;
    let renew_at = lease.renew_time_sec + cfg.retry_period_sec as u64;
    if lease.holder_identity == self_identity {
        // We hold; check if we should push a renewal.
        if now_sec >= renew_at {
            LeaderAction::Renew
        } else {
            LeaderAction::Hold
        }
    } else if now_sec > expire {
        LeaderAction::AcquireAttempt
    } else {
        LeaderAction::Follow
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/kubelet/nodelease/controller.go",
    "Controller",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn rec(holder: &str, dur: u32, renew: u64, acquire: u64, trans: u32) -> LeaseRecord {
        LeaseRecord {
            holder_identity: holder.into(),
            lease_duration_sec: dur,
            renew_time_sec: renew,
            acquire_time_sec: acquire,
            lease_transitions: trans,
        }
    }

    #[test]
    fn renewal_interval_is_quarter_of_duration() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/kubelet/nodelease/controller.go",
            "Controller",
            "tenant-nl2-renewal-quarter"
        );
        assert_eq!(renewal_interval_sec(40), 10);
        assert_eq!(renewal_interval_sec(120), 30);
    }

    #[test]
    fn renewal_interval_floor_is_one_second() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/kubelet/nodelease/controller.go",
            "Controller",
            "tenant-nl2-renewal-floor"
        );
        assert_eq!(renewal_interval_sec(1), 1);
        assert_eq!(renewal_interval_sec(2), 1);
    }

    #[test]
    fn observation_stale_when_transitions_advanced() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/kubelet/nodelease/controller.go",
            "ensureLease",
            "tenant-nl2-stale-trans"
        );
        let local = rec("kubelet-1", 40, 100, 0, 5);
        let latest = rec("kubelet-1", 40, 100, 0, 7);
        assert!(observation_is_stale(&local, &latest));
    }

    #[test]
    fn observation_stale_when_renew_advanced() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/kubelet/nodelease/controller.go",
            "ensureLease",
            "tenant-nl2-stale-renew"
        );
        let local = rec("kubelet-1", 40, 100, 0, 5);
        let latest = rec("kubelet-1", 40, 110, 0, 5);
        assert!(observation_is_stale(&local, &latest));
    }

    #[test]
    fn observation_fresh_when_equal() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/kubelet/nodelease/controller.go",
            "ensureLease",
            "tenant-nl2-fresh"
        );
        let r = rec("kubelet-1", 40, 100, 0, 5);
        assert!(!observation_is_stale(&r, &r));
    }

    #[test]
    fn observation_stale_via_holder_change() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/kubelet/nodelease/controller.go",
            "ensureLease",
            "tenant-nl2-stale-holder"
        );
        let local = rec("kubelet-1", 40, 100, 50, 5);
        let latest = rec("kubelet-2", 40, 100, 200, 0);
        assert!(observation_is_stale(&local, &latest));
    }

    #[test]
    fn detect_failover_after_expiry() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-nl2-failover-detect"
        );
        let prev = rec("kubelet-1", 40, 100, 50, 1);
        // expire = 100 + 40 = 140; new acquire after that.
        let curr = rec("kubelet-2", 40, 200, 200, 0);
        assert!(detect_failover(&prev, &curr));
    }

    #[test]
    fn detect_failover_negative_when_same_holder() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-nl2-no-failover-same-holder"
        );
        let prev = rec("kubelet-1", 40, 100, 50, 1);
        let curr = rec("kubelet-1", 40, 200, 50, 5);
        assert!(!detect_failover(&prev, &curr));
    }

    #[test]
    fn detect_failover_negative_when_within_lease() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-nl2-no-failover-within-lease"
        );
        let prev = rec("kubelet-1", 40, 100, 50, 1);
        // expire = 140; new holder claimed at 130 (still within lease window).
        let curr = rec("kubelet-2", 40, 130, 130, 0);
        assert!(!detect_failover(&prev, &curr));
    }

    #[test]
    fn validate_leader_config_default_is_ok() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/leaderelection/resourcelock/interface.go",
            "Validate",
            "tenant-le-cfg-default"
        );
        assert!(validate_leader_config(&LeaderElectionConfig::default()).is_ok());
    }

    #[test]
    fn validate_leader_config_rejects_zero_retry() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/leaderelection/resourcelock/interface.go",
            "Validate",
            "tenant-le-cfg-zero-retry"
        );
        let cfg = LeaderElectionConfig {
            lease_duration_sec: 15,
            renew_deadline_sec: 10,
            retry_period_sec: 0,
        };
        assert!(validate_leader_config(&cfg).is_err());
    }

    #[test]
    fn validate_leader_config_rejects_renew_under_retry() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/leaderelection/resourcelock/interface.go",
            "Validate",
            "tenant-le-cfg-renew-under-retry"
        );
        let cfg = LeaderElectionConfig {
            lease_duration_sec: 15,
            renew_deadline_sec: 1,
            retry_period_sec: 2,
        };
        assert!(validate_leader_config(&cfg).is_err());
    }

    #[test]
    fn validate_leader_config_rejects_duration_under_renew() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/leaderelection/resourcelock/interface.go",
            "Validate",
            "tenant-le-cfg-duration-under-renew"
        );
        let cfg = LeaderElectionConfig {
            lease_duration_sec: 5,
            renew_deadline_sec: 10,
            retry_period_sec: 2,
        };
        assert!(validate_leader_config(&cfg).is_err());
    }

    #[test]
    fn leader_step_holder_renews_after_retry_period() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/leaderelection/leaderelection.go",
            "tryAcquireOrRenew",
            "tenant-le-step-renew"
        );
        let cfg = LeaderElectionConfig::default();
        let lease = rec("self", cfg.lease_duration_sec, 100, 50, 1);
        // retry_period=2, so at now=103 we should renew.
        assert_eq!(leader_step("self", &lease, &cfg, 103), LeaderAction::Renew);
    }

    #[test]
    fn leader_step_holder_holds_before_retry_period() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/leaderelection/leaderelection.go",
            "tryAcquireOrRenew",
            "tenant-le-step-hold"
        );
        let cfg = LeaderElectionConfig::default();
        let lease = rec("self", cfg.lease_duration_sec, 100, 50, 1);
        assert_eq!(leader_step("self", &lease, &cfg, 101), LeaderAction::Hold);
    }

    #[test]
    fn leader_step_follower_within_lease_just_follows() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/leaderelection/leaderelection.go",
            "tryAcquireOrRenew",
            "tenant-le-step-follow"
        );
        let cfg = LeaderElectionConfig::default();
        let lease = rec("other", cfg.lease_duration_sec, 100, 50, 1);
        // expire = 100 + 15 = 115. now=110 → still leader.
        assert_eq!(leader_step("self", &lease, &cfg, 110), LeaderAction::Follow);
    }

    #[test]
    fn leader_step_follower_after_expire_attempts_acquire() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/leaderelection/leaderelection.go",
            "tryAcquireOrRenew",
            "tenant-le-step-acquire"
        );
        let cfg = LeaderElectionConfig::default();
        let lease = rec("other", cfg.lease_duration_sec, 100, 50, 1);
        assert_eq!(
            leader_step("self", &lease, &cfg, 200),
            LeaderAction::AcquireAttempt
        );
    }

    #[test]
    fn leader_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/leaderelection/leaderelection.go",
            "LeaderAction",
            "tenant-le-action-serde"
        );
        for a in [
            LeaderAction::Renew,
            LeaderAction::Hold,
            LeaderAction::Follow,
            LeaderAction::AcquireAttempt,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: LeaderAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}
