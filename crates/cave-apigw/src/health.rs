// SPDX-License-Identifier: AGPL-3.0-or-later
//! Active + passive health checks.

use crate::lb::LbState;
use crate::models::{ActiveHealthCheck, PassiveHealthCheck, Upstream};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult { pub target_id: Uuid, pub healthy: bool, pub status: u16, pub elapsed: Duration }

pub struct ActiveProbe;
impl ActiveProbe {
    pub fn classify(cfg: &ActiveHealthCheck, status: u16) -> bool { cfg.http_statuses.contains(&status) }
    pub fn apply(lb: &LbState, cfg: &ActiveHealthCheck, results: &[ProbeResult]) {
        let mut hr: std::collections::HashMap<Uuid, u32> = Default::default();
        let mut ur: std::collections::HashMap<Uuid, u32> = Default::default();
        for r in results {
            if Self::classify(cfg, r.status) { *hr.entry(r.target_id).or_insert(0) += 1; ur.insert(r.target_id, 0); }
            else { *ur.entry(r.target_id).or_insert(0) += 1; hr.insert(r.target_id, 0); }
        }
        for (id, c) in &hr { if *c >= cfg.healthy_threshold { lb.mark_healthy(*id, true); } }
        for (id, c) in &ur { if *c >= cfg.unhealthy_threshold { lb.mark_healthy(*id, false); } }
    }
}

pub struct PassiveTracker;
impl PassiveTracker {
    pub fn track(lb: &LbState, cfg: &PassiveHealthCheck, target_id: Uuid,
                 status_or_zero: u16, timed_out: bool, elapsed: Duration) -> u32 {
        let success = if timed_out { false } else if status_or_zero == 0 { false }
            else { cfg.http_statuses.contains(&status_or_zero) };
        lb.record_outcome(target_id, success, elapsed);
        let st = lb.target_state(target_id);
        if !success && st.consecutive_failures >= cfg.unhealthy_threshold {
            lb.mark_healthy(target_id, false);
        }
        st.consecutive_failures
    }
}

pub struct HealthOrchestrator;
impl HealthOrchestrator {
    pub fn tick(lb: &LbState, upstream: &Upstream, synthetic_results: &[ProbeResult]) {
        if upstream.healthchecks_active.enabled {
            ActiveProbe::apply(lb, &upstream.healthchecks_active, synthetic_results);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Target;

    #[test] fn active_mark_healthy() {
        let lb = LbState::new(); let t = Target::new("a", 80, 1); lb.mark_healthy(t.id, false);
        let cfg = ActiveHealthCheck { healthy_threshold: 2, ..Default::default() };
        let r = vec![
            ProbeResult { target_id: t.id, healthy: true, status: 200, elapsed: Duration::ZERO },
            ProbeResult { target_id: t.id, healthy: true, status: 200, elapsed: Duration::ZERO },
        ];
        ActiveProbe::apply(&lb, &cfg, &r);
        assert!(lb.target_state(t.id).healthy);
    }
    #[test] fn active_mark_unhealthy() {
        let lb = LbState::new(); let t = Target::new("a", 80, 1);
        let cfg = ActiveHealthCheck { unhealthy_threshold: 2, ..Default::default() };
        let r = vec![
            ProbeResult { target_id: t.id, healthy: false, status: 500, elapsed: Duration::ZERO },
            ProbeResult { target_id: t.id, healthy: false, status: 500, elapsed: Duration::ZERO },
        ];
        ActiveProbe::apply(&lb, &cfg, &r);
        assert!(!lb.target_state(t.id).healthy);
    }
    #[test] fn classify_explicit_list() {
        let cfg = ActiveHealthCheck { http_statuses: vec![200, 204], ..Default::default() };
        assert!(ActiveProbe::classify(&cfg, 200));
        assert!(ActiveProbe::classify(&cfg, 204));
        assert!(!ActiveProbe::classify(&cfg, 500));
    }
    #[test] fn passive_tracks_failures() {
        let lb = LbState::new(); let t = Target::new("a", 80, 1);
        let cfg = PassiveHealthCheck { unhealthy_threshold: 2, ..Default::default() };
        let n1 = PassiveTracker::track(&lb, &cfg, t.id, 500, false, Duration::from_millis(10));
        let n2 = PassiveTracker::track(&lb, &cfg, t.id, 500, false, Duration::from_millis(10));
        assert_eq!(n1, 1); assert_eq!(n2, 2);
        assert!(!lb.target_state(t.id).healthy);
    }
    #[test] fn passive_timeout_fails() {
        let lb = LbState::new(); let t = Target::new("a", 80, 1);
        let cfg = PassiveHealthCheck::default();
        assert_eq!(PassiveTracker::track(&lb, &cfg, t.id, 0, true, Duration::from_secs(2)), 1);
    }
    #[test] fn passive_success_resets() {
        let lb = LbState::new(); let t = Target::new("a", 80, 1);
        let cfg = PassiveHealthCheck::default();
        PassiveTracker::track(&lb, &cfg, t.id, 500, false, Duration::ZERO);
        PassiveTracker::track(&lb, &cfg, t.id, 200, false, Duration::from_millis(5));
        assert_eq!(lb.target_state(t.id).consecutive_failures, 0);
    }
    #[test] fn orchestrator_noop_when_disabled() {
        let lb = LbState::new();
        let mut u = Upstream::new("u"); u.healthchecks_active.enabled = false;
        u.targets = vec![Target::new("a", 80, 1)];
        HealthOrchestrator::tick(&lb, &u, &[]);
        assert!(lb.target_state(u.targets[0].id).healthy);
    }
}
