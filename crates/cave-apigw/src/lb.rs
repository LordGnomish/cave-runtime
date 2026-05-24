// SPDX-License-Identifier: AGPL-3.0-or-later
//! Load-balancing algorithms — round-robin, least-connections, consistent-hash,
//! EWMA, weighted random.
//!
//! Kong's ring-balancer (`kong.runloop.balancer`) + Envoy `Cluster.lb_policy`
//! references inform the design; this Rust port keeps per-target state in a
//! small struct guarded by `RwLock`.

use crate::models::{HashOn, Target, Upstream, UpstreamAlgorithm};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Per-target live counters maintained by the LB and the health checker.
#[derive(Debug, Clone)]
pub struct TargetState {
    pub healthy: bool,
    pub active_requests: u32,
    pub ewma_latency_ms: f64,
    pub consecutive_failures: u32,
    pub last_seen: Instant,
}

impl Default for TargetState {
    fn default() -> Self {
        Self {
            healthy: true,
            active_requests: 0,
            ewma_latency_ms: 0.0,
            consecutive_failures: 0,
            last_seen: Instant::now(),
        }
    }
}

/// Hint passed into `pick()` to drive consistent-hash + ewma deterministically.
#[derive(Debug, Clone, Default)]
pub struct PickHint {
    pub source_ip: Option<String>,
    pub consumer_id: Option<Uuid>,
    pub header: Option<String>,
    pub cookie: Option<String>,
    pub path: Option<String>,
}

/// Outcome of one pick.
#[derive(Debug, Clone)]
pub struct PickedTarget {
    pub target: Target,
    pub algorithm: UpstreamAlgorithm,
}

pub struct LbState {
    rr_cursor: RwLock<HashMap<Uuid, usize>>,
    targets: RwLock<HashMap<Uuid, TargetState>>,
}

impl Default for LbState {
    fn default() -> Self {
        Self {
            rr_cursor: RwLock::new(HashMap::new()),
            targets: RwLock::new(HashMap::new()),
        }
    }
}

impl LbState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark_healthy(&self, target_id: Uuid, healthy: bool) {
        let mut g = self.targets.write().unwrap();
        let entry = g.entry(target_id).or_default();
        entry.healthy = healthy;
        if healthy {
            entry.consecutive_failures = 0;
        }
        entry.last_seen = Instant::now();
    }

    pub fn record_outcome(&self, target_id: Uuid, success: bool, latency: Duration) {
        let mut g = self.targets.write().unwrap();
        let entry = g.entry(target_id).or_default();
        entry.last_seen = Instant::now();
        let sample = latency.as_secs_f64() * 1000.0;
        // EWMA with α = 0.3 — Envoy/Maglev default range.
        entry.ewma_latency_ms = if entry.ewma_latency_ms == 0.0 {
            sample
        } else {
            0.3 * sample + 0.7 * entry.ewma_latency_ms
        };
        if success {
            entry.consecutive_failures = 0;
            entry.healthy = true;
        } else {
            entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
        }
    }

    pub fn inc_active(&self, target_id: Uuid) {
        let mut g = self.targets.write().unwrap();
        let entry = g.entry(target_id).or_default();
        entry.active_requests = entry.active_requests.saturating_add(1);
    }

    pub fn dec_active(&self, target_id: Uuid) {
        let mut g = self.targets.write().unwrap();
        let entry = g.entry(target_id).or_default();
        entry.active_requests = entry.active_requests.saturating_sub(1);
    }

    pub fn target_state(&self, target_id: Uuid) -> TargetState {
        self.targets
            .read()
            .unwrap()
            .get(&target_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Pick a target from the upstream pool according to its algorithm.
    pub fn pick(&self, upstream: &Upstream, hint: &PickHint) -> Option<PickedTarget> {
        let healthy: Vec<&Target> = upstream
            .targets
            .iter()
            .filter(|t| {
                let st = self.targets.read().unwrap();
                st.get(&t.id).map(|s| s.healthy).unwrap_or(true)
            })
            .collect();
        if healthy.is_empty() {
            return None;
        }
        let target = match upstream.algorithm {
            UpstreamAlgorithm::RoundRobin => self.pick_round_robin(upstream, &healthy),
            UpstreamAlgorithm::LeastConnections => self.pick_least_connections(&healthy),
            UpstreamAlgorithm::ConsistentHashing => self.pick_consistent_hash(upstream, &healthy, hint),
            UpstreamAlgorithm::Ewma => self.pick_ewma(&healthy),
            UpstreamAlgorithm::Random => self.pick_random(&healthy),
        };
        target.cloned().map(|t| PickedTarget {
            target: t,
            algorithm: upstream.algorithm,
        })
    }

    fn pick_round_robin<'a>(&self, upstream: &Upstream, healthy: &'a [&Target]) -> Option<&'a Target> {
        let total_weight: u32 = healthy.iter().map(|t| t.weight.max(1)).sum();
        if total_weight == 0 {
            return healthy.first().copied();
        }
        let mut cur = self.rr_cursor.write().unwrap();
        let pos = cur.entry(upstream.id).or_insert(0);
        let mut step = *pos % (total_weight as usize);
        for t in healthy {
            let w = t.weight.max(1) as usize;
            if step < w {
                *pos = pos.wrapping_add(1);
                return Some(*t);
            }
            step -= w;
        }
        healthy.first().copied()
    }

    fn pick_least_connections<'a>(&self, healthy: &'a [&Target]) -> Option<&'a Target> {
        let st = self.targets.read().unwrap();
        healthy
            .iter()
            .min_by_key(|t| {
                let active = st.get(&t.id).map(|s| s.active_requests).unwrap_or(0);
                // weight-aware: lower active/weight is preferred
                (active * 1000) / t.weight.max(1)
            })
            .copied()
    }

    fn pick_consistent_hash<'a>(
        &self,
        upstream: &Upstream,
        healthy: &'a [&Target],
        hint: &PickHint,
    ) -> Option<&'a Target> {
        let key = hash_key(&upstream.hash_on, hint).unwrap_or_else(|| {
            // fallback chain
            hash_key(&upstream.hash_fallback, hint).unwrap_or_default()
        });
        if key.is_empty() {
            return healthy.first().copied();
        }
        let mut best: Option<(u64, &Target)> = None;
        for t in healthy {
            let mut h = Sha256::new();
            h.update(key.as_bytes());
            h.update(t.id.as_bytes());
            let d = h.finalize();
            let n = u64::from_be_bytes(d[..8].try_into().unwrap());
            match best {
                None => best = Some((n, *t)),
                Some((cur, _)) if n < cur => best = Some((n, *t)),
                _ => {}
            }
        }
        best.map(|(_, t)| t)
    }

    fn pick_ewma<'a>(&self, healthy: &'a [&Target]) -> Option<&'a Target> {
        let st = self.targets.read().unwrap();
        healthy
            .iter()
            .min_by(|a, b| {
                let la = st.get(&a.id).map(|s| s.ewma_latency_ms).unwrap_or(0.0);
                let lb = st.get(&b.id).map(|s| s.ewma_latency_ms).unwrap_or(0.0);
                la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
    }

    fn pick_random<'a>(&self, healthy: &'a [&Target]) -> Option<&'a Target> {
        let now = Instant::now().elapsed().subsec_nanos() as usize;
        let idx = now % healthy.len();
        Some(healthy[idx])
    }
}

fn hash_key(hash_on: &HashOn, hint: &PickHint) -> Option<String> {
    match hash_on {
        HashOn::None => None,
        HashOn::Consumer => hint.consumer_id.map(|u| u.to_string()),
        HashOn::Ip => hint.source_ip.clone(),
        HashOn::Header(_) => hint.header.clone(),
        HashOn::Cookie(_) => hint.cookie.clone(),
        HashOn::Path => hint.path.clone(),
        HashOn::QueryArg(_) => hint.path.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{HashOn, Target, Upstream, UpstreamAlgorithm};

    fn upstream_with_targets(algo: UpstreamAlgorithm) -> Upstream {
        let mut u = Upstream::new("api");
        u.algorithm = algo;
        u.targets = vec![
            Target::new("a", 80, 1),
            Target::new("b", 80, 1),
            Target::new("c", 80, 1),
        ];
        u
    }

    #[test]
    fn rr_cycles_through_targets() {
        let lb = LbState::new();
        let u = upstream_with_targets(UpstreamAlgorithm::RoundRobin);
        let hint = PickHint::default();
        let a = lb.pick(&u, &hint).unwrap().target.host;
        let b = lb.pick(&u, &hint).unwrap().target.host;
        let c = lb.pick(&u, &hint).unwrap().target.host;
        let d = lb.pick(&u, &hint).unwrap().target.host;
        assert_eq!(a, "a");
        assert_eq!(b, "b");
        assert_eq!(c, "c");
        assert_eq!(d, "a"); // wraps
    }

    #[test]
    fn rr_respects_weights() {
        let lb = LbState::new();
        let mut u = Upstream::new("api");
        u.algorithm = UpstreamAlgorithm::RoundRobin;
        u.targets = vec![Target::new("a", 80, 3), Target::new("b", 80, 1)];
        let hint = PickHint::default();
        let mut a_count = 0;
        let mut b_count = 0;
        for _ in 0..8 {
            let t = lb.pick(&u, &hint).unwrap();
            if t.target.host == "a" { a_count += 1; } else { b_count += 1; }
        }
        assert!(a_count > b_count);
    }

    #[test]
    fn lc_prefers_least_active() {
        let lb = LbState::new();
        let u = upstream_with_targets(UpstreamAlgorithm::LeastConnections);
        for _ in 0..3 {
            lb.inc_active(u.targets[0].id);
        }
        lb.inc_active(u.targets[1].id);
        let hint = PickHint::default();
        let pick = lb.pick(&u, &hint).unwrap();
        assert_eq!(pick.target.host, "c"); // zero active
    }

    #[test]
    fn ewma_prefers_lowest_latency() {
        let lb = LbState::new();
        let u = upstream_with_targets(UpstreamAlgorithm::Ewma);
        lb.record_outcome(u.targets[0].id, true, Duration::from_millis(200));
        lb.record_outcome(u.targets[1].id, true, Duration::from_millis(50));
        lb.record_outcome(u.targets[2].id, true, Duration::from_millis(300));
        let pick = lb.pick(&u, &PickHint::default()).unwrap();
        assert_eq!(pick.target.host, "b");
    }

    #[test]
    fn ch_is_sticky_for_same_key() {
        let lb = LbState::new();
        let mut u = upstream_with_targets(UpstreamAlgorithm::ConsistentHashing);
        u.hash_on = HashOn::Ip;
        let hint = PickHint {
            source_ip: Some("1.2.3.4".into()),
            ..Default::default()
        };
        let p1 = lb.pick(&u, &hint).unwrap().target.host.clone();
        for _ in 0..10 {
            let p = lb.pick(&u, &hint).unwrap().target.host;
            assert_eq!(p, p1);
        }
    }

    #[test]
    fn unhealthy_targets_excluded() {
        let lb = LbState::new();
        let u = upstream_with_targets(UpstreamAlgorithm::RoundRobin);
        lb.mark_healthy(u.targets[0].id, false);
        lb.mark_healthy(u.targets[1].id, false);
        let pick = lb.pick(&u, &PickHint::default()).unwrap();
        assert_eq!(pick.target.host, "c");
    }

    #[test]
    fn all_unhealthy_returns_none() {
        let lb = LbState::new();
        let u = upstream_with_targets(UpstreamAlgorithm::RoundRobin);
        for t in &u.targets {
            lb.mark_healthy(t.id, false);
        }
        assert!(lb.pick(&u, &PickHint::default()).is_none());
    }

    #[test]
    fn record_outcome_updates_ewma() {
        let lb = LbState::new();
        let t = Target::new("a", 80, 1);
        lb.record_outcome(t.id, true, Duration::from_millis(100));
        let st = lb.target_state(t.id);
        assert!(st.ewma_latency_ms > 0.0);
    }
}
