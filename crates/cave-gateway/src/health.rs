//! Health check logic — active probes and passive observation.
//!
//! Active health checks: periodic HTTP GET to a configured path on each target.
//! Passive health checks: recording observed success/failure on each proxied request.

use crate::models::TargetHealth;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

// ─────────────────────────────────────────────
//  Per-target observed health state
// ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TargetHealthState {
    pub consecutive_successes: u32,
    pub consecutive_failures: u32,
    pub consecutive_timeouts: u32,
    pub last_check: Option<Instant>,
    pub health: TargetHealth,
}

impl Default for TargetHealthState {
    fn default() -> Self {
        Self {
            consecutive_successes: 0,
            consecutive_failures: 0,
            consecutive_timeouts: 0,
            last_check: None,
            health: TargetHealth::Healthy,
        }
    }
}

impl TargetHealthState {
    /// Record a successful response.  `success_threshold` consecutive successes
    /// needed to move an unhealthy target back to healthy.
    pub fn record_success(&mut self, success_threshold: u32, unhealthy_statuses: &[u16]) {
        let _ = unhealthy_statuses; // status already validated by caller
        self.consecutive_failures = 0;
        self.consecutive_timeouts = 0;
        self.consecutive_successes += 1;
        self.last_check = Some(Instant::now());

        if self.health == TargetHealth::Unhealthy
            && self.consecutive_successes >= success_threshold
        {
            self.health = TargetHealth::Healthy;
        }
    }

    /// Record a failed/error response.
    pub fn record_failure(&mut self, failure_threshold: u32) {
        self.consecutive_successes = 0;
        self.consecutive_failures += 1;
        self.last_check = Some(Instant::now());

        if self.health == TargetHealth::Healthy
            && self.consecutive_failures >= failure_threshold
        {
            self.health = TargetHealth::Unhealthy;
        }
    }

    /// Record a timeout.
    pub fn record_timeout(&mut self, timeout_threshold: u32) {
        self.consecutive_successes = 0;
        self.consecutive_timeouts += 1;
        self.last_check = Some(Instant::now());

        if self.health == TargetHealth::Healthy
            && self.consecutive_timeouts >= timeout_threshold
        {
            self.health = TargetHealth::Unhealthy;
        }
    }
}

// ─────────────────────────────────────────────
//  Health checker registry
// ─────────────────────────────────────────────

#[derive(Default)]
pub struct HealthRegistry {
    /// target_id → health state
    states: HashMap<Uuid, TargetHealthState>,
}

impl HealthRegistry {
    pub fn get_or_create(&mut self, target_id: Uuid) -> &mut TargetHealthState {
        self.states.entry(target_id).or_default()
    }

    pub fn current_health(&self, target_id: Uuid) -> TargetHealth {
        self.states
            .get(&target_id)
            .map(|s| s.health.clone())
            .unwrap_or(TargetHealth::Healthy)
    }

    /// Passive health reporting: called after each proxied request.
    pub fn passive_report(
        &mut self,
        target_id: Uuid,
        status_code: u16,
        is_timeout: bool,
        success_threshold: u32,
        failure_threshold: u32,
        timeout_threshold: u32,
        healthy_statuses: &[u16],
        unhealthy_statuses: &[u16],
    ) -> TargetHealth {
        let state = self.get_or_create(target_id);

        if is_timeout {
            state.record_timeout(timeout_threshold);
        } else if unhealthy_statuses.contains(&status_code) {
            state.record_failure(failure_threshold);
        } else if healthy_statuses.contains(&status_code) {
            state.record_success(success_threshold, unhealthy_statuses);
        }

        state.health.clone()
    }

    /// Active health check result: directly set health based on probe outcome.
    pub fn active_report(
        &mut self,
        target_id: Uuid,
        probe_status: Option<u16>,
        healthy_statuses: &[u16],
        unhealthy_statuses: &[u16],
        success_threshold: u32,
        failure_threshold: u32,
    ) -> TargetHealth {
        let state = self.get_or_create(target_id);

        match probe_status {
            None => {
                // Connection error / timeout
                state.record_failure(failure_threshold);
            }
            Some(code) if unhealthy_statuses.contains(&code) => {
                state.record_failure(failure_threshold);
            }
            Some(code) if healthy_statuses.contains(&code) => {
                state.record_success(success_threshold, unhealthy_statuses);
            }
            Some(_) => {
                // Ambiguous status — do not count either way
            }
        }

        state.health.clone()
    }
}

/// Represent an upstream reference used by the health scheduler.
/// In the real implementation, this ties into the store.
pub struct UpstreamRef {
    pub upstream_id: Uuid,
    pub target_id: Uuid,
    pub target_addr: String,
    pub http_path: String,
    pub timeout: Duration,
}

/// Result of an active health probe.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub target_id: Uuid,
    pub status: Option<u16>,
    pub latency_ms: u64,
}

/// Execute an active HTTP health probe against a single target.
///
/// In tests, pass a `mock_status` to skip the real HTTP call.
pub async fn probe_target(
    target_addr: &str,
    http_path: &str,
    timeout: Duration,
    mock_status: Option<u16>,
) -> ProbeResult {
    let target_id = Uuid::new_v4(); // caller should pass the real ID
    let start = Instant::now();

    let status = if let Some(s) = mock_status {
        Some(s)
    } else {
        let url = format!("http://{target_addr}{http_path}");
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_default();

        match client.get(&url).send().await {
            Ok(resp) => Some(resp.status().as_u16()),
            Err(_) => None,
        }
    };

    ProbeResult {
        target_id,
        status,
        latency_ms: start.elapsed().as_millis() as u64,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn healthy_statuses() -> Vec<u16> {
        (200u16..=299).collect()
    }

    fn unhealthy_statuses() -> Vec<u16> {
        vec![429, 500, 502, 503, 504]
    }

    #[test]
    fn test_passive_marks_unhealthy_after_failures() {
        let mut registry = HealthRegistry::default();
        let id = Uuid::new_v4();

        // threshold = 3 consecutive failures
        for _ in 0..3 {
            registry.passive_report(
                id,
                500,
                false,
                5,
                3,
                3,
                &healthy_statuses(),
                &unhealthy_statuses(),
            );
        }

        assert_eq!(registry.current_health(id), TargetHealth::Unhealthy);
    }

    #[test]
    fn test_passive_recovers_after_successes() {
        let mut registry = HealthRegistry::default();
        let id = Uuid::new_v4();

        // First mark unhealthy
        for _ in 0..3 {
            registry.passive_report(id, 500, false, 3, 3, 3, &healthy_statuses(), &unhealthy_statuses());
        }
        assert_eq!(registry.current_health(id), TargetHealth::Unhealthy);

        // Then 3 successes → healthy
        for _ in 0..3 {
            registry.passive_report(id, 200, false, 3, 3, 3, &healthy_statuses(), &unhealthy_statuses());
        }
        assert_eq!(registry.current_health(id), TargetHealth::Healthy);
    }

    #[test]
    fn test_active_probe_marks_unhealthy() {
        let mut registry = HealthRegistry::default();
        let id = Uuid::new_v4();

        // 3 failing active probes
        for _ in 0..3 {
            registry.active_report(id, Some(503), &healthy_statuses(), &unhealthy_statuses(), 3, 3);
        }

        assert_eq!(registry.current_health(id), TargetHealth::Unhealthy);
    }

    #[test]
    fn test_timeout_marks_unhealthy() {
        let mut registry = HealthRegistry::default();
        let id = Uuid::new_v4();

        // 3 timeouts → unhealthy
        let state = registry.get_or_create(id);
        state.record_timeout(3);
        state.record_timeout(3);
        state.record_timeout(3);

        assert_eq!(registry.current_health(id), TargetHealth::Unhealthy);
    }

    #[test]
    fn test_single_failure_does_not_mark_unhealthy() {
        let mut registry = HealthRegistry::default();
        let id = Uuid::new_v4();

        // threshold = 3, so 1 failure should not trip
        registry.passive_report(id, 500, false, 3, 3, 3, &healthy_statuses(), &unhealthy_statuses());
        assert_eq!(registry.current_health(id), TargetHealth::Healthy);
    }
}
