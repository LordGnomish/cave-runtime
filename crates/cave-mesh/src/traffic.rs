<<<<<<< HEAD
//! Traffic management engine.
//!
//! Implements route resolution (VirtualService match → destination),
//! weighted traffic splitting, fault injection, retries, timeouts,
//! and W3C Trace Context propagation.

use crate::models::{
    DestinationRule, FaultEffect, HttpFaultInjection, HttpMatchRequest, HttpRoute,
    HttpRouteDestination, IncomingRequest, LoadBalancerMode, RouteDecision,
    VirtualService,
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::debug;
use uuid::Uuid;

/// Central traffic management store + resolver.
#[derive(Debug, Clone)]
pub struct TrafficManager {
    virtual_services: Arc<RwLock<HashMap<String, VirtualService>>>,
    destination_rules: Arc<RwLock<HashMap<String, DestinationRule>>>,
    /// Round-robin cursor per (host, subset) key
    rr_cursor: Arc<RwLock<HashMap<String, usize>>>,
}

impl Default for TrafficManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TrafficManager {
    pub fn new() -> Self {
        Self {
            virtual_services: Arc::new(RwLock::new(HashMap::new())),
            destination_rules: Arc::new(RwLock::new(HashMap::new())),
            rr_cursor: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ─── VirtualService CRUD ─────────────────────────────────

    pub fn upsert_virtual_service(&self, vs: VirtualService) {
        let mut map = self.virtual_services.write().unwrap();
        for host in &vs.hosts {
            map.insert(host.clone(), vs.clone());
        }
    }

    pub fn remove_virtual_service(&self, name: &str) {
        let mut map = self.virtual_services.write().unwrap();
        map.retain(|_, v| v.name != name);
    }

    pub fn list_virtual_services(&self) -> Vec<VirtualService> {
        let map = self.virtual_services.read().unwrap();
        // Deduplicate — VS can be stored under multiple host keys
        let mut seen = std::collections::HashSet::new();
        map.values()
            .filter(|v| seen.insert(v.name.clone()))
            .cloned()
            .collect()
    }

    pub fn get_virtual_service(&self, host: &str) -> Option<VirtualService> {
        let map = self.virtual_services.read().unwrap();
        map.get(host).cloned()
    }

    // ─── DestinationRule CRUD ────────────────────────────────

    pub fn upsert_destination_rule(&self, dr: DestinationRule) {
        let mut map = self.destination_rules.write().unwrap();
        map.insert(dr.host.clone(), dr);
    }

    pub fn remove_destination_rule(&self, host: &str) {
        let mut map = self.destination_rules.write().unwrap();
        map.remove(host);
    }

    pub fn list_destination_rules(&self) -> Vec<DestinationRule> {
        let map = self.destination_rules.read().unwrap();
        map.values().cloned().collect()
    }

    pub fn get_destination_rule(&self, host: &str) -> Option<DestinationRule> {
        let map = self.destination_rules.read().unwrap();
        map.get(host).cloned()
    }

    // ─── Route Resolution ────────────────────────────────────

    /// Resolve the routing decision for an incoming request aimed at `host`.
    ///
    /// Returns `None` when no VirtualService covers the host (fall-through to
    /// default routing).
    pub fn resolve_route(&self, host: &str, req: &IncomingRequest) -> Option<RouteDecision> {
        let vs = self.get_virtual_service(host)?;

        // Walk HTTP routes in order; first match wins.
        for route in &vs.http {
            if self.request_matches(req, &route.match_rules) {
                return Some(self.build_decision(route, req));
            }
        }
        None
    }

    // ─── Internal helpers ────────────────────────────────────

    /// Returns true if the request satisfies ALL match rules in the list.
    /// An empty list matches everything.
    fn request_matches(&self, req: &IncomingRequest, rules: &[HttpMatchRequest]) -> bool {
        if rules.is_empty() {
            return true;
        }
        // Any one rule matches (OR between rule entries, AND within a rule)
        rules.iter().any(|rule| self.single_rule_matches(req, rule))
    }

    fn single_rule_matches(&self, req: &IncomingRequest, rule: &HttpMatchRequest) -> bool {
        // URI
        if let Some(uri_match) = &rule.uri {
            if !uri_match.matches(&req.uri) {
                return false;
            }
        }
        // Method
        if let Some(method_match) = &rule.method {
            if !method_match.matches(&req.method) {
                return false;
            }
        }
        // Authority
        if let Some(auth_match) = &rule.authority {
            let authority = req.authority.as_deref().unwrap_or("");
            if !auth_match.matches(authority) {
                return false;
            }
        }
        // Headers — ALL specified headers must match
        for (header_name, header_match) in &rule.headers {
            let value = req
                .headers
                .get(header_name)
                .map(|s| s.as_str())
                .unwrap_or("");
            if !header_match.matches(value) {
                return false;
            }
        }
        // Query params — ALL specified params must match
        for (param, param_match) in &rule.query_params {
            let value = req.query_params.get(param).map(|s| s.as_str()).unwrap_or("");
            if !param_match.matches(value) {
                return false;
            }
        }
        // Source labels — request source pod must carry ALL specified labels
        for (k, v) in &rule.source_labels {
            if req.source_labels.get(k).map(|s| s.as_str()) != Some(v.as_str()) {
                return false;
            }
        }
        true
    }

    fn build_decision(&self, route: &HttpRoute, req: &IncomingRequest) -> RouteDecision {
        let dest = self.pick_destination(&route.route);

        // Determine fault effect (if any)
        let fault = route.fault.as_ref().and_then(|f| self.evaluate_fault(f));

        // Trace context propagation (W3C Trace Context)
        let traceparent = Some(propagate_traceparent(req.traceparent.as_deref()));

        // Header additions from route-level header manipulation
        let mut request_headers_add = HashMap::new();
        let mut response_headers_add = HashMap::new();
        if let Some(hops) = &route.headers {
            if let Some(req_h) = &hops.request {
                request_headers_add.extend(req_h.set.clone());
                request_headers_add.extend(req_h.add.clone());
            }
            if let Some(res_h) = &hops.response {
                response_headers_add.extend(res_h.set.clone());
                response_headers_add.extend(res_h.add.clone());
            }
        }
        // Also inject traceparent as a request header
        if let Some(tp) = &traceparent {
            request_headers_add.insert("traceparent".to_string(), tp.clone());
        }

        debug!(
            dest = %dest.destination.host,
            subset = ?dest.destination.subset,
            "Route decision"
        );

        RouteDecision {
            destination_host: dest.destination.host.clone(),
            destination_subset: dest.destination.subset.clone(),
            destination_port: dest.destination.port.as_ref().map(|p| p.number),
            weight: dest.weight.unwrap_or(100),
            fault,
            retry: route.retries.clone(),
            timeout_ms: route.timeout_ms,
            request_headers_add,
            response_headers_add,
            traceparent,
        }
    }

    /// Pick a destination from the weighted list.
    /// Uses UUID random bytes for weighted random selection.
    fn pick_destination<'a>(&self, dests: &'a [HttpRouteDestination]) -> &'a HttpRouteDestination {
        if dests.is_empty() {
            panic!("route has no destinations");
        }
        if dests.len() == 1 {
            return &dests[0];
        }

        let total: u32 = dests.iter().map(|d| d.weight.unwrap_or(100)).sum();
        if total == 0 {
            return &dests[0];
        }

        // Use UUID v4 random bytes as a lightweight PRNG
        let rand_val = (Uuid::new_v4().as_u128() % total as u128) as u32;
        let mut cursor = 0u32;
        for dest in dests {
            cursor += dest.weight.unwrap_or(100);
            if rand_val < cursor {
                return dest;
            }
        }
        dests.last().unwrap()
    }

    fn evaluate_fault(&self, fault: &HttpFaultInjection) -> Option<FaultEffect> {
        // Use UUID random to determine if fault applies
        let pct = (Uuid::new_v4().as_u128() % 100) as f64;

        // Abort takes priority over delay if both are configured
        if let Some(abort) = &fault.abort {
            if pct < abort.percent {
                return Some(FaultEffect::Abort(abort.http_status));
            }
        }
        if let Some(delay) = &fault.delay {
            if pct < delay.percent {
                return Some(FaultEffect::Delay(delay.fixed_delay_ms));
            }
        }
        None
    }

    /// Select an endpoint using the configured load-balancing algorithm.
    pub fn select_endpoint_index(
        &self,
        host: &str,
        subset: Option<&str>,
        endpoint_count: usize,
    ) -> usize {
        if endpoint_count == 0 {
            return 0;
        }
        let dr = self.get_destination_rule(host);
        let mode = dr
            .as_ref()
            .and_then(|d| {
                // Check subset-level policy first
                let subset_policy = subset.and_then(|s| {
                    d.subsets
                        .iter()
                        .find(|sub| sub.name == s)
                        .and_then(|sub| sub.traffic_policy.as_ref())
                        .and_then(|tp| tp.load_balancer.as_ref())
                        .map(|lb| &lb.mode)
                });
                // Fall back to top-level policy
                subset_policy.or_else(|| {
                    d.traffic_policy
                        .as_ref()
                        .and_then(|tp| tp.load_balancer.as_ref())
                        .map(|lb| &lb.mode)
                })
            })
            .unwrap_or(&LoadBalancerMode::RoundRobin);

        match mode {
            LoadBalancerMode::RoundRobin => {
                let key = format!("{host}/{}", subset.unwrap_or(""));
                let mut cursors = self.rr_cursor.write().unwrap();
                let idx = cursors.entry(key).or_insert(0);
                let chosen = *idx % endpoint_count;
                *idx = chosen + 1;
                chosen
            }
            LoadBalancerMode::Random | LoadBalancerMode::Passthrough => {
                (Uuid::new_v4().as_u128() % endpoint_count as u128) as usize
            }
            LoadBalancerMode::LeastConn => {
                // Without live connection counters, fall back to random
                (Uuid::new_v4().as_u128() % endpoint_count as u128) as usize
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// W3C Trace Context helpers
// ─────────────────────────────────────────────────────────────

/// Propagate an existing traceparent or generate a new one.
/// Format: 00-<trace-id>-<parent-id>-<flags>
pub fn propagate_traceparent(incoming: Option<&str>) -> String {
    if let Some(tp) = incoming {
        // Preserve trace-id, generate new parent-id
        let parts: Vec<&str> = tp.splitn(4, '-').collect();
        if parts.len() == 4 {
            let trace_id = parts[1];
            let new_parent_id = &Uuid::new_v4().simple().to_string()[..16];
            return format!("00-{trace_id}-{new_parent_id}-01");
        }
    }
    // Generate a fresh traceparent
    let trace_id = Uuid::new_v4().simple().to_string();
    let parent_id = &Uuid::new_v4().simple().to_string()[..16];
    format!("00-{trace_id}-{parent_id}-01")
=======
//! Traffic management — splitting, canary routing, header-based routing,
//! fault injection, traffic mirroring.

use crate::models::{FaultInjection, HttpRoute, VirtualService, WeightedDestination};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Traffic Split ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficSplitResult {
    pub destination: String,
    pub weight_percent: f64,
}

/// Compute the traffic split percentages across all destinations in a VirtualService.
pub fn traffic_split(virtual_service: &VirtualService) -> Vec<TrafficSplitResult> {
    virtual_service
        .http_routes
        .iter()
        .flat_map(|route| {
            let total: u32 = route.destinations.iter().map(|d| d.weight).sum();
            route.destinations.iter().map(move |dest| {
                let pct = if total > 0 {
                    (dest.weight as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                let label = match &dest.subset {
                    Some(s) => format!("{}:{s}", dest.host),
                    None => dest.host.clone(),
                };
                TrafficSplitResult {
                    destination: label,
                    weight_percent: pct,
                }
            })
        })
        .collect()
}

// ─── Canary Routing ───────────────────────────────────────────────────────────

/// Build weighted destinations for a canary deployment.
/// `canary_weight` is 0–100; the remainder goes to the stable subset.
pub fn canary_routing(
    stable_host: &str,
    canary_host: &str,
    canary_weight: u32,
) -> Vec<WeightedDestination> {
    let canary_weight = canary_weight.min(100);
    let stable_weight = 100 - canary_weight;
    vec![
        WeightedDestination {
            host: stable_host.to_string(),
            subset: Some("stable".to_string()),
            port: None,
            weight: stable_weight,
        },
        WeightedDestination {
            host: canary_host.to_string(),
            subset: Some("canary".to_string()),
            port: None,
            weight: canary_weight,
        },
    ]
}

// ─── Header-Based Routing ─────────────────────────────────────────────────────

/// Return the first route whose match rules are satisfied by the provided headers.
pub fn header_based_routing<'a>(
    request_headers: &HashMap<String, String>,
    routes: &'a [HttpRoute],
) -> Option<&'a HttpRoute> {
    routes.iter().find(|route| {
        route.match_rules.iter().any(|rule| {
            if rule.headers.is_empty() {
                return false;
            }
            rule.headers.iter().all(|(k, matcher)| {
                request_headers
                    .get(k)
                    .map(|v| crate::proxy::string_matches(matcher, v))
                    .unwrap_or(false)
            })
        })
    })
}

// ─── Fault Injection ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultInjectionResult {
    pub applied: bool,
    pub fault_type: Option<FaultType>,
    pub delay_ms: Option<u64>,
    pub abort_status: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultType {
    Delay,
    Abort,
}

/// Evaluate a FaultInjection config and return which fault (if any) to apply.
/// Percentage ≥ 50.0 is treated as "would trigger" for deterministic evaluation.
pub fn fault_injection(fault: &FaultInjection) -> FaultInjectionResult {
    if let Some(abort) = &fault.abort {
        if abort.percentage >= 50.0 {
            return FaultInjectionResult {
                applied: true,
                fault_type: Some(FaultType::Abort),
                delay_ms: None,
                abort_status: Some(abort.http_status),
            };
        }
    }
    if let Some(delay) = &fault.delay {
        if delay.percentage >= 50.0 {
            return FaultInjectionResult {
                applied: true,
                fault_type: Some(FaultType::Delay),
                delay_ms: Some(delay.fixed_delay_ms),
                abort_status: None,
            };
        }
    }
    FaultInjectionResult {
        applied: false,
        fault_type: None,
        delay_ms: None,
        abort_status: None,
    }
}

// ─── Traffic Mirroring ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorResult {
    pub mirrored: bool,
    pub target_host: String,
    pub target_port: Option<u16>,
    /// Percentage of traffic to mirror (0.0–100.0)
    pub percentage: f64,
}

/// Evaluate whether a route has a mirror config and return it.
pub fn mirror_traffic(route: &HttpRoute) -> Option<MirrorResult> {
    let mirror = route.mirror.as_ref()?;
    Some(MirrorResult {
        mirrored: mirror.percentage > 0.0,
        target_host: mirror.host.clone(),
        target_port: mirror.port,
        percentage: mirror.percentage,
    })
>>>>>>> claude/peaceful-lederberg
}
