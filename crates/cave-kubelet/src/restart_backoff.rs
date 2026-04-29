//! Container restart back-off — `CrashLoopBackOff`.
//!
//! Mirrors `pkg/kubelet/util/cache/object_cache.go` + `pkg/kubelet/
//! kuberuntime/kuberuntime_manager.go` `computePodActions` semantics:
//! exponential back-off starting at 10 s, doubling on each failure,
//! capped at 5 min (300 s). Restart policy gating (Always / OnFailure /
//! Never) decides if a restart is even attempted, and the back-off
//! timer is reset to 10 s once the container has stayed alive for a
//! "stable run" threshold (10 min by default).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestartPolicy {
    Always,
    OnFailure,
    Never,
}

#[derive(Debug, Clone, Copy)]
pub struct BackoffConfig {
    pub initial: Duration,
    pub max: Duration,
    /// Run-length above which the back-off resets to `initial`.
    pub stable_run: Duration,
    pub multiplier: u32,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial: Duration::seconds(10),
            max: Duration::seconds(300),
            stable_run: Duration::seconds(600),
            multiplier: 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContainerRestartState {
    pub last_terminated_at: Option<DateTime<Utc>>,
    /// Current back-off duration to wait before the next restart.
    pub current_delay: Duration,
    pub restart_count: u32,
    /// When the last restart attempt was made (i.e. when the new container started).
    pub last_started_at: Option<DateTime<Utc>>,
    pub stable_since: Option<DateTime<Utc>>,
}

impl ContainerRestartState {
    pub fn new() -> Self {
        Self {
            last_terminated_at: None,
            current_delay: BackoffConfig::default().initial,
            restart_count: 0,
            last_started_at: None,
            stable_since: None,
        }
    }
}

impl Default for ContainerRestartState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartDecision {
    Restart,
    /// Restart is allowed but not yet — the timer hasn't elapsed.
    BackOff { remaining_seconds: i64 },
    /// Restart not allowed by policy (Never, or OnFailure with exit 0).
    DoNotRestart,
    /// Container hasn't terminated yet.
    NotApplicable,
}

/// Decide whether the container should be restarted now given:
///   - the configured RestartPolicy
///   - the exit code of the most recent termination
///   - the time since termination
///   - the current back-off state
pub fn decide_restart(
    policy: RestartPolicy,
    exit_code: Option<i32>,
    state: &ContainerRestartState,
    now: DateTime<Utc>,
    _cfg: &BackoffConfig,
) -> RestartDecision {
    let term_time = match state.last_terminated_at {
        Some(t) => t,
        None => return RestartDecision::NotApplicable,
    };
    let exit = match exit_code {
        Some(e) => e,
        // Termination recorded but no exit yet — treated as failure.
        None => 1,
    };
    match policy {
        RestartPolicy::Never => RestartDecision::DoNotRestart,
        RestartPolicy::OnFailure if exit == 0 => RestartDecision::DoNotRestart,
        _ => {
            let elapsed = now - term_time;
            if elapsed >= state.current_delay {
                RestartDecision::Restart
            } else {
                let remaining = (state.current_delay - elapsed).num_seconds().max(0);
                RestartDecision::BackOff {
                    remaining_seconds: remaining,
                }
            }
        }
    }
}

/// Apply the next-back-off update after a restart fires. Doubles delay
/// up to the cap and bumps the restart counter.
pub fn record_restart(
    state: &mut ContainerRestartState,
    now: DateTime<Utc>,
    cfg: &BackoffConfig,
) {
    let next = Duration::seconds(
        state
            .current_delay
            .num_seconds()
            .saturating_mul(cfg.multiplier as i64),
    );
    state.current_delay = if next > cfg.max { cfg.max } else { next };
    state.restart_count += 1;
    state.last_started_at = Some(now);
    state.stable_since = Some(now);
}

/// Called when the kubelet observes the container has been Running for
/// `stable_run` seconds: resets back-off to `initial`.
pub fn record_stable_run(state: &mut ContainerRestartState, now: DateTime<Utc>, cfg: &BackoffConfig) {
    if let Some(since) = state.stable_since {
        if now - since >= cfg.stable_run {
            state.current_delay = cfg.initial;
        }
    }
}

/// Mark the container as terminated (used by the runtime sync loop).
pub fn record_termination(state: &mut ContainerRestartState, now: DateTime<Utc>) {
    state.last_terminated_at = Some(now);
    state.stable_since = None;
}

/// Convenience: returns the *next* back-off delay (after applying the
/// multiplier once) given the current delay and config — pure function.
pub fn next_delay(current: Duration, cfg: &BackoffConfig) -> Duration {
    let next = Duration::seconds(
        current.num_seconds().saturating_mul(cfg.multiplier as i64),
    );
    if next > cfg.max {
        cfg.max
    } else {
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(s, 0).unwrap()
    }

    #[test]
    fn never_does_not_restart_even_on_failure() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::Never, Some(1), &s, t(20), &cfg);
        assert_eq!(d, RestartDecision::DoNotRestart);
    }

    #[test]
    fn never_does_not_restart_on_success_either() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::Never, Some(0), &s, t(20), &cfg);
        assert_eq!(d, RestartDecision::DoNotRestart);
    }

    #[test]
    fn on_failure_does_not_restart_when_exit_zero() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::OnFailure, Some(0), &s, t(20), &cfg);
        assert_eq!(d, RestartDecision::DoNotRestart);
    }

    #[test]
    fn on_failure_restarts_when_nonzero() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::OnFailure, Some(137), &s, t(20), &cfg);
        assert_eq!(d, RestartDecision::Restart);
    }

    #[test]
    fn always_restarts_after_initial_delay() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::Always, Some(0), &s, t(11), &cfg);
        assert_eq!(d, RestartDecision::Restart);
    }

    #[test]
    fn always_backs_off_before_initial_delay_elapsed() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::Always, Some(1), &s, t(3), &cfg);
        assert_eq!(d, RestartDecision::BackOff { remaining_seconds: 7 });
    }

    #[test]
    fn back_off_remaining_is_zero_at_threshold() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        // Exactly at the threshold → Restart.
        let d = decide_restart(RestartPolicy::Always, Some(1), &s, t(10), &cfg);
        assert_eq!(d, RestartDecision::Restart);
    }

    #[test]
    fn record_restart_doubles_delay() {
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        record_restart(&mut s, t(0), &cfg);
        assert_eq!(s.current_delay, Duration::seconds(20));
        record_restart(&mut s, t(20), &cfg);
        assert_eq!(s.current_delay, Duration::seconds(40));
        record_restart(&mut s, t(40), &cfg);
        assert_eq!(s.current_delay, Duration::seconds(80));
    }

    #[test]
    fn record_restart_caps_at_max() {
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        s.current_delay = Duration::seconds(200);
        record_restart(&mut s, t(0), &cfg);
        // 200*2=400 → capped at 300.
        assert_eq!(s.current_delay, Duration::seconds(300));
    }

    #[test]
    fn record_restart_increments_count() {
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        record_restart(&mut s, t(0), &cfg);
        record_restart(&mut s, t(20), &cfg);
        assert_eq!(s.restart_count, 2);
    }

    #[test]
    fn record_stable_run_resets_when_threshold_met() {
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        s.current_delay = Duration::seconds(160);
        s.stable_since = Some(t(0));
        record_stable_run(&mut s, t(601), &cfg);
        assert_eq!(s.current_delay, cfg.initial);
    }

    #[test]
    fn record_stable_run_does_not_reset_before_threshold() {
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        s.current_delay = Duration::seconds(160);
        s.stable_since = Some(t(0));
        record_stable_run(&mut s, t(599), &cfg);
        assert_eq!(s.current_delay, Duration::seconds(160));
    }

    #[test]
    fn record_stable_run_noop_when_no_since() {
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        s.current_delay = Duration::seconds(80);
        record_stable_run(&mut s, t(99999), &cfg);
        assert_eq!(s.current_delay, Duration::seconds(80));
    }

    #[test]
    fn record_termination_clears_stable() {
        let mut s = ContainerRestartState::new();
        s.stable_since = Some(t(0));
        record_termination(&mut s, t(100));
        assert!(s.stable_since.is_none());
        assert_eq!(s.last_terminated_at, Some(t(100)));
    }

    #[test]
    fn next_delay_pure_function() {
        let cfg = BackoffConfig::default();
        assert_eq!(next_delay(Duration::seconds(10), &cfg), Duration::seconds(20));
        assert_eq!(next_delay(Duration::seconds(80), &cfg), Duration::seconds(160));
        assert_eq!(next_delay(Duration::seconds(160), &cfg), Duration::seconds(300));
        assert_eq!(next_delay(Duration::seconds(300), &cfg), Duration::seconds(300));
    }

    #[test]
    fn no_termination_returns_not_applicable() {
        let s = ContainerRestartState::new();
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::Always, Some(0), &s, t(0), &cfg);
        assert_eq!(d, RestartDecision::NotApplicable);
    }

    #[test]
    fn always_with_zero_exit_still_restarts() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::Always, Some(0), &s, t(11), &cfg);
        assert_eq!(d, RestartDecision::Restart);
    }

    #[test]
    fn missing_exit_code_treated_as_failure_for_on_failure() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::OnFailure, None, &s, t(15), &cfg);
        assert_eq!(d, RestartDecision::Restart);
    }

    #[test]
    fn back_off_state_with_increased_delay_blocks_correctly() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        s.current_delay = Duration::seconds(80);
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::Always, Some(1), &s, t(50), &cfg);
        assert_eq!(d, RestartDecision::BackOff { remaining_seconds: 30 });
    }

    #[test]
    fn classic_crashloop_progression_doubles_to_300_cap() {
        // Simulate failure → restart → failure → restart loop.
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        let progression = [10i64, 20, 40, 80, 160, 300, 300, 300];
        for (i, expected) in progression.iter().enumerate() {
            assert_eq!(
                s.current_delay,
                Duration::seconds(*expected),
                "iteration {} expected {} got {}",
                i,
                expected,
                s.current_delay.num_seconds()
            );
            record_restart(&mut s, t(i as i64), &cfg);
        }
    }

    #[test]
    fn record_restart_sets_last_started_and_stable() {
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        record_restart(&mut s, t(100), &cfg);
        assert_eq!(s.last_started_at, Some(t(100)));
        assert_eq!(s.stable_since, Some(t(100)));
    }

    #[test]
    fn always_after_max_delay_remaining_is_capped() {
        let mut s = ContainerRestartState::new();
        s.current_delay = Duration::seconds(300);
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::Always, Some(1), &s, t(100), &cfg);
        assert_eq!(d, RestartDecision::BackOff { remaining_seconds: 200 });
    }

    #[test]
    fn back_off_remaining_never_negative() {
        let mut s = ContainerRestartState::new();
        s.current_delay = Duration::seconds(5);
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::Always, Some(1), &s, t(4), &cfg);
        assert_eq!(d, RestartDecision::BackOff { remaining_seconds: 1 });
    }

    #[test]
    fn record_stable_run_at_exact_threshold_resets() {
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        s.current_delay = Duration::seconds(160);
        s.stable_since = Some(t(0));
        record_stable_run(&mut s, t(600), &cfg);
        assert_eq!(s.current_delay, cfg.initial);
    }

    #[test]
    fn back_off_config_default_values_match_kubelet() {
        let cfg = BackoffConfig::default();
        assert_eq!(cfg.initial, Duration::seconds(10));
        assert_eq!(cfg.max, Duration::seconds(300));
        assert_eq!(cfg.stable_run, Duration::seconds(600));
        assert_eq!(cfg.multiplier, 2);
    }

    #[test]
    fn restart_count_starts_at_zero() {
        let s = ContainerRestartState::new();
        assert_eq!(s.restart_count, 0);
    }

    #[test]
    fn restart_count_grows_monotonically() {
        let cfg = BackoffConfig::default();
        let mut s = ContainerRestartState::new();
        for i in 0..5 {
            record_restart(&mut s, t(i as i64), &cfg);
            assert_eq!(s.restart_count, (i + 1) as u32);
        }
    }

    #[test]
    fn on_failure_with_negative_exit_code_treated_as_failure() {
        let mut s = ContainerRestartState::new();
        s.last_terminated_at = Some(t(0));
        let cfg = BackoffConfig::default();
        let d = decide_restart(RestartPolicy::OnFailure, Some(-1), &s, t(15), &cfg);
        assert_eq!(d, RestartDecision::Restart);
    }
}
