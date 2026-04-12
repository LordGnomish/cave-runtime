//! Gateway engine — load balancing, circuit breakers, and request routing.
//!
//! Load-balancing algorithms:
//!   - Round-robin (with weight support)
//!   - Consistent hashing (virtual-node ring, key = IP / header / cookie)
//!   - Least-connections
//!   - Latency-aware (weighted least-connections with latency bias)
//!
//! Circuit breaker per upstream target:
//!   Closed → Open (after N failures) → HalfOpen (after timeout) → Closed

use crate::models::{HashOn, LoadBalancingAlgorithm, Target, Upstream};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

// ─────────────────────────────────────────────
//  Load balancer state (per upstream)
// ─────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct LbState {
    /// Next index for round-robin
    pub rr_index: usize,
    /// Active connection count per target
    pub connections: HashMap<Uuid, u32>,
    /// Latency samples (ms) per target, bounded to last 100
    pub latency_samples: HashMap<Uuid, Vec<u64>>,
}

impl LbState {
    pub fn record_request_start(&mut self, target_id: Uuid) {
        *self.connections.entry(target_id).or_insert(0) += 1;
    }

    pub fn record_request_end(&mut self, target_id: Uuid, latency_ms: u64) {
        let conns = self.connections.entry(target_id).or_insert(0);
        if *conns > 0 {
            *conns -= 1;
        }
        let samples = self.latency_samples.entry(target_id).or_default();
        samples.push(latency_ms);
        if samples.len() > 100 {
            samples.remove(0);
        }
    }

    fn avg_latency(&self, target_id: Uuid) -> f64 {
        match self.latency_samples.get(&target_id) {
            None => 0.0,
            Some(s) if s.is_empty() => 0.0,
            Some(s) => s.iter().sum::<u64>() as f64 / s.len() as f64,
        }
    }
}

/// Select the next target from a list of healthy targets using the upstream's algorithm.
///
/// Returns the selected target index into `targets`, or `None` if empty.
pub fn select_target<'a>(
    upstream: &Upstream,
    targets: &'a [&'a Target],
    lb: &mut LbState,
    hash_key: Option<&str>,
) -> Option<&'a Target> {
    if targets.is_empty() {
        return None;
    }

    match upstream.algorithm {
        LoadBalancingAlgorithm::RoundRobin => select_round_robin(targets, lb),
        LoadBalancingAlgorithm::ConsistentHashing => {
            let key = resolve_hash_key(upstream, hash_key);
            select_consistent_hash(targets, &key)
        }
        LoadBalancingAlgorithm::LeastConnections => select_least_connections(targets, lb),
        LoadBalancingAlgorithm::LatencyAware => select_latency_aware(targets, lb),
    }
}

fn select_round_robin<'a>(targets: &'a [&'a Target], lb: &mut LbState) -> Option<&'a Target> {
    if targets.is_empty() {
        return None;
    }
    // Weighted round-robin: expand by weight, then index into that
    let total_weight: u32 = targets.iter().map(|t| t.weight.max(1)).sum();
    if total_weight == 0 {
        return None;
    }
    let idx = lb.rr_index % total_weight as usize;
    lb.rr_index = lb.rr_index.wrapping_add(1);

    let mut cumulative = 0u32;
    for target in targets {
        cumulative += target.weight.max(1);
        if idx < cumulative as usize {
            return Some(target);
        }
    }
    targets.last().copied()
}

fn select_consistent_hash<'a>(
    targets: &'a [&'a Target],
    key: &str,
) -> Option<&'a Target> {
    if targets.is_empty() {
        return None;
    }
    // FNV-1a 32-bit hash for speed (no external dep)
    let hash = fnv1a_32(key.as_bytes());
    // Map hash to virtual ring with 128 vnodes per target
    let total_vnodes = targets.len() as u32 * 128;
    let vnode = hash % total_vnodes;
    let idx = (vnode / 128) as usize % targets.len();
    Some(targets[idx])
}

fn fnv1a_32(data: &[u8]) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for &byte in data {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

fn select_least_connections<'a>(
    targets: &'a [&'a Target],
    lb: &LbState,
) -> Option<&'a Target> {
    targets
        .iter()
        .min_by_key(|t| lb.connections.get(&t.id).copied().unwrap_or(0))
        .copied()
}

fn select_latency_aware<'a>(targets: &'a [&'a Target], lb: &LbState) -> Option<&'a Target> {
    targets
        .iter()
        .min_by(|a, b| {
            let score_a = lb.avg_latency(a.id)
                + lb.connections.get(&a.id).copied().unwrap_or(0) as f64 * 10.0;
            let score_b = lb.avg_latency(b.id)
                + lb.connections.get(&b.id).copied().unwrap_or(0) as f64 * 10.0;
            score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}

fn resolve_hash_key(upstream: &Upstream, provided: Option<&str>) -> String {
    match (&upstream.hash_on, provided) {
        (HashOn::None, _) => "default".to_string(),
        (_, Some(k)) => k.to_string(),
        _ => "default".to_string(),
    }
}

// ─────────────────────────────────────────────
//  Circuit breaker
// ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Consecutive failures required to trip open
    pub failure_threshold: u32,
    /// How long to stay Open before trying HalfOpen
    pub timeout: Duration,
    /// Consecutive successes in HalfOpen before closing
    pub success_threshold: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            timeout: Duration::from_secs(30),
            success_threshold: 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    pub state: CircuitState,
    pub failure_count: u32,
    pub success_count: u32,
    pub last_state_change: Instant,
    pub config: CircuitBreakerConfig,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_state_change: Instant::now(),
            config,
        }
    }

    /// Can a request pass through?
    pub fn is_open(&self) -> bool {
        self.state == CircuitState::Open
    }

    /// Call before forwarding a request.
    /// Returns `true` if allowed, `false` if circuit is Open.
    pub fn allow_request(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => true, // probe request
            CircuitState::Open => {
                // Try transitioning to HalfOpen after timeout
                if self.last_state_change.elapsed() >= self.config.timeout {
                    self.transition_to(CircuitState::HalfOpen);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Record a successful upstream response.
    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::Closed => {
                self.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                self.success_count += 1;
                if self.success_count >= self.config.success_threshold {
                    self.transition_to(CircuitState::Closed);
                }
            }
            CircuitState::Open => {}
        }
    }

    /// Record a failed upstream response.
    pub fn record_failure(&mut self) {
        match self.state {
            CircuitState::Closed => {
                self.failure_count += 1;
                if self.failure_count >= self.config.failure_threshold {
                    self.transition_to(CircuitState::Open);
                }
            }
            CircuitState::HalfOpen => {
                // Single failure in HalfOpen → back to Open
                self.transition_to(CircuitState::Open);
            }
            CircuitState::Open => {}
        }
    }

    fn transition_to(&mut self, new_state: CircuitState) {
        self.state = new_state;
        self.failure_count = 0;
        self.success_count = 0;
        self.last_state_change = Instant::now();
    }
}

/// Manages circuit breakers for all upstream targets.
#[derive(Default)]
pub struct CircuitBreakerRegistry {
    breakers: HashMap<Uuid, CircuitBreaker>,
}

impl CircuitBreakerRegistry {
    pub fn get_or_create(&mut self, target_id: Uuid) -> &mut CircuitBreaker {
        self.breakers
            .entry(target_id)
            .or_insert_with(|| CircuitBreaker::new(CircuitBreakerConfig::default()))
    }

    pub fn allow(&mut self, target_id: Uuid) -> bool {
        self.get_or_create(target_id).allow_request()
    }

    pub fn success(&mut self, target_id: Uuid) {
        self.get_or_create(target_id).record_success();
    }

    pub fn failure(&mut self, target_id: Uuid) {
        self.get_or_create(target_id).record_failure();
    }

    pub fn state(&mut self, target_id: Uuid) -> &CircuitState {
        &self.get_or_create(target_id).state
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{HealthCheckConfig, HashFallback, HashOn, LoadBalancingAlgorithm, Target, TargetHealth, Upstream};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_upstream(algo: LoadBalancingAlgorithm) -> Upstream {
        Upstream {
            id: Uuid::new_v4(),
            name: "test-upstream".into(),
            algorithm: algo,
            hash_on: HashOn::Ip,
            hash_fallback: HashFallback::None,
            hash_on_header: None,
            healthchecks: HealthCheckConfig::default(),
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_target(upstream_id: Uuid, weight: u32) -> Target {
        Target {
            id: Uuid::new_v4(),
            upstream_id,
            target: "127.0.0.1:8080".into(),
            weight,
            health: TargetHealth::Healthy,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ── Load-balancing tests ───────────────────────────────────────────────

    #[test]
    fn test_round_robin_cycles() {
        let up = make_upstream(LoadBalancingAlgorithm::RoundRobin);
        let t0 = make_target(up.id, 1);
        let t1 = make_target(up.id, 1);
        let t2 = make_target(up.id, 1);
        let targets = vec![&t0, &t1, &t2];
        let mut lb = LbState::default();

        let ids: Vec<Uuid> = (0..6)
            .map(|_| select_target(&up, &targets, &mut lb, None).unwrap().id)
            .collect();

        // Should cycle through all three
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn test_consistent_hash_stability() {
        let up = make_upstream(LoadBalancingAlgorithm::ConsistentHashing);
        let t0 = make_target(up.id, 1);
        let t1 = make_target(up.id, 1);
        let t2 = make_target(up.id, 1);
        let targets = vec![&t0, &t1, &t2];
        let mut lb = LbState::default();

        let first = select_target(&up, &targets, &mut lb, Some("192.168.1.1"))
            .unwrap()
            .id;
        let second = select_target(&up, &targets, &mut lb, Some("192.168.1.1"))
            .unwrap()
            .id;

        // Same key must always go to the same target
        assert_eq!(first, second);
    }

    #[test]
    fn test_consistent_hash_different_keys() {
        let up = make_upstream(LoadBalancingAlgorithm::ConsistentHashing);
        let t0 = make_target(up.id, 1);
        let t1 = make_target(up.id, 1);
        let t2 = make_target(up.id, 1);
        let targets = vec![&t0, &t1, &t2];
        let mut lb = LbState::default();

        let keys = ["10.0.0.1", "10.0.0.2", "10.0.0.3", "172.16.0.1", "192.168.0.1"];
        let selected: Vec<Uuid> = keys
            .iter()
            .map(|k| select_target(&up, &targets, &mut lb, Some(k)).unwrap().id)
            .collect();

        // Different keys should distribute (at least 2 different targets across 5 keys with 3 targets)
        let unique: std::collections::HashSet<_> = selected.iter().collect();
        assert!(unique.len() >= 2, "Expected distribution across targets");
    }

    #[test]
    fn test_least_connections_routes_to_least_loaded() {
        let up = make_upstream(LoadBalancingAlgorithm::LeastConnections);
        let t0 = make_target(up.id, 1);
        let t1 = make_target(up.id, 1);
        let targets = vec![&t0, &t1];
        let mut lb = LbState::default();

        // Simulate t0 being busy
        lb.connections.insert(t0.id, 5);
        lb.connections.insert(t1.id, 0);

        let selected = select_target(&up, &targets, &mut lb, None).unwrap();
        assert_eq!(selected.id, t1.id);
    }

    // ── Circuit breaker tests ──────────────────────────────────────────────

    #[test]
    fn test_circuit_opens_on_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let mut cb = CircuitBreaker::new(config);

        assert_eq!(cb.state, CircuitState::Closed);
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state, CircuitState::Closed);
        cb.record_failure(); // 3rd failure → Open
        assert_eq!(cb.state, CircuitState::Open);
    }

    #[test]
    fn test_circuit_rejects_in_open_state() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            timeout: Duration::from_secs(9999),
            ..Default::default()
        };
        let mut cb = CircuitBreaker::new(config);
        cb.record_failure(); // → Open

        assert!(!cb.allow_request());
    }

    #[test]
    fn test_circuit_half_open_after_timeout() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            timeout: Duration::from_millis(1),
            ..Default::default()
        };
        let mut cb = CircuitBreaker::new(config);
        cb.record_failure(); // → Open

        // Sleep past the timeout
        std::thread::sleep(Duration::from_millis(10));

        assert!(cb.allow_request()); // should transition to HalfOpen
        assert_eq!(cb.state, CircuitState::HalfOpen);
    }

    #[test]
    fn test_circuit_closes_after_successes_in_half_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            timeout: Duration::from_millis(1),
            success_threshold: 2,
        };
        let mut cb = CircuitBreaker::new(config);
        cb.record_failure(); // → Open
        std::thread::sleep(Duration::from_millis(10));
        cb.allow_request(); // → HalfOpen

        cb.record_success();
        assert_eq!(cb.state, CircuitState::HalfOpen);
        cb.record_success(); // 2nd success → Closed
        assert_eq!(cb.state, CircuitState::Closed);
    }

    #[test]
    fn test_circuit_reopens_on_failure_in_half_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            timeout: Duration::from_millis(1),
            success_threshold: 5,
        };
        let mut cb = CircuitBreaker::new(config);
        cb.record_failure(); // → Open
        std::thread::sleep(Duration::from_millis(10));
        cb.allow_request(); // → HalfOpen

        cb.record_failure(); // → Open again
        assert_eq!(cb.state, CircuitState::Open);
    }

    #[test]
    fn test_lb_connection_tracking() {
        let mut lb = LbState::default();
        let id = Uuid::new_v4();

        lb.record_request_start(id);
        lb.record_request_start(id);
        assert_eq!(*lb.connections.get(&id).unwrap(), 2);

        lb.record_request_end(id, 50);
        assert_eq!(*lb.connections.get(&id).unwrap(), 1);
    }
}
