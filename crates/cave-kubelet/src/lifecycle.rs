// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pod lifecycle hooks — `preStop` + `postStart` orchestration.
//!
//! Cite: pkg/kubelet/lifecycle/ (v1.36.0).
//!
//! Two kinds of handler shapes per upstream's `core/v1.Handler`:
//!
//!   * `exec` — run a command inside the container.
//!   * `httpGet` — send a request to a port + path on the pod.
//!
//! (TCPSocket is deprecated in v1.31; sleep is a newer addition but
//! upstream still surfaces it in v1.32+ which is past our pin.)
//!
//! `preStop` runs synchronously inside the pod's
//! `terminationGracePeriodSeconds` window; if the handler exceeds
//! the per-hook timeout the kubelet records `PreStopHookFailed` but
//! continues to terminate. `postStart` runs after the container has
//! started; failure marks the container as `Failed` and triggers a
//! restart per the pod's restart policy.
//!
//! This module is a pure decision layer. The kubelet sync loop
//! converts each [`HookOutcome`] into actual CRI calls or HTTP
//! requests; testing that integration belongs in `agent.rs`.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[allow(dead_code)]
pub const UPSTREAM_PATH: &str = "pkg/kubelet/lifecycle/handlers.go";

/// Which lifecycle stage a hook fires at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookStage {
    PostStart,
    PreStop,
}

/// One handler. Mirrors `core/v1.LifecycleHandler` reduced to the
/// fields cave actually exercises today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookHandler {
    Exec { command: Vec<String> },
    HttpGet { port: u16, path: String, scheme: HttpScheme },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpScheme {
    Http,
    Https,
}

/// One pending hook execution. The kubelet sync loop calls
/// [`evaluate`] each tick with the hook's clock state; the action
/// drives whether to fire / await / record-failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookExecution {
    pub stage: HookStage,
    pub handler: HookHandler,
    /// First time we observed the hook becoming due.
    pub started_at: DateTime<Utc>,
    /// Per-hook timeout, applied independently of the pod-wide
    /// `terminationGracePeriodSeconds`.
    pub timeout: Duration,
    /// `true` once the handler returned success.
    pub completed: bool,
    /// `Some(reason)` once a permanent failure has been recorded.
    pub failure_reason: Option<String>,
}

impl HookExecution {
    pub fn new(stage: HookStage, handler: HookHandler, now: DateTime<Utc>, timeout: Duration) -> Self {
        Self {
            stage,
            handler,
            started_at: now,
            timeout,
            completed: false,
            failure_reason: None,
        }
    }

    /// Whether the per-hook timer has fired by `now`.
    pub fn timed_out(&self, now: DateTime<Utc>) -> bool {
        let dur = ChronoDuration::from_std(self.timeout).unwrap_or(ChronoDuration::zero());
        now - self.started_at >= dur
    }
}

/// What the kubelet sync loop should do this tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookOutcome {
    /// Hook hasn't been launched yet — fire it now.
    Fire(HookHandler),
    /// Hook is in flight and within its timeout — keep waiting.
    Pending,
    /// Hook completed successfully — record + move on.
    Completed,
    /// Hook exceeded its per-hook timeout — record failure +
    /// continue (for PreStop, terminate the container; for
    /// PostStart, mark the container Failed so restart-policy
    /// kicks in).
    TimedOut { reason: String },
    /// Hook returned a hard failure (exec non-zero, HTTP non-2xx) —
    /// same handling as TimedOut.
    Failed { reason: String },
}

/// Sample result the kubelet sync loop feeds back after calling the
/// handler. The reconciler uses it to advance the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookSample {
    /// Handler hasn't been called yet on this tick.
    NotFiredYet,
    /// Most recent call returned success.
    Success,
    /// Most recent call returned a permanent failure.
    Failure,
}

/// Pure reconciler. Given a hook execution + the latest sample
/// + current time, return the next action.
pub fn evaluate(
    exec: &mut HookExecution,
    sample: HookSample,
    now: DateTime<Utc>,
) -> HookOutcome {
    if exec.completed {
        return HookOutcome::Completed;
    }
    if let Some(r) = &exec.failure_reason {
        return HookOutcome::Failed { reason: r.clone() };
    }
    match sample {
        HookSample::Success => {
            exec.completed = true;
            HookOutcome::Completed
        }
        HookSample::Failure => {
            let reason = format!("{:?} handler returned failure", exec.stage);
            exec.failure_reason = Some(reason.clone());
            HookOutcome::Failed { reason }
        }
        HookSample::NotFiredYet => {
            if exec.timed_out(now) {
                let reason = format!(
                    "{:?} handler exceeded timeout={}s",
                    exec.stage,
                    exec.timeout.as_secs()
                );
                exec.failure_reason = Some(reason.clone());
                HookOutcome::TimedOut { reason }
            } else {
                // First evaluation → fire; subsequent NotFiredYet
                // before sample arrives → Pending.
                if exec.started_at == now {
                    HookOutcome::Fire(exec.handler.clone())
                } else {
                    HookOutcome::Pending
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn exec_hook() -> HookExecution {
        HookExecution::new(
            HookStage::PreStop,
            HookHandler::Exec {
                command: vec!["/bin/sh".into(), "-c".into(), "drain.sh".into()],
            },
            now(),
            Duration::from_secs(30),
        )
    }

    #[test]
    fn fresh_execution_fires_the_handler_first_tick() {
        let mut e = exec_hook();
        let out = evaluate(&mut e, HookSample::NotFiredYet, now());
        assert!(matches!(out, HookOutcome::Fire(_)));
    }

    #[test]
    fn pending_returned_when_handler_in_flight_within_timeout() {
        let mut e = exec_hook();
        let mid = now() + ChronoDuration::seconds(5);
        let out = evaluate(&mut e, HookSample::NotFiredYet, mid);
        assert_eq!(out, HookOutcome::Pending);
    }

    #[test]
    fn timed_out_returned_once_handler_exceeds_timeout() {
        let mut e = exec_hook();
        let later = now() + ChronoDuration::seconds(31);
        let out = evaluate(&mut e, HookSample::NotFiredYet, later);
        assert!(matches!(out, HookOutcome::TimedOut { .. }));
        // Subsequent evaluation keeps reporting Failed (sticky).
        let out2 = evaluate(&mut e, HookSample::NotFiredYet, later);
        assert!(matches!(out2, HookOutcome::Failed { .. }));
    }

    #[test]
    fn sample_success_marks_completed_and_is_sticky() {
        let mut e = exec_hook();
        let out = evaluate(&mut e, HookSample::Success, now());
        assert_eq!(out, HookOutcome::Completed);
        // Sticky: even with NotFiredYet later we stay Completed.
        let out2 = evaluate(&mut e, HookSample::NotFiredYet, now() + ChronoDuration::seconds(60));
        assert_eq!(out2, HookOutcome::Completed);
    }

    #[test]
    fn sample_failure_records_reason_and_is_sticky() {
        let mut e = exec_hook();
        let out = evaluate(&mut e, HookSample::Failure, now());
        assert!(matches!(out, HookOutcome::Failed { .. }));
        let out2 = evaluate(&mut e, HookSample::Success, now());
        // Failure is sticky — once recorded the reconciler doesn't
        // un-fail even if a later success sneaks in. Upstream's
        // PostStart hook treats first non-zero exit as terminal.
        assert!(matches!(out2, HookOutcome::Failed { .. }));
    }

    #[test]
    fn http_handler_round_trips_through_serde() {
        let h = HookHandler::HttpGet {
            port: 8080,
            path: "/drain".into(),
            scheme: HttpScheme::Http,
        };
        let json = serde_json::to_string(&h).unwrap();
        let back: HookHandler = serde_json::from_str(&json).unwrap();
        assert_eq!(h, back);
    }

    #[test]
    fn timed_out_is_correct_at_exact_boundary() {
        let mut e = exec_hook();
        let exact = now() + ChronoDuration::seconds(30);
        assert!(e.timed_out(exact));
        let _ = evaluate(&mut e, HookSample::NotFiredYet, exact);
        assert!(e.failure_reason.is_some());
    }

    #[test]
    fn post_start_hook_distinct_from_pre_stop() {
        // Sanity: stage is honoured in error messages.
        let mut e = HookExecution::new(
            HookStage::PostStart,
            HookHandler::Exec { command: vec!["init".into()] },
            now(),
            Duration::from_secs(10),
        );
        let _ = evaluate(&mut e, HookSample::Failure, now());
        let msg = e.failure_reason.unwrap();
        assert!(msg.contains("PostStart"));
    }
}
