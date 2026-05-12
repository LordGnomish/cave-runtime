//! Proxy logic — request routing, load balancing, circuit breaking, retries, timeouts.

use crate::models::{HealthStatus, HttpRoute, LoadBalancerAlgorithm, ServiceInstance, StringMatch};
use crate::MeshState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use uuid::Uuid;

// ─── Circuit Breaker ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerState {
    pub service_id: Uuid,
    pub state: CircuitState,
    pub failure_count: u32,
    pub success_count: u32,
    pub last_failure: Option<DateTime<Utc>>,
    pub opened_at: Option<DateTime<Utc>>,
    /// When the circuit may allow a probe request through
    pub next_probe_at: Option<DateTime<Utc>>,
}

impl CircuitBreakerState {
    pub fn new(service_id: Uuid) -> Self {
        Self {
            service_id,
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure: None,
            opened_at: None,
            next_probe_at: None,
        }
    }
}

// ─── Routing Decision ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub service_id: Uuid,
    pub instance_address: String,
    pub instance_port: u16,
    pub subset: Option<String>,
    pub route_name: Option<String>,
}

/// Route an incoming request to a destination based on VirtualService rules.
/// Returns None if no matching VirtualService or no healthy instance is found.
pub fn route_request(
    host: &str,
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
    state: &MeshState,
) -> Option<RouteDecision> {
    let virtual_services = state.virtual_services.lock().unwrap();
    let services = state.services.lock().unwrap();
    let instances = state.instances.lock().unwrap();

    // Find VirtualService whose hosts list includes this host (or a wildcard)
    let vs = virtual_services
        .values()
        .find(|vs| vs.hosts.iter().any(|h| h == host || h == "*"))?;

    for route in &vs.http_routes {
        if route_matches(route, method, path, headers) {
            let dest = weighted_pick(&route.destinations)?;
            let svc = services.values().find(|s| s.name == dest.host)?;
            let available: Vec<_> = instances
                .values()
                .filter(|i| i.service_id == svc.id && i.health == HealthStatus::Healthy)
                .collect();
            let instance = pick_instance(&available, &LoadBalancerAlgorithm::RoundRobin)?;
            return Some(RouteDecision {
                service_id: svc.id,
                instance_address: instance.address.clone(),
                instance_port: instance.port,
                subset: dest.subset.clone(),
                route_name: Some(route.name.clone()),
            });
        }
    }
    None
}

/// All conditions within a RouteMatch must match (AND); any RouteMatch can satisfy the route (OR).
fn route_matches(
    route: &HttpRoute,
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
) -> bool {
    if route.match_rules.is_empty() {
        return true;
    }
    route.match_rules.iter().any(|rule| {
        let method_ok = rule
            .method
            .as_deref()
            .map(|m| m.eq_ignore_ascii_case(method))
            .unwrap_or(true);
        let uri_ok = rule
            .uri
            .as_ref()
            .map(|u| u.matches(path))
            .unwrap_or(true);
        let headers_ok = rule.headers.iter().all(|(k, matcher)| {
            headers
                .get(k)
                .map(|v| matcher.matches(v))
                .unwrap_or(false)
        });
        method_ok && uri_ok && headers_ok
    })
}

/// Pick from weighted destinations deterministically (highest weight wins).
fn weighted_pick(
    destinations: &[crate::models::WeightedDestination],
) -> Option<&crate::models::WeightedDestination> {
    destinations.iter().max_by_key(|d| d.weight)
}

/// Load balance among healthy instances using the given algorithm.
pub fn load_balance<'a>(
    instances: &'a [ServiceInstance],
    algorithm: &LoadBalancerAlgorithm,
) -> Option<&'a ServiceInstance> {
    let healthy: Vec<_> = instances
        .iter()
        .filter(|i| i.health == HealthStatus::Healthy)
        .collect();
    pick_instance(&healthy, algorithm)
}

fn pick_instance<'a>(
    instances: &[&'a ServiceInstance],
    _algorithm: &LoadBalancerAlgorithm,
) -> Option<&'a ServiceInstance> {
    // Deterministic: pick the highest-weight healthy instance.
    // A real implementation would use per-connection counters for round-robin.
    instances.iter().max_by_key(|i| i.weight).copied()
}

// ─── Circuit Breaker ──────────────────────────────────────────────────────────

/// Returns true if the request should be allowed through.
/// Transitions Open→HalfOpen when the probe window has elapsed.
pub fn circuit_breaker(service_id: Uuid, state: &MeshState) -> bool {
    let mut breakers = state.circuit_breakers.lock().unwrap();
    let cb = breakers
        .entry(service_id)
        .or_insert_with(|| CircuitBreakerState::new(service_id));

    match cb.state {
        CircuitState::Closed | CircuitState::HalfOpen => true,
        CircuitState::Open => {
            if let Some(probe_at) = cb.next_probe_at {
                if Utc::now() >= probe_at {
                    cb.state = CircuitState::HalfOpen;
                    tracing::info!(service_id = %service_id, "circuit breaker: Open → HalfOpen");
                    return true;
                }
            }
            false
        }
    }
}

/// Record a request outcome and update circuit breaker state.
pub fn record_outcome(service_id: Uuid, success: bool, threshold: u32, state: &MeshState) {
    let mut breakers = state.circuit_breakers.lock().unwrap();
    let cb = breakers
        .entry(service_id)
        .or_insert_with(|| CircuitBreakerState::new(service_id));

    if success {
        cb.success_count += 1;
        cb.failure_count = 0;
        if cb.state == CircuitState::HalfOpen {
            cb.state = CircuitState::Closed;
            cb.opened_at = None;
            cb.next_probe_at = None;
            tracing::info!(service_id = %service_id, "circuit breaker: HalfOpen → Closed");
        }
    } else {
        cb.failure_count += 1;
        cb.last_failure = Some(Utc::now());
        if cb.failure_count >= threshold && cb.state == CircuitState::Closed {
            cb.state = CircuitState::Open;
            cb.opened_at = Some(Utc::now());
            cb.next_probe_at = Some(Utc::now() + chrono::Duration::seconds(30));
            tracing::warn!(
                service_id = %service_id,
                failures = cb.failure_count,
                "circuit breaker: Closed → Open"
            );
        }
    }
}

// ─── Retry ───────────────────────────────────────────────────────────────────

/// Compute exponential backoff delay schedule for `attempts` retries.
///
/// Sweep-007: delegates to `cave_kernel::retrypolicy::RetryPolicy` so the
/// kernel's jitter-aware exponential strategy is the single source of
/// truth across the workspace. The local helper preserves the existing
/// signature (`attempts/base_ms/max_ms`) and the no-jitter semantics
/// that mesh xDS retries rely on.
pub fn retry_with_backoff(attempts: u32, base_ms: u64, max_ms: u64) -> Vec<Duration> {
    use cave_kernel::retrypolicy::{BackoffStrategy, RetryPolicy};
    use rand::SeedableRng;
    if attempts == 0 {
        return Vec::new();
    }
    let strategy = BackoffStrategy::Exponential {
        base: Duration::from_millis(base_ms),
        cap: Duration::from_millis(max_ms),
    };
    // `RetryPolicy::schedule` emits `max_attempts - 1` delays, so request
    // one extra attempt to keep the caller-facing count consistent.
    let policy = RetryPolicy::new(attempts + 1, strategy);
    // The non-jitter `Exponential` strategy doesn't consume the rng; use
    // a deterministic seed so the schedule stays reproducible.
    let mut rng = rand::rngs::StdRng::seed_from_u64(0);
    policy.schedule(&mut rng)
}

// ─── Timeout ─────────────────────────────────────────────────────────────────

/// Returns true if the request has exceeded its timeout budget.
pub fn timeout_handler(started_at: DateTime<Utc>, timeout_ms: u64) -> bool {
    let elapsed = Utc::now()
        .signed_duration_since(started_at)
        .num_milliseconds();
    elapsed >= 0 && elapsed as u64 >= timeout_ms
}

// ─── String matching (re-exported for traffic.rs) ─────────────────────────────

/// Public wrapper used by traffic.rs for header-based routing.
pub fn string_matches(matcher: &StringMatch, value: &str) -> bool {
    matcher.matches(value)
}
