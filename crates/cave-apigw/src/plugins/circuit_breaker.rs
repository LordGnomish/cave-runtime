// SPDX-License-Identifier: AGPL-3.0-or-later
//! `circuit-breaker` plugin — half-open state machine per route.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_u64, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState { Closed, Open, HalfOpen }

#[derive(Debug)]
struct BE {
    state: BreakerState, failures: u32, opened_at: Option<Instant>, half_open_probes: u32,
}

pub struct Breakers { inner: RwLock<HashMap<String, BE>> }
impl Default for Breakers { fn default() -> Self { Self { inner: RwLock::new(HashMap::new()) } } }
impl Breakers {
    pub fn new() -> Self { Self::default() }
    pub fn state(&self, key: &str) -> BreakerState {
        self.inner.read().unwrap().get(key).map(|e| e.state).unwrap_or(BreakerState::Closed)
    }
    pub fn record_failure(&self, key: &str, threshold: u32) -> BreakerState {
        let mut g = self.inner.write().unwrap();
        let e = g.entry(key.into()).or_insert(BE { state: BreakerState::Closed, failures: 0, opened_at: None, half_open_probes: 0 });
        e.failures += 1;
        if e.failures >= threshold { e.state = BreakerState::Open; e.opened_at = Some(Instant::now()); }
        e.state
    }
    pub fn record_success(&self, key: &str) {
        if let Some(e) = self.inner.write().unwrap().get_mut(key) {
            e.failures = 0; e.state = BreakerState::Closed; e.opened_at = None; e.half_open_probes = 0;
        }
    }
    pub fn try_request(&self, key: &str, open_for: Duration) -> bool {
        let mut g = self.inner.write().unwrap();
        let e = g.entry(key.into()).or_insert(BE { state: BreakerState::Closed, failures: 0, opened_at: None, half_open_probes: 0 });
        match e.state {
            BreakerState::Closed => true,
            BreakerState::Open => {
                if let Some(t) = e.opened_at {
                    if t.elapsed() >= open_for {
                        e.state = BreakerState::HalfOpen; e.half_open_probes = 1;
                        return true;
                    }
                }
                false
            }
            BreakerState::HalfOpen => {
                e.half_open_probes = e.half_open_probes.saturating_add(1);
                e.half_open_probes <= 3
            }
        }
    }
}

thread_local! { static BR: Breakers = Breakers::new(); }

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let _threshold = cfg_u64(cfg, "failure_threshold").unwrap_or(5) as u32;
    let open_for_ms = cfg_u64(cfg, "open_for_ms").unwrap_or(10_000);
    let key = ctx.route.id.to_string();
    let allow = BR.with(|b| b.try_request(&key, Duration::from_millis(open_for_ms)));
    if !allow { return Err(AGwError::CircuitOpen { service: ctx.route.name.clone() }); }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn initial_closed() { assert_eq!(Breakers::new().state("k"), BreakerState::Closed); }
    #[test] fn open_at_threshold() {
        let b = Breakers::new();
        assert_eq!(b.record_failure("k", 3), BreakerState::Closed);
        assert_eq!(b.record_failure("k", 3), BreakerState::Closed);
        assert_eq!(b.record_failure("k", 3), BreakerState::Open);
    }
    #[test] fn try_blocks_when_open() {
        let b = Breakers::new();
        for _ in 0..5 { b.record_failure("k", 3); }
        assert!(!b.try_request("k", Duration::from_secs(10)));
    }
    #[test] fn half_open_after_window() {
        let b = Breakers::new();
        for _ in 0..5 { b.record_failure("k", 3); }
        assert!(b.try_request("k", Duration::from_nanos(1)));
        assert_eq!(b.state("k"), BreakerState::HalfOpen);
    }
    #[test] fn success_resets() {
        let b = Breakers::new();
        for _ in 0..5 { b.record_failure("k", 3); }
        b.record_success("k");
        assert_eq!(b.state("k"), BreakerState::Closed);
    }
    #[test] fn half_open_caps_probes() {
        let b = Breakers::new();
        for _ in 0..3 { b.record_failure("k", 3); }
        assert!(b.try_request("k", Duration::from_nanos(1)));
        assert!(b.try_request("k", Duration::from_secs(60)));
        assert!(b.try_request("k", Duration::from_secs(60)));
        assert!(!b.try_request("k", Duration::from_secs(60)));
    }
}
