// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Liveness / readiness / startup probe coordinator.
//!
//! Mirrors `pkg/kubelet/prober`.  The umbrella layer maintains the
//! probe state machine — success / failure thresholds, transition
//! timestamps, restart triggers — while the actual HTTP / TCP / exec
//! call lives in `cave-kubelet`.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeKind {
    Liveness,
    Readiness,
    Startup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeResult {
    Success,
    Failure,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeConfig {
    pub kind: ProbeKind,
    pub initial_delay: Duration,
    pub period: Duration,
    pub timeout: Duration,
    pub success_threshold: u32,
    pub failure_threshold: u32,
}

impl ProbeConfig {
    pub fn liveness_default() -> Self {
        Self {
            kind: ProbeKind::Liveness,
            initial_delay: Duration::from_secs(0),
            period: Duration::from_secs(10),
            timeout: Duration::from_secs(1),
            success_threshold: 1,
            failure_threshold: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeState {
    pub kind: ProbeKind,
    pub last: ProbeResult,
    pub consecutive_success: u32,
    pub consecutive_failure: u32,
    pub passing: bool,
}

impl ProbeState {
    pub fn new(kind: ProbeKind) -> Self {
        Self {
            kind,
            last: ProbeResult::Unknown,
            consecutive_success: 0,
            consecutive_failure: 0,
            passing: kind != ProbeKind::Startup,
        }
    }

    pub fn record(&mut self, result: ProbeResult, cfg: &ProbeConfig) -> Option<ProbeTransition> {
        self.last = result;
        match result {
            ProbeResult::Success => {
                self.consecutive_success += 1;
                self.consecutive_failure = 0;
                if !self.passing && self.consecutive_success >= cfg.success_threshold {
                    self.passing = true;
                    return Some(ProbeTransition::ToPassing);
                }
            }
            ProbeResult::Failure => {
                self.consecutive_failure += 1;
                self.consecutive_success = 0;
                if self.passing && self.consecutive_failure >= cfg.failure_threshold {
                    self.passing = false;
                    return Some(ProbeTransition::ToFailing);
                }
            }
            ProbeResult::Unknown => {}
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeTransition {
    ToPassing,
    ToFailing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveness_default_thresholds() {
        let c = ProbeConfig::liveness_default();
        assert_eq!(c.success_threshold, 1);
        assert_eq!(c.failure_threshold, 3);
    }

    #[test]
    fn liveness_three_failures_trips_failing() {
        let cfg = ProbeConfig::liveness_default();
        let mut s = ProbeState::new(ProbeKind::Liveness);
        assert_eq!(s.passing, true);
        s.record(ProbeResult::Failure, &cfg);
        s.record(ProbeResult::Failure, &cfg);
        let t = s.record(ProbeResult::Failure, &cfg);
        assert_eq!(t, Some(ProbeTransition::ToFailing));
        assert!(!s.passing);
    }

    #[test]
    fn startup_starts_failing_until_success() {
        let cfg = ProbeConfig {
            kind: ProbeKind::Startup,
            success_threshold: 1,
            failure_threshold: 30,
            ..ProbeConfig::liveness_default()
        };
        let mut s = ProbeState::new(ProbeKind::Startup);
        assert!(!s.passing);
        let t = s.record(ProbeResult::Success, &cfg);
        assert_eq!(t, Some(ProbeTransition::ToPassing));
        assert!(s.passing);
    }

    #[test]
    fn single_failure_below_threshold_no_transition() {
        let cfg = ProbeConfig::liveness_default();
        let mut s = ProbeState::new(ProbeKind::Liveness);
        let t = s.record(ProbeResult::Failure, &cfg);
        assert!(t.is_none());
        assert!(s.passing);
    }

    #[test]
    fn success_resets_failure_counter() {
        let cfg = ProbeConfig::liveness_default();
        let mut s = ProbeState::new(ProbeKind::Liveness);
        s.record(ProbeResult::Failure, &cfg);
        s.record(ProbeResult::Success, &cfg);
        assert_eq!(s.consecutive_failure, 0);
    }

    #[test]
    fn readiness_kind_distinct() {
        let s = ProbeState::new(ProbeKind::Readiness);
        assert_eq!(s.kind, ProbeKind::Readiness);
        assert!(s.passing);
    }

    #[test]
    fn unknown_result_does_not_change_counters() {
        let cfg = ProbeConfig::liveness_default();
        let mut s = ProbeState::new(ProbeKind::Liveness);
        s.record(ProbeResult::Unknown, &cfg);
        assert_eq!(s.consecutive_failure, 0);
        assert_eq!(s.consecutive_success, 0);
    }
}
