//! Health check subsystem — active probes + passive observation.
//!
//! Active: periodic HTTP/TCP pings to each target.
//! Passive: intercept proxy responses; mark unhealthy on thresholds.

use crate::models::{ActiveHealthCheck, HealthCheckType, PassiveHealthCheck};
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug)]
struct TargetState {
    status: HealthStatus,
    consecutive_successes: u32,
    consecutive_failures: u32,
    consecutive_http_failures: u32,
    consecutive_tcp_failures: u32,
    consecutive_timeouts: u32,
    last_checked: Option<Instant>,
}

impl Default for TargetState {
    fn default() -> Self {
        Self {
            status: HealthStatus::Unknown,
            consecutive_successes: 0,
            consecutive_failures: 0,
            consecutive_http_failures: 0,
            consecutive_tcp_failures: 0,
            consecutive_timeouts: 0,
            last_checked: None,
        }
    }
}

#[derive(Clone)]
pub struct HealthRegistry {
    // upstream_id → target_id → state
    states: Arc<DashMap<Uuid, DashMap<Uuid, TargetState>>>,
}

impl HealthRegistry {
    pub fn new() -> Self {
        Self { states: Arc::new(DashMap::new()) }
    }

    fn _state_map_unused(&self, _upstream_id: Uuid) {
        // removed — use states directly
    }

    pub fn is_healthy(&self, upstream_id: Uuid, target_id: Uuid) -> bool {
        if let Some(up) = self.states.get(&upstream_id) {
            if let Some(t) = up.get(&target_id) {
                return t.status != HealthStatus::Unhealthy;
            }
        }
        true // unknown = assume healthy
    }

    /// Called by passive health checker after each upstream response.
    pub fn observe(
        &self,
        upstream_id: Uuid,
        target_id: Uuid,
        status_code: Option<u16>,
        is_tcp_failure: bool,
        is_timeout: bool,
        passive: &PassiveHealthCheck,
    ) {
        if !passive.enabled {
            return;
        }
        self.states.entry(upstream_id).or_insert_with(DashMap::new);
        let up = self.states.get(&upstream_id).unwrap();
        let mut entry = up.entry(target_id).or_insert_with(TargetState::default);

        if is_timeout {
            entry.consecutive_timeouts += 1;
            entry.consecutive_successes = 0;
        } else if is_tcp_failure {
            entry.consecutive_tcp_failures += 1;
            entry.consecutive_successes = 0;
        } else if let Some(code) = status_code {
            if passive.unhealthy.http_statuses.contains(&code) {
                entry.consecutive_http_failures += 1;
                entry.consecutive_successes = 0;
            } else if passive.healthy.http_statuses.contains(&code) {
                entry.consecutive_successes += 1;
                entry.consecutive_http_failures = 0;
                entry.consecutive_tcp_failures = 0;
                entry.consecutive_timeouts = 0;
            }
        }

        // Evaluate transitions
        let unhealthy = passive.unhealthy.http_failures > 0
            && entry.consecutive_http_failures >= passive.unhealthy.http_failures
            || passive.unhealthy.tcp_failures > 0
                && entry.consecutive_tcp_failures >= passive.unhealthy.tcp_failures
            || passive.unhealthy.timeouts > 0
                && entry.consecutive_timeouts >= passive.unhealthy.timeouts;

        let healthy = passive.healthy.successes > 0
            && entry.consecutive_successes >= passive.healthy.successes;

        if unhealthy {
            if entry.status != HealthStatus::Unhealthy {
                warn!(upstream=%upstream_id, target=%target_id, "target marked unhealthy (passive)");
            }
            entry.status = HealthStatus::Unhealthy;
        } else if healthy {
            entry.status = HealthStatus::Healthy;
        }
    }

    /// Active health check probe — called by background task.
    pub async fn probe_http(
        &self,
        upstream_id: Uuid,
        target_id: Uuid,
        url: &str,
        active: &ActiveHealthCheck,
    ) {
        if !active.enabled {
            return;
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(active.timeout))
            .danger_accept_invalid_certs(!active.https_verify_certificate)
            .build()
            .unwrap_or_default();

        let result = client.get(url).send().await;

        self.states.entry(upstream_id).or_insert_with(DashMap::new);
        let up = self.states.get(&upstream_id).unwrap();
        let mut entry = up.entry(target_id).or_insert_with(TargetState::default);
        entry.last_checked = Some(Instant::now());

        match result {
            Ok(resp) => {
                let code = resp.status().as_u16();
                debug!(target=%target_id, status=code, "active health probe");
                if active.healthy.http_statuses.contains(&code) {
                    entry.consecutive_successes += 1;
                    entry.consecutive_failures = 0;
                    if entry.consecutive_successes >= active.healthy.successes
                        && active.healthy.successes > 0
                    {
                        entry.status = HealthStatus::Healthy;
                    }
                } else if active.unhealthy.http_statuses.contains(&code) {
                    entry.consecutive_failures += 1;
                    entry.consecutive_successes = 0;
                    if entry.consecutive_failures >= active.unhealthy.http_failures
                        && active.unhealthy.http_failures > 0
                    {
                        warn!(target=%target_id, "target marked unhealthy (active)");
                        entry.status = HealthStatus::Unhealthy;
                    }
                }
            }
            Err(e) => {
                entry.consecutive_failures += 1;
                entry.consecutive_successes = 0;
                if e.is_timeout() {
                    entry.consecutive_timeouts += 1;
                }
                if entry.consecutive_failures >= active.unhealthy.http_failures.max(1) {
                    warn!(target=%target_id, err=%e, "target marked unhealthy (active probe failed)");
                    entry.status = HealthStatus::Unhealthy;
                }
            }
        }
    }

    pub fn get_status(&self, upstream_id: Uuid, target_id: Uuid) -> HealthStatus {
        if let Some(up) = self.states.get(&upstream_id) {
            if let Some(t) = up.get(&target_id) {
                return t.status.clone();
            }
        }
        HealthStatus::Unknown
    }

    /// Force-set a target healthy/unhealthy (Admin API endpoint).
    pub fn set_status(&self, upstream_id: Uuid, target_id: Uuid, healthy: bool) {
        self.states.entry(upstream_id).or_insert_with(DashMap::new);
        let up = self.states.get(&upstream_id).unwrap();
        let mut entry = up.entry(target_id).or_insert_with(TargetState::default);
        entry.status = if healthy { HealthStatus::Healthy } else { HealthStatus::Unhealthy };
        entry.consecutive_successes = 0;
        entry.consecutive_failures = 0;
    }
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{HealthThreshold, PassiveHealthCheck};

    fn make_passive(http_failures: u32, successes: u32) -> PassiveHealthCheck {
        PassiveHealthCheck {
            enabled: true,
            r#type: HealthCheckType::Http,
            healthy: HealthThreshold {
                successes,
                http_statuses: vec![200],
                ..Default::default()
            },
            unhealthy: HealthThreshold {
                http_failures,
                http_statuses: vec![500],
                ..Default::default()
            },
        }
    }

    #[test]
    fn marks_unhealthy_after_threshold() {
        let reg = HealthRegistry::new();
        let up = Uuid::new_v4();
        let tgt = Uuid::new_v4();
        let passive = make_passive(3, 2);

        for _ in 0..3 {
            reg.observe(up, tgt, Some(500), false, false, &passive);
        }
        assert_eq!(reg.get_status(up, tgt), HealthStatus::Unhealthy);
    }

    #[test]
    fn recovers_after_successes() {
        let reg = HealthRegistry::new();
        let up = Uuid::new_v4();
        let tgt = Uuid::new_v4();
        let passive = make_passive(3, 2);

        for _ in 0..3 {
            reg.observe(up, tgt, Some(500), false, false, &passive);
        }
        for _ in 0..2 {
            reg.observe(up, tgt, Some(200), false, false, &passive);
        }
        assert_eq!(reg.get_status(up, tgt), HealthStatus::Healthy);
    }

    #[test]
    fn force_set_status() {
        let reg = HealthRegistry::new();
        let up = Uuid::new_v4();
        let tgt = Uuid::new_v4();
        reg.set_status(up, tgt, false);
        assert!(!reg.is_healthy(up, tgt));
        reg.set_status(up, tgt, true);
        assert!(reg.is_healthy(up, tgt));
    }
}
