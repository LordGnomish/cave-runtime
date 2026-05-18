// SPDX-License-Identifier: AGPL-3.0-or-later
//! Container health checks — exec, http, tcp.
//!
//! Mirrors Docker/containerd health check semantics:
//!  - exec:  run command inside container, healthy if exit code == 0
//!  - http:  GET url, healthy if status code matches expected (default 200)
//!  - tcp:   connect to host:port, healthy if connection succeeds
//!
//! After `retries` consecutive failures the state moves to Unhealthy.
//! While waiting for `start_period_secs`, failures do not count against
//! the streak (state stays Starting).

use crate::error::CriResult;
use crate::models::{HealthCheck, HealthCheckKind, HealthState, HealthStatus};
use chrono::Utc;
use std::net::TcpStream;
use std::time::Duration;

/// Run a single health check probe and return updated HealthStatus.
pub async fn run_check(
    check: &HealthCheck,
    current: &HealthStatus,
    container_started_at: Option<chrono::DateTime<Utc>>,
) -> CriResult<HealthStatus> {
    let in_start_period = is_in_start_period(container_started_at, check.start_period_secs);

    let (success, output) = probe(check).await;

    let mut next = current.clone();
    next.last_checked_at = Some(Utc::now());
    next.last_output = output;

    if success {
        next.failing_streak = 0;
        next.state = HealthState::Healthy;
    } else if in_start_period {
        // During start_period, keep Starting regardless of probe result
        next.state = HealthState::Starting;
    } else {
        next.failing_streak = next.failing_streak.saturating_add(1);
        if next.failing_streak >= check.retries {
            next.state = HealthState::Unhealthy;
        }
        // else stay Healthy/Starting until retries exhausted
    }

    Ok(next)
}

fn is_in_start_period(
    started_at: Option<chrono::DateTime<Utc>>,
    start_period_secs: u64,
) -> bool {
    if start_period_secs == 0 {
        return false;
    }
    match started_at {
        Some(t) => {
            let elapsed = Utc::now().signed_duration_since(t).num_seconds();
            elapsed < start_period_secs as i64
        }
        None => true,
    }
}

/// Run the probe, return (success, output_message).
async fn probe(check: &HealthCheck) -> (bool, String) {
    let timeout = Duration::from_secs(check.timeout_secs.max(1));

    match &check.kind {
        HealthCheckKind::Exec { command } => exec_probe(command, timeout).await,
        HealthCheckKind::Http { url, expected_status } => {
            http_probe(url, *expected_status, timeout).await
        }
        HealthCheckKind::Tcp { host, port } => tcp_probe(host, *port, timeout),
    }
}

async fn exec_probe(command: &[String], timeout: Duration) -> (bool, String) {
    if command.is_empty() {
        return (false, "exec health check: command is empty".into());
    }

    let result = tokio::time::timeout(timeout, async {
        let mut cmd = tokio::process::Command::new(&command[0]);
        if command.len() > 1 {
            cmd.args(&command[1..]);
        }
        cmd.output().await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            let success = output.status.success();
            let out = String::from_utf8_lossy(&output.stdout).into_owned()
                + &String::from_utf8_lossy(&output.stderr);
            (success, out.trim().to_string())
        }
        Ok(Err(e)) => (false, format!("exec failed: {}", e)),
        Err(_) => (false, "exec health check timed out".into()),
    }
}

async fn http_probe(url: &str, expected_status: u16, timeout: Duration) -> (bool, String) {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_default();

    match client.get(url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let success = if expected_status == 0 {
                status >= 200 && status < 400
            } else {
                status == expected_status
            };
            (success, format!("HTTP {}", status))
        }
        Err(e) => (false, format!("http probe failed: {}", e)),
    }
}

fn tcp_probe(host: &str, port: u16, timeout: Duration) -> (bool, String) {
    let addr = format!("{}:{}", host, port);
    match TcpStream::connect_timeout(&addr.parse().unwrap_or_else(|_| "127.0.0.1:80".parse().unwrap()), timeout) {
        Ok(_) => (true, format!("TCP connection to {} succeeded", addr)),
        Err(e) => (false, format!("TCP probe failed: {}", e)),
    }
}

/// Build the initial HealthStatus for a new container.
pub fn initial_status() -> HealthStatus {
    HealthStatus {
        state: HealthState::Starting,
        failing_streak: 0,
        last_output: String::new(),
        last_checked_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{HealthCheck, HealthCheckKind, HealthState, HealthStatus};
    use chrono::Utc;

    fn default_check(kind: HealthCheckKind) -> HealthCheck {
        HealthCheck {
            kind,
            interval_secs: 30,
            timeout_secs: 5,
            retries: 3,
            start_period_secs: 0,
        }
    }

    fn starting_status() -> HealthStatus {
        HealthStatus {
            state: HealthState::Starting,
            failing_streak: 0,
            last_output: String::new(),
            last_checked_at: None,
        }
    }

    // ── exec probe ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn exec_true_command_is_healthy() {
        let check = default_check(HealthCheckKind::Exec { command: vec!["true".into()] });
        let next = run_check(&check, &starting_status(), Some(Utc::now())).await.unwrap();
        assert_eq!(next.state, HealthState::Healthy);
        assert_eq!(next.failing_streak, 0);
    }

    #[tokio::test]
    async fn exec_false_command_increments_streak() {
        let check = default_check(HealthCheckKind::Exec { command: vec!["false".into()] });
        let next = run_check(&check, &starting_status(), Some(Utc::now())).await.unwrap();
        assert_eq!(next.failing_streak, 1);
        // Not yet Unhealthy (retries=3)
        assert_ne!(next.state, HealthState::Unhealthy);
    }

    #[tokio::test]
    async fn exec_three_consecutive_failures_become_unhealthy() {
        let check = default_check(HealthCheckKind::Exec { command: vec!["false".into()] });
        let mut status = starting_status();
        for _ in 0..3 {
            status = run_check(&check, &status, Some(Utc::now())).await.unwrap();
        }
        assert_eq!(status.state, HealthState::Unhealthy);
    }

    #[tokio::test]
    async fn exec_recovery_resets_streak() {
        let check = default_check(HealthCheckKind::Exec { command: vec!["false".into()] });
        let mut status = starting_status();
        // Two failures
        for _ in 0..2 {
            status = run_check(&check, &status, Some(Utc::now())).await.unwrap();
        }
        assert_eq!(status.failing_streak, 2);

        // Recovery with true
        let ok_check = default_check(HealthCheckKind::Exec { command: vec!["true".into()] });
        status = run_check(&ok_check, &status, Some(Utc::now())).await.unwrap();
        assert_eq!(status.failing_streak, 0);
        assert_eq!(status.state, HealthState::Healthy);
    }

    #[tokio::test]
    async fn exec_empty_command_fails() {
        let check = default_check(HealthCheckKind::Exec { command: vec![] });
        let next = run_check(&check, &starting_status(), Some(Utc::now())).await.unwrap();
        assert_eq!(next.failing_streak, 1);
        assert!(next.last_output.contains("empty"));
    }

    #[tokio::test]
    async fn exec_nonexistent_command_fails() {
        let check = default_check(HealthCheckKind::Exec { command: vec!["/nonexistent/binary".into()] });
        let next = run_check(&check, &starting_status(), Some(Utc::now())).await.unwrap();
        assert_eq!(next.failing_streak, 1);
    }

    #[tokio::test]
    async fn exec_with_args() {
        let check = default_check(HealthCheckKind::Exec {
            command: vec!["sh".into(), "-c".into(), "exit 0".into()],
        });
        let next = run_check(&check, &starting_status(), Some(Utc::now())).await.unwrap();
        assert_eq!(next.state, HealthState::Healthy);
    }

    // ── start_period ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn failures_during_start_period_keep_starting_state() {
        let mut check = default_check(HealthCheckKind::Exec { command: vec!["false".into()] });
        check.start_period_secs = 3600; // 1 hour start period

        let started_at = Utc::now(); // just started
        let mut status = starting_status();
        for _ in 0..5 {
            status = run_check(&check, &status, Some(started_at)).await.unwrap();
        }
        assert_eq!(status.state, HealthState::Starting);
    }

    #[tokio::test]
    async fn success_during_start_period_becomes_healthy() {
        let mut check = default_check(HealthCheckKind::Exec { command: vec!["true".into()] });
        check.start_period_secs = 3600;
        let started_at = Utc::now();
        let next = run_check(&check, &starting_status(), Some(started_at)).await.unwrap();
        assert_eq!(next.state, HealthState::Healthy);
    }

    #[tokio::test]
    async fn failures_after_start_period_count() {
        let mut check = default_check(HealthCheckKind::Exec { command: vec!["false".into()] });
        check.start_period_secs = 1;
        check.retries = 1;

        // started_at far in the past (1 hour ago) → start period expired
        let started_at = Utc::now() - chrono::Duration::hours(1);
        let next = run_check(&check, &starting_status(), Some(started_at)).await.unwrap();
        assert_eq!(next.state, HealthState::Unhealthy);
    }

    // ── tcp probe ─────────────────────────────────────────────────────────────

    #[test]
    fn tcp_probe_to_closed_port_fails() {
        // Port 1 is almost certainly closed
        let (success, msg) = tcp_probe("127.0.0.1", 1, Duration::from_millis(200));
        assert!(!success, "expected failure, got: {}", msg);
    }

    #[test]
    fn tcp_probe_invalid_host_fails() {
        let (success, _msg) = tcp_probe("this-host-does-not-exist.invalid", 80, Duration::from_millis(200));
        assert!(!success);
    }

    // ── http probe ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn http_probe_to_invalid_url_fails() {
        let (success, msg) = http_probe("http://127.0.0.1:1/health", 200, Duration::from_millis(200)).await;
        assert!(!success, "expected failure, got: {}", msg);
    }

    // ── last_checked_at updated ───────────────────────────────────────────────

    #[tokio::test]
    async fn run_check_sets_last_checked_at() {
        let check = default_check(HealthCheckKind::Exec { command: vec!["true".into()] });
        let next = run_check(&check, &starting_status(), Some(Utc::now())).await.unwrap();
        assert!(next.last_checked_at.is_some());
    }

    // ── initial_status ────────────────────────────────────────────────────────

    #[test]
    fn initial_status_is_starting() {
        let s = initial_status();
        assert_eq!(s.state, HealthState::Starting);
        assert_eq!(s.failing_streak, 0);
        assert!(s.last_checked_at.is_none());
    }

    // ── HealthState serialization ─────────────────────────────────────────────

    #[test]
    fn health_state_roundtrip() {
        for state in [HealthState::Starting, HealthState::Healthy, HealthState::Unhealthy] {
            let json = serde_json::to_string(&state).unwrap();
            let back: HealthState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }
}
