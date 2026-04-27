//! Service LoadBalancer lifecycle — full ensure/update/delete state
//! machine.
//!
//! Mirrors the deeper bits of
//! `staging/src/k8s.io/cloud-provider/controllers/service/controller.go`
//! that wrap the three provider entry points
//! (`EnsureLoadBalancer` / `UpdateLoadBalancer` / `EnsureLoadBalancerDeleted`):
//!
//! * **Target sync** — per-target state machine (Pending → Initializing
//!   → Healthy → Draining → Removed) backing the connection-draining
//!   timer that upstream's controller honours.
//! * **Health gate** — only flip a target to Healthy once `N` consecutive
//!   healthchecks have passed, mirroring the LB-side bring-up loop.
//! * **Slow start** — gradual traffic ramp for newly-Healthy targets.
//! * **Backend pool diff** — per-LB diff against the desired set of
//!   nodes, returning add / remove batches.
//! * **Listener / rule diff** — what the controller would PATCH on the
//!   LB to make the listener/rule list match the Service spec.

use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── Target sync state machine ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TargetSyncState {
    /// Target was just submitted; provider has not yet acknowledged it.
    Pending,
    /// Provider acknowledged; LB is starting health checks but not yet
    /// receiving traffic at full weight.
    Initializing,
    /// Healthy; receiving traffic at full weight.
    Healthy,
    /// Marked for removal; existing connections are being drained.
    Draining,
    /// Connections drained; LB has removed the target.
    Removed,
}

impl TargetSyncState {
    pub const fn key(self) -> &'static str {
        match self {
            TargetSyncState::Pending => "Pending",
            TargetSyncState::Initializing => "Initializing",
            TargetSyncState::Healthy => "Healthy",
            TargetSyncState::Draining => "Draining",
            TargetSyncState::Removed => "Removed",
        }
    }

    /// True iff the state is a terminal "no traffic flowing" state.
    pub const fn is_terminal(self) -> bool {
        matches!(self, TargetSyncState::Removed)
    }

    /// True iff a target in this state is currently receiving traffic.
    pub const fn carries_traffic(self) -> bool {
        matches!(self, TargetSyncState::Healthy | TargetSyncState::Draining)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSyncEntry {
    pub identifier: String,
    pub state: TargetSyncState,
    /// Number of consecutive successful health checks. Resets on flip.
    pub consecutive_healthy_probes: u32,
    /// Number of consecutive failed health checks. Resets on success.
    pub consecutive_failed_probes: u32,
    /// Seconds remaining in the connection-drain timer. 0 means "drained".
    pub draining_remaining_seconds: u32,
    /// Current traffic weight in `[0, 100]` — slow-start ramps this up.
    pub traffic_weight_percent: u8,
}

impl TargetSyncEntry {
    pub fn pending(identifier: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            state: TargetSyncState::Pending,
            consecutive_healthy_probes: 0,
            consecutive_failed_probes: 0,
            draining_remaining_seconds: 0,
            traffic_weight_percent: 0,
        }
    }
}

/// Configuration for the lifecycle state machine. Mirrors the upstream
/// flag set on the cloud-provider service controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleParams {
    /// Number of consecutive successful probes before flipping to Healthy.
    pub healthy_threshold: u32,
    /// Number of consecutive failed probes before flipping to Draining.
    pub unhealthy_threshold: u32,
    /// Seconds to keep an unhealthy target receiving traffic at zero
    /// weight before removing it. Mirrors `--connection-drain-timeout`.
    pub drain_timeout_seconds: u32,
    /// Seconds for the slow-start ramp to reach 100% weight.
    pub slow_start_seconds: u32,
}

impl LifecycleParams {
    pub const fn defaults() -> Self {
        Self {
            healthy_threshold: 3,
            unhealthy_threshold: 3,
            drain_timeout_seconds: 30,
            slow_start_seconds: 30,
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if !(1..=10).contains(&self.healthy_threshold) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("healthy_threshold {} outside [1, 10]", self.healthy_threshold),
            });
        }
        if !(1..=10).contains(&self.unhealthy_threshold) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "unhealthy_threshold {} outside [1, 10]",
                    self.unhealthy_threshold
                ),
            });
        }
        if !(0..=600).contains(&self.drain_timeout_seconds) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "drain_timeout_seconds {} outside [0, 600]",
                    self.drain_timeout_seconds
                ),
            });
        }
        if !(0..=300).contains(&self.slow_start_seconds) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "slow_start_seconds {} outside [0, 300]",
                    self.slow_start_seconds
                ),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProbeOutcome {
    Success,
    Failure,
}

/// Step the target's state machine on a probe outcome. Returns the new
/// state. Mirrors the per-target update upstream's controller runs every
/// `--node-monitor-period`.
pub fn step_on_probe(
    entry: &mut TargetSyncEntry,
    outcome: ProbeOutcome,
    params: &LifecycleParams,
) -> TargetSyncState {
    match outcome {
        ProbeOutcome::Success => {
            entry.consecutive_failed_probes = 0;
            entry.consecutive_healthy_probes =
                entry.consecutive_healthy_probes.saturating_add(1);
            if matches!(entry.state, TargetSyncState::Pending | TargetSyncState::Initializing)
                && entry.consecutive_healthy_probes >= params.healthy_threshold
            {
                entry.state = TargetSyncState::Healthy;
                entry.traffic_weight_percent = if params.slow_start_seconds == 0 { 100 } else { 0 };
            }
            if matches!(entry.state, TargetSyncState::Pending) {
                entry.state = TargetSyncState::Initializing;
            }
        }
        ProbeOutcome::Failure => {
            entry.consecutive_healthy_probes = 0;
            entry.consecutive_failed_probes =
                entry.consecutive_failed_probes.saturating_add(1);
            if matches!(entry.state, TargetSyncState::Healthy)
                && entry.consecutive_failed_probes >= params.unhealthy_threshold
            {
                entry.state = TargetSyncState::Draining;
                entry.draining_remaining_seconds = params.drain_timeout_seconds;
                entry.traffic_weight_percent = 0;
            }
        }
    }
    entry.state
}

/// Advance the slow-start ramp by `elapsed_seconds`. Mirrors the upstream
/// gradual-weight programming the LB does for newly-healthy targets.
pub fn advance_slow_start(
    entry: &mut TargetSyncEntry,
    elapsed_seconds: u32,
    params: &LifecycleParams,
) -> u8 {
    if entry.state != TargetSyncState::Healthy {
        return entry.traffic_weight_percent;
    }
    if params.slow_start_seconds == 0 {
        entry.traffic_weight_percent = 100;
        return 100;
    }
    let step =
        ((elapsed_seconds as u64 * 100) / params.slow_start_seconds as u64).min(100) as u32;
    entry.traffic_weight_percent =
        (entry.traffic_weight_percent as u32 + step).min(100) as u8;
    entry.traffic_weight_percent
}

/// Tick the connection-drain timer. Returns the new state.
pub fn tick_drain(entry: &mut TargetSyncEntry, elapsed_seconds: u32) -> TargetSyncState {
    if entry.state != TargetSyncState::Draining {
        return entry.state;
    }
    entry.draining_remaining_seconds =
        entry.draining_remaining_seconds.saturating_sub(elapsed_seconds);
    if entry.draining_remaining_seconds == 0 {
        entry.state = TargetSyncState::Removed;
    }
    entry.state
}

/// Mark the entry for removal — sets its state to Draining and starts
/// the drain timer. Idempotent for already-Draining entries.
pub fn begin_drain(entry: &mut TargetSyncEntry, params: &LifecycleParams) {
    if entry.state == TargetSyncState::Draining || entry.state == TargetSyncState::Removed {
        return;
    }
    entry.state = TargetSyncState::Draining;
    entry.traffic_weight_percent = 0;
    entry.draining_remaining_seconds = params.drain_timeout_seconds;
}

// ─── Backend pool diff ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendPoolDiff {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

impl BackendPoolDiff {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
    pub fn write_count(&self) -> u32 {
        (self.add.len() + self.remove.len()) as u32
    }
}

/// Compute the diff between the LB's current backend pool and the
/// desired node set. Mirrors the loop in
/// `controller.servicePortMembers`.
pub fn diff_backend_pool(current: &[String], desired: &[String]) -> BackendPoolDiff {
    let add: Vec<String> = desired
        .iter()
        .filter(|d| !current.contains(d))
        .cloned()
        .collect();
    let remove: Vec<String> = current
        .iter()
        .filter(|c| !desired.contains(c))
        .cloned()
        .collect();
    BackendPoolDiff { add, remove }
}

// ─── Listener / rule diff ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerSpec {
    pub name: String,
    pub frontend_port: u16,
    pub backend_port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerDiff {
    pub add: Vec<ListenerSpec>,
    pub remove: Vec<String>,
    pub update: Vec<ListenerSpec>,
}

impl ListenerDiff {
    pub fn write_count(&self) -> u32 {
        (self.add.len() + self.remove.len() + self.update.len()) as u32
    }
}

pub fn diff_listeners(
    current: &[ListenerSpec],
    desired: &[ListenerSpec],
) -> ListenerDiff {
    let mut add = Vec::new();
    let mut update = Vec::new();
    for d in desired {
        match current.iter().find(|c| c.name == d.name) {
            None => add.push(d.clone()),
            Some(c) if c != d => update.push(d.clone()),
            Some(_) => {}
        }
    }
    let remove: Vec<String> = current
        .iter()
        .filter(|c| !desired.iter().any(|d| d.name == c.name))
        .map(|c| c.name.clone())
        .collect();
    ListenerDiff { add, remove, update }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) {
        let (cite, _t) = test_ctx!(path, sym, tenant);
        assert_eq!(cite.repo, "kubernetes/kubernetes");
    }

    fn defaults() -> LifecycleParams {
        LifecycleParams::defaults()
    }

    // ─── State machine ───────────────────────────────────────────────────────

    #[test]
    fn state_keys_match_lower_case_camel() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "TargetState");
        assert_eq!(TargetSyncState::Pending.key(), "Pending");
        assert_eq!(TargetSyncState::Initializing.key(), "Initializing");
        assert_eq!(TargetSyncState::Healthy.key(), "Healthy");
        assert_eq!(TargetSyncState::Draining.key(), "Draining");
        assert_eq!(TargetSyncState::Removed.key(), "Removed");
    }

    #[test]
    fn carries_traffic_returns_true_only_for_healthy_and_draining() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "TargetState");
        assert!(TargetSyncState::Healthy.carries_traffic());
        assert!(TargetSyncState::Draining.carries_traffic());
        assert!(!TargetSyncState::Pending.carries_traffic());
        assert!(!TargetSyncState::Initializing.carries_traffic());
        assert!(!TargetSyncState::Removed.carries_traffic());
    }

    #[test]
    fn is_terminal_only_for_removed() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "TargetState");
        assert!(TargetSyncState::Removed.is_terminal());
        for s in [
            TargetSyncState::Pending,
            TargetSyncState::Initializing,
            TargetSyncState::Healthy,
            TargetSyncState::Draining,
        ] {
            assert!(!s.is_terminal());
        }
    }

    // ─── LifecycleParams ─────────────────────────────────────────────────────

    #[test]
    fn lifecycle_defaults_validate() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "DefaultLifecycle");
        assert!(defaults().validate().is_ok());
    }

    #[test]
    fn lifecycle_healthy_threshold_outside_1_10_is_rejected() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "DefaultLifecycle");
        let mut p = defaults();
        p.healthy_threshold = 0;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        p.healthy_threshold = 99;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn lifecycle_unhealthy_threshold_outside_1_10_is_rejected() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "DefaultLifecycle");
        let mut p = defaults();
        p.unhealthy_threshold = 0;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn lifecycle_drain_timeout_outside_0_600_is_rejected() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "DefaultLifecycle");
        let mut p = defaults();
        p.drain_timeout_seconds = 700;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn lifecycle_slow_start_outside_0_300_is_rejected() {
        ctx("acme", "cmd/cloud-controller-manager/app/options/options.go", "DefaultLifecycle");
        let mut p = defaults();
        p.slow_start_seconds = 999;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── Probe transitions ───────────────────────────────────────────────────

    #[test]
    fn pending_target_first_success_moves_to_initializing() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncTargets");
        let mut e = TargetSyncEntry::pending("t1");
        let s = step_on_probe(&mut e, ProbeOutcome::Success, &defaults());
        assert_eq!(s, TargetSyncState::Initializing);
    }

    #[test]
    fn initializing_target_promotes_to_healthy_after_threshold() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncTargets");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        assert_eq!(e.state, TargetSyncState::Healthy);
    }

    #[test]
    fn healthy_target_drains_after_unhealthy_threshold_failures() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncTargets");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        assert_eq!(e.state, TargetSyncState::Healthy);
        for _ in 0..p.unhealthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Failure, &p);
        }
        assert_eq!(e.state, TargetSyncState::Draining);
        assert_eq!(e.traffic_weight_percent, 0);
        assert_eq!(e.draining_remaining_seconds, p.drain_timeout_seconds);
    }

    #[test]
    fn probe_success_resets_failure_counter() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncTargets");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        step_on_probe(&mut e, ProbeOutcome::Failure, &p);
        assert_eq!(e.consecutive_failed_probes, 1);
        step_on_probe(&mut e, ProbeOutcome::Success, &p);
        assert_eq!(e.consecutive_failed_probes, 0);
        assert_eq!(e.state, TargetSyncState::Healthy);
    }

    #[test]
    fn intermittent_failures_below_threshold_keep_healthy_state() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncTargets");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        for _ in 0..p.unhealthy_threshold - 1 {
            step_on_probe(&mut e, ProbeOutcome::Failure, &p);
        }
        assert_eq!(e.state, TargetSyncState::Healthy);
    }

    // ─── Slow start ──────────────────────────────────────────────────────────

    #[test]
    fn slow_start_ramps_up_proportional_to_elapsed() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "slowStart");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        // 30 s ramp; 6 s elapsed → 20% weight.
        let w = advance_slow_start(&mut e, 6, &p);
        assert_eq!(w, 20);
    }

    #[test]
    fn slow_start_caps_at_one_hundred() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "slowStart");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        let w = advance_slow_start(&mut e, 9999, &p);
        assert_eq!(w, 100);
    }

    #[test]
    fn slow_start_is_a_noop_for_non_healthy_targets() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "slowStart");
        let mut e = TargetSyncEntry::pending("t1");
        let w = advance_slow_start(&mut e, 30, &defaults());
        assert_eq!(w, 0);
    }

    #[test]
    fn slow_start_zero_seconds_jumps_to_full_weight_on_promotion() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "slowStart");
        let mut p = defaults();
        p.slow_start_seconds = 0;
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        assert_eq!(e.traffic_weight_percent, 100);
    }

    // ─── Drain timer ─────────────────────────────────────────────────────────

    #[test]
    fn tick_drain_decrements_remaining_then_removes() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "ConnectionDraining");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        for _ in 0..p.unhealthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Failure, &p);
        }
        assert_eq!(e.state, TargetSyncState::Draining);
        let s = tick_drain(&mut e, p.drain_timeout_seconds / 2);
        assert_eq!(s, TargetSyncState::Draining);
        let s = tick_drain(&mut e, p.drain_timeout_seconds);
        assert_eq!(s, TargetSyncState::Removed);
    }

    #[test]
    fn tick_drain_is_a_noop_for_non_draining_states() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "ConnectionDraining");
        let mut e = TargetSyncEntry::pending("t1");
        let s = tick_drain(&mut e, 60);
        assert_eq!(s, TargetSyncState::Pending);
    }

    #[test]
    fn begin_drain_sets_state_and_starts_timer() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "removeTarget");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        begin_drain(&mut e, &p);
        assert_eq!(e.state, TargetSyncState::Draining);
        assert_eq!(e.draining_remaining_seconds, p.drain_timeout_seconds);
        assert_eq!(e.traffic_weight_percent, 0);
    }

    #[test]
    fn begin_drain_is_idempotent_for_already_draining_target() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "removeTarget");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        for _ in 0..p.healthy_threshold {
            step_on_probe(&mut e, ProbeOutcome::Success, &p);
        }
        begin_drain(&mut e, &p);
        let before = e.draining_remaining_seconds;
        // Tick a bit, then begin_drain again — must not reset the timer.
        tick_drain(&mut e, 10);
        let mid = e.draining_remaining_seconds;
        begin_drain(&mut e, &p);
        assert_eq!(e.draining_remaining_seconds, mid);
        assert!(mid < before);
    }

    #[test]
    fn begin_drain_does_not_resurrect_removed_targets() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "removeTarget");
        let p = defaults();
        let mut e = TargetSyncEntry::pending("t1");
        e.state = TargetSyncState::Removed;
        begin_drain(&mut e, &p);
        assert_eq!(e.state, TargetSyncState::Removed);
    }

    // ─── Backend pool diff ───────────────────────────────────────────────────

    #[test]
    fn backend_pool_diff_returns_empty_for_identical_sets() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "servicePortMembers");
        let cur = vec!["n1".to_string(), "n2".to_string()];
        let want = cur.clone();
        let d = diff_backend_pool(&cur, &want);
        assert!(d.is_empty());
        assert_eq!(d.write_count(), 0);
    }

    #[test]
    fn backend_pool_diff_emits_add_for_new_members() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "servicePortMembers");
        let cur = vec!["n1".to_string()];
        let want = vec!["n1".to_string(), "n2".to_string()];
        let d = diff_backend_pool(&cur, &want);
        assert_eq!(d.add, vec!["n2".to_string()]);
        assert!(d.remove.is_empty());
    }

    #[test]
    fn backend_pool_diff_emits_remove_for_stale_members() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "servicePortMembers");
        let cur = vec!["n1".to_string(), "n2".to_string()];
        let want = vec!["n1".to_string()];
        let d = diff_backend_pool(&cur, &want);
        assert_eq!(d.remove, vec!["n2".to_string()]);
        assert!(d.add.is_empty());
    }

    #[test]
    fn backend_pool_diff_handles_full_swap() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "servicePortMembers");
        let cur = vec!["n1".to_string(), "n2".to_string()];
        let want = vec!["n3".to_string(), "n4".to_string()];
        let d = diff_backend_pool(&cur, &want);
        assert_eq!(d.write_count(), 4);
    }

    // ─── Listener diff ───────────────────────────────────────────────────────

    fn lst(name: &str, fp: u16, bp: u16, proto: &str) -> ListenerSpec {
        ListenerSpec {
            name: name.into(),
            frontend_port: fp,
            backend_port: bp,
            protocol: proto.into(),
        }
    }

    #[test]
    fn listener_diff_returns_empty_for_identical_lists() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncListeners");
        let l = vec![lst("http", 80, 8080, "TCP")];
        let d = diff_listeners(&l, &l);
        assert!(d.add.is_empty() && d.remove.is_empty() && d.update.is_empty());
    }

    #[test]
    fn listener_diff_emits_add_for_new_listener() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncListeners");
        let cur = vec![lst("http", 80, 8080, "TCP")];
        let want = vec![lst("http", 80, 8080, "TCP"), lst("https", 443, 8443, "TCP")];
        let d = diff_listeners(&cur, &want);
        assert_eq!(d.add.len(), 1);
        assert_eq!(d.add[0].name, "https");
    }

    #[test]
    fn listener_diff_emits_remove_for_stale_listener() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncListeners");
        let cur = vec![lst("http", 80, 8080, "TCP"), lst("legacy", 81, 8081, "TCP")];
        let want = vec![lst("http", 80, 8080, "TCP")];
        let d = diff_listeners(&cur, &want);
        assert_eq!(d.remove, vec!["legacy".to_string()]);
    }

    #[test]
    fn listener_diff_emits_update_for_port_change() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncListeners");
        let cur = vec![lst("http", 80, 8080, "TCP")];
        let want = vec![lst("http", 80, 8081, "TCP")];
        let d = diff_listeners(&cur, &want);
        assert_eq!(d.update.len(), 1);
        assert_eq!(d.update[0].backend_port, 8081);
    }

    #[test]
    fn listener_diff_write_count_sums_three_categories() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncListeners");
        let cur = vec![lst("a", 80, 8080, "TCP"), lst("b", 81, 8081, "TCP")];
        let want = vec![lst("a", 80, 8090, "TCP"), lst("c", 82, 8082, "TCP")];
        let d = diff_listeners(&cur, &want);
        assert_eq!(d.write_count(), 3); // 1 update + 1 remove + 1 add
    }

    // ─── pending() constructor ───────────────────────────────────────────────

    #[test]
    fn target_sync_entry_pending_constructor_zeroes_counters() {
        ctx("acme", "staging/src/k8s.io/cloud-provider/controllers/service/controller.go", "syncTargets");
        let e = TargetSyncEntry::pending("t1");
        assert_eq!(e.state, TargetSyncState::Pending);
        assert_eq!(e.consecutive_healthy_probes, 0);
        assert_eq!(e.consecutive_failed_probes, 0);
        assert_eq!(e.traffic_weight_percent, 0);
        assert_eq!(e.identifier, "t1");
    }
}
