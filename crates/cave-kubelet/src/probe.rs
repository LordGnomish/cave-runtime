// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Probe handler — liveness, readiness, startup probes.
//!
//! Mirrors `pkg/kubelet/prober` semantics: HTTP-Get, TCP-Socket, Exec, gRPC.
//! Honours initialDelaySeconds, periodSeconds, timeoutSeconds, successThreshold,
//! failureThreshold, terminationGracePeriodSeconds. Tracks consecutive successes
//! and failures and produces the resulting probe outcome.
//!
//! The actual transport is abstracted behind `ProbeExecutor` so the kubelet's
//! state-machine logic is independently testable.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProbeKind {
    Liveness,
    Readiness,
    Startup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeResult {
    /// Probe succeeded.
    Success,
    /// Probe ran and reported failure.
    Failure,
    /// Probe could not run (config invalid, transport error treated as failure
    /// at threshold-counting time but distinct from Failure for telemetry).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeAction {
    HttpGet {
        scheme: HttpScheme,
        host: Option<String>,
        port: u16,
        path: String,
        http_headers: Vec<(String, String)>,
    },
    TcpSocket {
        host: Option<String>,
        port: u16,
    },
    Exec {
        command: Vec<String>,
    },
    Grpc {
        port: u16,
        service: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpScheme {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeSpec {
    pub kind: ProbeKind,
    pub action: ProbeAction,
    pub initial_delay_seconds: u32,
    pub period_seconds: u32,
    pub timeout_seconds: u32,
    pub success_threshold: u32,
    pub failure_threshold: u32,
    /// Optional grace override (k8s 1.25+).
    pub termination_grace_period_seconds: Option<u32>,
}

impl ProbeSpec {
    /// Validate the configuration; matches upstream defaulting / validation.
    pub fn validate(&self) -> Result<(), String> {
        if self.period_seconds == 0 {
            return Err("periodSeconds must be > 0".into());
        }
        if self.timeout_seconds == 0 {
            return Err("timeoutSeconds must be > 0".into());
        }
        if self.success_threshold == 0 {
            return Err("successThreshold must be > 0".into());
        }
        if self.failure_threshold == 0 {
            return Err("failureThreshold must be > 0".into());
        }
        // Liveness/Startup: successThreshold must be 1.
        if matches!(self.kind, ProbeKind::Liveness | ProbeKind::Startup)
            && self.success_threshold != 1
        {
            return Err(format!(
                "{:?} probe must have successThreshold=1",
                self.kind
            ));
        }
        match &self.action {
            ProbeAction::HttpGet { port, .. }
            | ProbeAction::TcpSocket { port, .. }
            | ProbeAction::Grpc { port, .. } => {
                if *port == 0 {
                    return Err("port must be in 1..=65535".into());
                }
            }
            ProbeAction::Exec { command } => {
                if command.is_empty() {
                    return Err("exec command must not be empty".into());
                }
            }
        }
        Ok(())
    }

    pub fn http_get(port: u16, path: &str) -> Self {
        Self {
            kind: ProbeKind::Liveness,
            action: ProbeAction::HttpGet {
                scheme: HttpScheme::Http,
                host: None,
                port,
                path: path.to_string(),
                http_headers: vec![],
            },
            initial_delay_seconds: 0,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
            termination_grace_period_seconds: None,
        }
    }

    pub fn tcp(port: u16) -> Self {
        Self {
            kind: ProbeKind::Liveness,
            action: ProbeAction::TcpSocket { host: None, port },
            initial_delay_seconds: 0,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
            termination_grace_period_seconds: None,
        }
    }

    pub fn exec(command: Vec<String>) -> Self {
        Self {
            kind: ProbeKind::Liveness,
            action: ProbeAction::Exec { command },
            initial_delay_seconds: 0,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
            termination_grace_period_seconds: None,
        }
    }

    pub fn grpc(port: u16) -> Self {
        Self {
            kind: ProbeKind::Liveness,
            action: ProbeAction::Grpc {
                port,
                service: None,
            },
            initial_delay_seconds: 0,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
            termination_grace_period_seconds: None,
        }
    }
}

/// Per-container probe runtime state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeWorkerState {
    pub last_outcome: ProbeOutcome,
    pub consecutive_success: u32,
    pub consecutive_failure: u32,
    pub last_run_at: Option<DateTime<Utc>>,
    pub container_started_at: DateTime<Utc>,
    pub started_seen_success: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeOutcome {
    /// Initial state — probe hasn't completed yet (still in initialDelay or
    /// hasn't crossed thresholds). Treated as Success for liveness, Failure
    /// for readiness — see `effective_outcome`.
    Unknown,
    Success,
    Failure,
}

impl ProbeWorkerState {
    pub fn new(container_started_at: DateTime<Utc>) -> Self {
        Self {
            last_outcome: ProbeOutcome::Unknown,
            consecutive_success: 0,
            consecutive_failure: 0,
            last_run_at: None,
            container_started_at,
            started_seen_success: false,
        }
    }

    /// Whether the probe is allowed to fire at `now` given initialDelay
    /// and periodSeconds since the last run.
    pub fn should_fire(&self, spec: &ProbeSpec, now: DateTime<Utc>) -> bool {
        let elapsed = now - self.container_started_at;
        if elapsed < Duration::seconds(spec.initial_delay_seconds as i64) {
            return false;
        }
        match self.last_run_at {
            None => true,
            Some(t) => now - t >= Duration::seconds(spec.period_seconds as i64),
        }
    }

    /// Record a probe sample; updates counters and last_outcome with thresholding.
    pub fn record(&mut self, spec: &ProbeSpec, sample: ProbeResult, now: DateTime<Utc>) {
        self.last_run_at = Some(now);
        match sample {
            ProbeResult::Success => {
                self.consecutive_success += 1;
                self.consecutive_failure = 0;
                if self.consecutive_success >= spec.success_threshold {
                    self.last_outcome = ProbeOutcome::Success;
                    self.started_seen_success = true;
                }
            }
            ProbeResult::Failure | ProbeResult::Unknown => {
                self.consecutive_failure += 1;
                self.consecutive_success = 0;
                if self.consecutive_failure >= spec.failure_threshold {
                    self.last_outcome = ProbeOutcome::Failure;
                }
            }
        }
    }

    /// Effective outcome consumed by the kubelet sync loop: defaults differ by kind
    /// when thresholds haven't been crossed yet.
    pub fn effective_outcome(&self, spec: &ProbeSpec) -> ProbeOutcome {
        if self.last_outcome != ProbeOutcome::Unknown {
            return self.last_outcome;
        }
        // Pre-threshold defaults match upstream prober/results.go.
        match spec.kind {
            // Liveness: default-success until proven failing, so a slow-to-start
            // container isn't killed.
            ProbeKind::Liveness => ProbeOutcome::Success,
            // Readiness: default-failure until proven ready (not Ready in service endpoints).
            ProbeKind::Readiness => ProbeOutcome::Failure,
            // Startup: default-failure (gates liveness/readiness).
            ProbeKind::Startup => ProbeOutcome::Failure,
        }
    }
}

/// Decision the prober emits for the kubelet sync loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProberAction {
    /// Nothing to do this tick.
    NoOp,
    /// Container failed liveness — restart per RestartPolicy.
    RestartContainer,
    /// Mark Ready=true.
    MarkReady,
    /// Mark Ready=false.
    MarkNotReady,
    /// Startup probe finished successfully — liveness/readiness can run.
    StartupComplete,
    /// Startup probe exhausted failure threshold — restart.
    StartupFailed,
}

/// Per-tick decision from the prober.
pub fn decide(spec: &ProbeSpec, state: &ProbeWorkerState) -> ProberAction {
    match (spec.kind, state.effective_outcome(spec)) {
        (ProbeKind::Liveness, ProbeOutcome::Failure) => ProberAction::RestartContainer,
        (ProbeKind::Liveness, _) => ProberAction::NoOp,
        (ProbeKind::Readiness, ProbeOutcome::Success) => ProberAction::MarkReady,
        (ProbeKind::Readiness, _) => ProberAction::MarkNotReady,
        (ProbeKind::Startup, ProbeOutcome::Success) => ProberAction::StartupComplete,
        (ProbeKind::Startup, ProbeOutcome::Failure) => ProberAction::StartupFailed,
        (ProbeKind::Startup, ProbeOutcome::Unknown) => ProberAction::NoOp,
    }
}

/// HTTP probe: kubelet semantics — 2xx/3xx success, anything else failure.
pub fn http_status_to_result(status: u16) -> ProbeResult {
    if (200..400).contains(&status) {
        ProbeResult::Success
    } else {
        ProbeResult::Failure
    }
}

/// gRPC health: SERVING → success; anything else → failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrpcServingStatus {
    Unknown,
    Serving,
    NotServing,
    ServiceUnknown,
}

pub fn grpc_status_to_result(s: GrpcServingStatus) -> ProbeResult {
    match s {
        GrpcServingStatus::Serving => ProbeResult::Success,
        GrpcServingStatus::NotServing => ProbeResult::Failure,
        GrpcServingStatus::Unknown => ProbeResult::Unknown,
        GrpcServingStatus::ServiceUnknown => ProbeResult::Failure,
    }
}

/// Exec: nonzero exit → failure.
pub fn exec_exit_to_result(exit_code: i32) -> ProbeResult {
    if exit_code == 0 {
        ProbeResult::Success
    } else {
        ProbeResult::Failure
    }
}

/// Manages probe state for many containers, indexed by (pod_uid, container_name, kind).
#[derive(Debug, Default)]
pub struct ProberManager {
    states: BTreeMap<ProbeKey, (ProbeSpec, ProbeWorkerState)>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProbeKey {
    pub pod_uid: String,
    pub container: String,
    pub kind: ProbeKind,
}

impl ProberManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        pod_uid: &str,
        container: &str,
        spec: ProbeSpec,
        container_started_at: DateTime<Utc>,
    ) -> Result<(), String> {
        spec.validate()?;
        let key = ProbeKey {
            pod_uid: pod_uid.to_string(),
            container: container.to_string(),
            kind: spec.kind,
        };
        self.states
            .insert(key, (spec, ProbeWorkerState::new(container_started_at)));
        Ok(())
    }

    pub fn deregister(&mut self, pod_uid: &str, container: &str) {
        self.states
            .retain(|k, _| !(k.pod_uid == pod_uid && k.container == container));
    }

    pub fn record_sample(
        &mut self,
        pod_uid: &str,
        container: &str,
        kind: ProbeKind,
        sample: ProbeResult,
        now: DateTime<Utc>,
    ) -> Option<ProberAction> {
        let key = ProbeKey {
            pod_uid: pod_uid.to_string(),
            container: container.to_string(),
            kind,
        };
        let (spec, state) = self.states.get_mut(&key)?;
        state.record(spec, sample, now);
        Some(decide(spec, state))
    }

    pub fn snapshot(
        &self,
        pod_uid: &str,
        container: &str,
        kind: ProbeKind,
    ) -> Option<(ProbeSpec, ProbeWorkerState)> {
        let key = ProbeKey {
            pod_uid: pod_uid.to_string(),
            container: container.to_string(),
            kind,
        };
        self.states.get(&key).cloned()
    }

    /// Returns whether the container has any probes registered.
    pub fn has_any(&self, pod_uid: &str, container: &str) -> bool {
        self.states
            .keys()
            .any(|k| k.pod_uid == pod_uid && k.container == container)
    }

    /// Per-spec gating: until startup probe succeeds, liveness/readiness do not run.
    pub fn liveness_should_run(&self, pod_uid: &str, container: &str) -> bool {
        let startup = self.snapshot(pod_uid, container, ProbeKind::Startup);
        match startup {
            None => true,
            Some((_, s)) => s.started_seen_success,
        }
    }

    pub fn readiness_should_run(&self, pod_uid: &str, container: &str) -> bool {
        self.liveness_should_run(pod_uid, container)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn http_status_2xx_is_success() {
        assert_eq!(http_status_to_result(200), ProbeResult::Success);
        assert_eq!(http_status_to_result(204), ProbeResult::Success);
        assert_eq!(http_status_to_result(301), ProbeResult::Success);
        assert_eq!(http_status_to_result(399), ProbeResult::Success);
    }

    #[test]
    fn http_status_4xx_5xx_is_failure() {
        assert_eq!(http_status_to_result(400), ProbeResult::Failure);
        assert_eq!(http_status_to_result(404), ProbeResult::Failure);
        assert_eq!(http_status_to_result(500), ProbeResult::Failure);
        assert_eq!(http_status_to_result(599), ProbeResult::Failure);
    }

    #[test]
    fn grpc_serving_success_others_fail() {
        assert_eq!(
            grpc_status_to_result(GrpcServingStatus::Serving),
            ProbeResult::Success
        );
        assert_eq!(
            grpc_status_to_result(GrpcServingStatus::NotServing),
            ProbeResult::Failure
        );
        assert_eq!(
            grpc_status_to_result(GrpcServingStatus::ServiceUnknown),
            ProbeResult::Failure
        );
        assert_eq!(
            grpc_status_to_result(GrpcServingStatus::Unknown),
            ProbeResult::Unknown
        );
    }

    #[test]
    fn exec_zero_exit_success() {
        assert_eq!(exec_exit_to_result(0), ProbeResult::Success);
        assert_eq!(exec_exit_to_result(1), ProbeResult::Failure);
        assert_eq!(exec_exit_to_result(137), ProbeResult::Failure);
    }

    #[test]
    fn validate_rejects_zero_period() {
        let mut s = ProbeSpec::http_get(8080, "/health");
        s.period_seconds = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_timeout() {
        let mut s = ProbeSpec::http_get(8080, "/health");
        s.timeout_seconds = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_success_threshold() {
        let mut s = ProbeSpec::http_get(8080, "/health");
        s.success_threshold = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_failure_threshold() {
        let mut s = ProbeSpec::http_get(8080, "/health");
        s.failure_threshold = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_liveness_must_have_success_threshold_one() {
        let mut s = ProbeSpec::http_get(8080, "/health");
        s.kind = ProbeKind::Liveness;
        s.success_threshold = 2;
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_startup_must_have_success_threshold_one() {
        let mut s = ProbeSpec::http_get(8080, "/health");
        s.kind = ProbeKind::Startup;
        s.success_threshold = 3;
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_readiness_can_have_higher_success_threshold() {
        let mut s = ProbeSpec::http_get(8080, "/health");
        s.kind = ProbeKind::Readiness;
        s.success_threshold = 3;
        assert!(s.validate().is_ok());
    }

    #[test]
    fn validate_rejects_zero_port() {
        let mut s = ProbeSpec::tcp(0);
        assert!(s.validate().is_err());
        s = ProbeSpec::http_get(0, "/h");
        assert!(s.validate().is_err());
        s = ProbeSpec::grpc(0);
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_exec() {
        let s = ProbeSpec::exec(vec![]);
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_accepts_default_http() {
        ProbeSpec::http_get(8080, "/health").validate().unwrap();
    }

    #[test]
    fn should_fire_blocked_during_initial_delay() {
        let started = Utc::now();
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.initial_delay_seconds = 30;
        let s = ProbeWorkerState::new(started);
        assert!(!s.should_fire(&spec, started + Duration::seconds(10)));
        assert!(s.should_fire(&spec, started + Duration::seconds(31)));
    }

    #[test]
    fn should_fire_after_period_elapsed() {
        let started = Utc::now();
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.period_seconds = 5;
        let mut s = ProbeWorkerState::new(started);
        s.last_run_at = Some(started);
        assert!(!s.should_fire(&spec, started + Duration::seconds(4)));
        assert!(s.should_fire(&spec, started + Duration::seconds(5)));
    }

    #[test]
    fn liveness_failure_threshold_reached_marks_failure() {
        let started = Utc::now();
        let spec = ProbeSpec::http_get(80, "/h");
        let mut s = ProbeWorkerState::new(started);
        s.record(&spec, ProbeResult::Failure, started);
        assert_eq!(s.last_outcome, ProbeOutcome::Unknown);
        s.record(&spec, ProbeResult::Failure, started);
        assert_eq!(s.last_outcome, ProbeOutcome::Unknown);
        s.record(&spec, ProbeResult::Failure, started);
        assert_eq!(s.last_outcome, ProbeOutcome::Failure);
    }

    #[test]
    fn liveness_success_resets_failure_counter() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.failure_threshold = 3;
        let mut s = ProbeWorkerState::new(Utc::now());
        s.record(&spec, ProbeResult::Failure, now());
        s.record(&spec, ProbeResult::Failure, now());
        s.record(&spec, ProbeResult::Success, now());
        assert_eq!(s.consecutive_failure, 0);
        assert_eq!(s.consecutive_success, 1);
    }

    #[test]
    fn readiness_success_threshold_three_requires_three_in_a_row() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.kind = ProbeKind::Readiness;
        spec.success_threshold = 3;
        let mut s = ProbeWorkerState::new(now());
        s.record(&spec, ProbeResult::Success, now());
        assert_eq!(s.last_outcome, ProbeOutcome::Unknown);
        s.record(&spec, ProbeResult::Success, now());
        assert_eq!(s.last_outcome, ProbeOutcome::Unknown);
        s.record(&spec, ProbeResult::Success, now());
        assert_eq!(s.last_outcome, ProbeOutcome::Success);
    }

    #[test]
    fn liveness_pre_threshold_default_is_success() {
        let spec = ProbeSpec::http_get(80, "/h");
        let s = ProbeWorkerState::new(now());
        assert_eq!(s.effective_outcome(&spec), ProbeOutcome::Success);
    }

    #[test]
    fn readiness_pre_threshold_default_is_failure() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.kind = ProbeKind::Readiness;
        let s = ProbeWorkerState::new(now());
        assert_eq!(s.effective_outcome(&spec), ProbeOutcome::Failure);
    }

    #[test]
    fn startup_pre_threshold_default_is_failure() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.kind = ProbeKind::Startup;
        let s = ProbeWorkerState::new(now());
        assert_eq!(s.effective_outcome(&spec), ProbeOutcome::Failure);
    }

    #[test]
    fn liveness_failure_decision_restart() {
        let spec = ProbeSpec::http_get(80, "/h");
        let mut s = ProbeWorkerState::new(now());
        for _ in 0..3 {
            s.record(&spec, ProbeResult::Failure, now());
        }
        assert_eq!(decide(&spec, &s), ProberAction::RestartContainer);
    }

    #[test]
    fn liveness_success_decision_noop() {
        let spec = ProbeSpec::http_get(80, "/h");
        let mut s = ProbeWorkerState::new(now());
        s.record(&spec, ProbeResult::Success, now());
        assert_eq!(decide(&spec, &s), ProberAction::NoOp);
    }

    #[test]
    fn readiness_decision_marks_ready_or_not() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.kind = ProbeKind::Readiness;
        let mut s = ProbeWorkerState::new(now());
        // Pre-threshold → MarkNotReady.
        assert_eq!(decide(&spec, &s), ProberAction::MarkNotReady);
        s.record(&spec, ProbeResult::Success, now());
        assert_eq!(decide(&spec, &s), ProberAction::MarkReady);
        s.record(&spec, ProbeResult::Failure, now());
        s.record(&spec, ProbeResult::Failure, now());
        s.record(&spec, ProbeResult::Failure, now());
        assert_eq!(decide(&spec, &s), ProberAction::MarkNotReady);
    }

    #[test]
    fn startup_decision_complete_on_success() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.kind = ProbeKind::Startup;
        let mut s = ProbeWorkerState::new(now());
        s.record(&spec, ProbeResult::Success, now());
        assert_eq!(decide(&spec, &s), ProberAction::StartupComplete);
    }

    #[test]
    fn startup_decision_failed_after_threshold() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.kind = ProbeKind::Startup;
        spec.failure_threshold = 30;
        let mut s = ProbeWorkerState::new(now());
        for _ in 0..30 {
            s.record(&spec, ProbeResult::Failure, now());
        }
        assert_eq!(decide(&spec, &s), ProberAction::StartupFailed);
    }

    #[test]
    fn manager_register_validates() {
        let mut m = ProberManager::new();
        let mut bad = ProbeSpec::http_get(80, "/h");
        bad.period_seconds = 0;
        assert!(m.register("p", "c", bad, now()).is_err());
    }

    #[test]
    fn manager_record_sample_returns_decision() {
        let mut m = ProberManager::new();
        let spec = ProbeSpec::http_get(80, "/h");
        m.register("p", "c", spec, now()).unwrap();
        let act = m
            .record_sample("p", "c", ProbeKind::Liveness, ProbeResult::Success, now())
            .unwrap();
        assert_eq!(act, ProberAction::NoOp);
    }

    #[test]
    fn manager_record_failure_three_times_decides_restart() {
        let mut m = ProberManager::new();
        let spec = ProbeSpec::http_get(80, "/h");
        m.register("p", "c", spec, now()).unwrap();
        for _ in 0..2 {
            m.record_sample("p", "c", ProbeKind::Liveness, ProbeResult::Failure, now())
                .unwrap();
        }
        let act = m
            .record_sample("p", "c", ProbeKind::Liveness, ProbeResult::Failure, now())
            .unwrap();
        assert_eq!(act, ProberAction::RestartContainer);
    }

    #[test]
    fn manager_deregister_removes_all_kinds_for_container() {
        let mut m = ProberManager::new();
        let mut live = ProbeSpec::http_get(80, "/h");
        live.kind = ProbeKind::Liveness;
        let mut ready = ProbeSpec::http_get(80, "/h");
        ready.kind = ProbeKind::Readiness;
        m.register("p", "c", live, now()).unwrap();
        m.register("p", "c", ready, now()).unwrap();
        m.deregister("p", "c");
        assert!(!m.has_any("p", "c"));
    }

    #[test]
    fn liveness_does_not_run_until_startup_succeeds() {
        let mut m = ProberManager::new();
        let mut startup = ProbeSpec::http_get(80, "/h");
        startup.kind = ProbeKind::Startup;
        m.register("p", "c", startup, now()).unwrap();
        assert!(!m.liveness_should_run("p", "c"));
        m.record_sample("p", "c", ProbeKind::Startup, ProbeResult::Success, now())
            .unwrap();
        assert!(m.liveness_should_run("p", "c"));
    }

    #[test]
    fn liveness_runs_when_no_startup_probe_configured() {
        let mut m = ProberManager::new();
        let live = ProbeSpec::http_get(80, "/h");
        m.register("p", "c", live, now()).unwrap();
        assert!(m.liveness_should_run("p", "c"));
    }

    #[test]
    fn snapshot_returns_state_copy() {
        let mut m = ProberManager::new();
        let spec = ProbeSpec::http_get(80, "/h");
        m.register("p", "c", spec, now()).unwrap();
        let (_, s) = m.snapshot("p", "c", ProbeKind::Liveness).unwrap();
        assert_eq!(s.consecutive_failure, 0);
    }

    #[test]
    fn snapshot_unknown_returns_none() {
        let m = ProberManager::new();
        assert!(m.snapshot("p", "c", ProbeKind::Liveness).is_none());
    }

    #[test]
    fn record_unknown_treated_as_failure_for_threshold() {
        let mut m = ProberManager::new();
        let spec = ProbeSpec::http_get(80, "/h");
        m.register("p", "c", spec, now()).unwrap();
        for _ in 0..3 {
            m.record_sample("p", "c", ProbeKind::Liveness, ProbeResult::Unknown, now())
                .unwrap();
        }
        let (_, s) = m.snapshot("p", "c", ProbeKind::Liveness).unwrap();
        assert_eq!(s.last_outcome, ProbeOutcome::Failure);
    }

    #[test]
    fn termination_grace_period_override_persists_in_spec() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.termination_grace_period_seconds = Some(15);
        spec.validate().unwrap();
        assert_eq!(spec.termination_grace_period_seconds, Some(15));
    }

    #[test]
    fn http_action_carries_headers() {
        if let ProbeAction::HttpGet {
            ref mut http_headers,
            ..
        } = ProbeSpec::http_get(80, "/h").action
        {
            http_headers.push(("X-Token".into(), "abc".into()));
            assert_eq!(http_headers.len(), 1);
        }
    }

    #[test]
    fn record_sample_unknown_container_returns_none() {
        let mut m = ProberManager::new();
        assert!(m
            .record_sample("p", "c", ProbeKind::Liveness, ProbeResult::Success, now())
            .is_none());
    }

    #[test]
    fn manager_has_any_works() {
        let mut m = ProberManager::new();
        let spec = ProbeSpec::http_get(80, "/h");
        m.register("p", "c", spec, now()).unwrap();
        assert!(m.has_any("p", "c"));
        assert!(!m.has_any("p", "other"));
    }

    #[test]
    fn higher_success_threshold_for_readiness() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.kind = ProbeKind::Readiness;
        spec.success_threshold = 5;
        let mut s = ProbeWorkerState::new(now());
        for _ in 0..4 {
            s.record(&spec, ProbeResult::Success, now());
        }
        assert_eq!(s.last_outcome, ProbeOutcome::Unknown);
        s.record(&spec, ProbeResult::Success, now());
        assert_eq!(s.last_outcome, ProbeOutcome::Success);
    }

    #[test]
    fn streak_breaks_reset_streak() {
        let spec = ProbeSpec::http_get(80, "/h");
        let mut s = ProbeWorkerState::new(now());
        s.record(&spec, ProbeResult::Success, now());
        s.record(&spec, ProbeResult::Failure, now());
        assert_eq!(s.consecutive_success, 0);
        assert_eq!(s.consecutive_failure, 1);
    }

    #[test]
    fn liveness_outcome_persists_until_streak_recovers() {
        let mut spec = ProbeSpec::http_get(80, "/h");
        spec.failure_threshold = 2;
        let mut s = ProbeWorkerState::new(now());
        s.record(&spec, ProbeResult::Failure, now());
        s.record(&spec, ProbeResult::Failure, now());
        assert_eq!(s.last_outcome, ProbeOutcome::Failure);
        // One success doesn't flip back to Success because success_threshold=1
        // for liveness — it does flip immediately. Let's verify that.
        s.record(&spec, ProbeResult::Success, now());
        assert_eq!(s.last_outcome, ProbeOutcome::Success);
    }
}
