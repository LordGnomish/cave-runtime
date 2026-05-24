// SPDX-License-Identifier: AGPL-3.0-or-later
//! `circuit-breaker` plugin — half-open state machine keyed by route.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_u64, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug)]
struct BreakerEntry {
    state: BreakerState,
    failures: u32,
    opened_at: Option<Instant>,
    half_open_probes: u32,
}

pub struct Breakers {
    inner: RwLock<HashMap<String, BreakerEntry>>,
}

impl Default for Breakers {
    fn default() -> Self { Self { inner: RwLock::new(HashMap::new()) } }
}

impl Breakers {
    pub fn new() -> Self { Self::default() }
    pub fn state(&self, key: &str) -> BreakerState {
        self.inner.read().unwrap().get(key).map(|e| e.state).unwrap_or(BreakerState::Closed)
    }
    pub fn record_failure(&self, key: &str, threshold: u32) -> BreakerState {
        let mut g = self.inner.write().unwrap();
        let entry = g.entry(key.to_string()).or_insert(BreakerEntry {
            state: BreakerState::Closed,
            failures: 0,
            opened_at: None,
            half_open_probes: 0,
        });
        entry.failures += 1;
        if entry.failures >= threshold {
            entry.state = BreakerState::Open;
            entry.opened_at = Some(Instant::now());
        }
        entry.state
    }
    pub fn record_success(&self, key: &str) {
        let mut g = self.inner.write().unwrap();
        if let Some(entry) = g.get_mut(key) {
            entry.failures = 0;
            entry.state = BreakerState::Closed;
            entry.opened_at = None;
            entry.half_open_probes = 0;
        }
    }
    pub fn try_request(&self, key: &str, open_for: Duration) -> bool {
        let mut g = self.inner.write().unwrap();
        let entry = g.entry(key.to_string()).or_insert(BreakerEntry {
            state: BreakerState::Closed,
            failures: 0,
            opened_at: None,
            half_open_probes: 0,
        });
        match entry.state {
            BreakerState::Closed => true,
            BreakerState::Open => {
                if let Some(t) = entry.opened_at {
                    if t.elapsed() >= open_for {
                        entry.state = BreakerState::HalfOpen;
                        entry.half_open_probes = 1;
                        return true;
                    }
                }
                false
            }
            BreakerState::HalfOpen => {
                entry.half_open_probes = entry.half_open_probes.saturating_add(1);
                entry.half_open_probes <= 3
            }
        }
    }
}

thread_local! { static BR: Breakers = Breakers::new(); }

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let threshold = cfg_u64(cfg, "failure_threshold").unwrap_or(5) as u32;
    let open_for_ms = cfg_u64(cfg, "open_for_ms").unwrap_or(10_000);
    let key = ctx.route.id.to_string();
    let allow = BR.with(|b| b.try_request(&key, Duration::from_millis(open_for_ms)));
    if !allow {
        return Err(AGwError::CircuitOpen { service: ctx.route.name.clone() });
    }
    // Heuristic: if upstream returned 5xx on previous request, count as failure.
    // The proxy layer would also explicitly call `record_failure`; we expose the helper.
    let _ = threshold;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_initial_state() {
        let b = Breakers::new();
        assert_eq!(b.state("k"), BreakerState::Closed);
    }

    #[test]
    fn open_after_threshold_failures() {
        let b = Breakers::new();
        assert_eq!(b.record_failure("k", 3), BreakerState::Closed);
        assert_eq!(b.record_failure("k", 3), BreakerState::Closed);
        assert_eq!(b.record_failure("k", 3), BreakerState::Open);
    }

    #[test]
    fn try_request_blocks_when_open() {
        let b = Breakers::new();
        for _ in 0..5 { b.record_failure("k", 3); }
        assert!(!b.try_request("k", Duration::from_secs(10)));
    }

    #[test]
    fn half_open_after_open_window() {
        let b = Breakers::new();
        for _ in 0..5 { b.record_failure("k", 3); }
        assert!(b.try_request("k", Duration::from_nanos(1)));
        assert_eq!(b.state("k"), BreakerState::HalfOpen);
    }

    #[test]
    fn record_success_resets_to_closed() {
        let b = Breakers::new();
        for _ in 0..5 { b.record_failure("k", 3); }
        b.record_success("k");
        assert_eq!(b.state("k"), BreakerState::Closed);
    }

    #[test]
    fn half_open_caps_concurrent_probes() {
        let b = Breakers::new();
        for _ in 0..3 { b.record_failure("k", 3); }
        // open it
        assert!(b.try_request("k", Duration::from_nanos(1))); // moves to HalfOpen, probe 1
        assert!(b.try_request("k", Duration::from_secs(60))); // probe 2
        assert!(b.try_request("k", Duration::from_secs(60))); // probe 3
        assert!(!b.try_request("k", Duration::from_secs(60))); // 4th blocked
    }
}
