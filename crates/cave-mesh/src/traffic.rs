// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Traffic management engine.
//!
//! Implements:
//!   • VirtualService route resolution (match → destination)
//!   • Weighted traffic splitting
//!   • HTTP redirect / rewrite
//!   • Fault injection (delays + aborts)
//!   • Retries and timeouts
//!   • Traffic mirroring (shadowing) with percentage control
//!   • Locality-aware load balancing
//!   • Round-robin / random / least-conn / consistent-hash endpoint selection
//!   • W3C Trace Context propagation

use crate::models::{
    ConsistentHashKey, Destination, DestinationRule, FaultEffect, HttpFaultInjection,
    HttpMatchRequest, HttpRedirect, HttpRewrite, HttpRoute, HttpRouteDestination, IncomingRequest,
    LoadBalancerMode, Locality, MirrorDecision, RouteDecision, VirtualService,
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::debug;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct TrafficManager {
    virtual_services: Arc<RwLock<HashMap<String, VirtualService>>>,
    destination_rules: Arc<RwLock<HashMap<String, DestinationRule>>>,
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
        let mut seen = std::collections::HashSet::new();
        map.values().filter(|v| seen.insert(v.name.clone())).cloned().collect()
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
    /// Returns `None` when no VirtualService covers the host.
    pub fn resolve_route(&self, host: &str, req: &IncomingRequest) -> Option<RouteDecision> {
        let vs = self.get_virtual_service(host)?;

        for route in &vs.http {
            if self.request_matches(req, &route.match_rules) {
                return Some(self.build_decision(route, req));
            }
        }
        None
    }

    // ─── Match helpers ───────────────────────────────────────

    fn request_matches(&self, req: &IncomingRequest, rules: &[HttpMatchRequest]) -> bool {
        if rules.is_empty() {
            return true;
        }
        rules.iter().any(|rule| self.single_rule_matches(req, rule))
    }

    fn single_rule_matches(&self, req: &IncomingRequest, rule: &HttpMatchRequest) -> bool {
        if let Some(uri_match) = &rule.uri {
            let uri = if rule.ignore_uri_case {
                req.uri.to_lowercase()
            } else {
                req.uri.clone()
            };
            if !uri_match.matches(&uri) {
                return false;
            }
        }
        if let Some(method_match) = &rule.method {
            if !method_match.matches(&req.method) {
                return false;
            }
        }
        if let Some(auth_match) = &rule.authority {
            let authority = req.authority.as_deref().unwrap_or("");
            if !auth_match.matches(authority) {
                return false;
            }
        }
        for (name, header_match) in &rule.headers {
            let value = req.headers.get(name).map(|s| s.as_str()).unwrap_or("");
            if !header_match.matches(value) {
                return false;
            }
        }
        // Without-headers: these headers must NOT be present / must NOT match
        for (name, header_match) in &rule.without_headers {
            let value = req.headers.get(name).map(|s| s.as_str()).unwrap_or("");
            if header_match.matches(value) {
                return false;
            }
        }
        for (param, param_match) in &rule.query_params {
            let value = req.query_params.get(param).map(|s| s.as_str()).unwrap_or("");
            if !param_match.matches(value) {
                return false;
            }
        }
        for (k, v) in &rule.source_labels {
            if req.source_labels.get(k).map(|s| s.as_str()) != Some(v.as_str()) {
                return false;
            }
        }
        if let Some(ns) = &rule.source_namespace {
            if req.source_namespace.as_deref() != Some(ns.as_str()) {
                return false;
            }
        }
        if let Some(gw) = &rule.gateways.first() {
            if req.gateway.as_deref() != Some(gw.as_str()) {
                return false;
            }
        }
        if let Some(port) = rule.port {
            // Only enforce port if the request carries port context (optional)
            let _ = port; // no-op — port filtering is enforced by Gateway, not VS
        }
        true
    }

    // ─── Decision Builder ────────────────────────────────────

    fn build_decision(&self, route: &HttpRoute, req: &IncomingRequest) -> RouteDecision {
        // Handle redirect first (short-circuit: no destination needed)
        if let Some(redirect) = &route.redirect {
            return RouteDecision {
                destination_host: String::new(),
                destination_subset: None,
                destination_port: None,
                weight: 0,
                fault: None,
                retry: None,
                timeout_ms: None,
                request_headers_add: HashMap::new(),
                request_headers_remove: vec![],
                response_headers_add: HashMap::new(),
                response_headers_remove: vec![],
                traceparent: None,
                redirect: Some(redirect.clone()),
                rewrite: None,
                mirror: None,
                cors_policy: route.cors_policy.clone(),
            };
        }

        let dest = self.pick_destination(&route.route);
        let fault = route.fault.as_ref().and_then(|f| self.evaluate_fault(f));
        let traceparent = Some(propagate_traceparent(req.traceparent.as_deref()));

        let mut request_headers_add = HashMap::new();
        let mut request_headers_remove = Vec::new();
        let mut response_headers_add = HashMap::new();
        let mut response_headers_remove = Vec::new();

        if let Some(hops) = &route.headers {
            if let Some(req_h) = &hops.request {
                request_headers_add.extend(req_h.set.clone());
                request_headers_add.extend(req_h.add.clone());
                request_headers_remove.extend(req_h.remove.clone());
            }
            if let Some(res_h) = &hops.response {
                response_headers_add.extend(res_h.set.clone());
                response_headers_add.extend(res_h.add.clone());
                response_headers_remove.extend(res_h.remove.clone());
            }
        }
        if let Some(tp) = &traceparent {
            request_headers_add.insert("traceparent".to_string(), tp.clone());
        }

        // Mirror decision
        let mirror = route.mirror.as_ref().map(|m| {
            MirrorDecision {
                host: m.host.clone(),
                subset: m.subset.clone(),
                port: m.port.as_ref().map(|p| p.number),
                percentage: route.mirror_percentage.unwrap_or(100.0),
            }
        });

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
            request_headers_remove,
            response_headers_add,
            response_headers_remove,
            traceparent,
            redirect: None,
            rewrite: route.rewrite.clone(),
            mirror,
            cors_policy: route.cors_policy.clone(),
        }
    }

    /// Weighted random destination selection.
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
        let pct = (Uuid::new_v4().as_u128() % 100) as f64;
        // Abort takes priority
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

    // ─── Endpoint selection (data-plane facing) ──────────────

    /// Select an endpoint index using the configured load-balancing algorithm.
    pub fn select_endpoint_index(
        &self,
        host: &str,
        subset: Option<&str>,
        endpoint_count: usize,
        request_key: Option<&str>,
    ) -> usize {
        if endpoint_count == 0 {
            return 0;
        }
        let dr = self.get_destination_rule(host);
        let (mode, consistent_hash) = dr
            .as_ref()
            .map(|d| {
                let subset_policy = subset.and_then(|s| {
                    d.subsets
                        .iter()
                        .find(|sub| sub.name == s)
                        .and_then(|sub| sub.traffic_policy.as_ref())
                        .and_then(|tp| tp.load_balancer.as_ref())
                });
                let policy = subset_policy
                    .or_else(|| d.traffic_policy.as_ref().and_then(|tp| tp.load_balancer.as_ref()));
                let mode = policy.map(|lb| &lb.mode).unwrap_or(&LoadBalancerMode::RoundRobin);
                let ch = policy.and_then(|lb| lb.consistent_hash.as_ref());
                (mode.clone(), ch.cloned())
            })
            .unwrap_or((LoadBalancerMode::RoundRobin, None));

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
                // Without live counters, fall back to random
                (Uuid::new_v4().as_u128() % endpoint_count as u128) as usize
            }
            LoadBalancerMode::ConsistentHash => {
                if let (Some(key_val), Some(_ch)) = (request_key, consistent_hash) {
                    hash_key(key_val) % endpoint_count
                } else {
                    (Uuid::new_v4().as_u128() % endpoint_count as u128) as usize
                }
            }
        }
    }

    /// Locality-aware endpoint selection: prefer same-region endpoints,
    /// fall back to weighted failover, finally random across all.
    pub fn select_endpoint_locality(
        &self,
        host: &str,
        all_endpoints: &[crate::models::Endpoint],
        request_locality: Option<&Locality>,
    ) -> usize {
        if all_endpoints.is_empty() {
            return 0;
        }
        let dr = self.get_destination_rule(host);
        let locality_enabled = dr
            .as_ref()
            .and_then(|d| d.traffic_policy.as_ref())
            .and_then(|tp| tp.load_balancer.as_ref())
            .and_then(|lb| lb.locality_lb_setting.as_ref())
            .and_then(|l| l.enabled)
            .unwrap_or(true);

        if !locality_enabled || request_locality.is_none() {
            return (Uuid::new_v4().as_u128() % all_endpoints.len() as u128) as usize;
        }
        let req_loc = request_locality.unwrap();

        // First: same region + zone
        let same_zone: Vec<usize> = all_endpoints
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.locality.as_ref().map(|l| {
                    l.region == req_loc.region
                        && l.zone == req_loc.zone
                }).unwrap_or(false)
            })
            .map(|(i, _)| i)
            .collect();
        if !same_zone.is_empty() {
            return same_zone[(Uuid::new_v4().as_u128() % same_zone.len() as u128) as usize];
        }

        // Second: same region
        let same_region: Vec<usize> = all_endpoints
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.locality.as_ref().map(|l| l.region == req_loc.region).unwrap_or(false)
            })
            .map(|(i, _)| i)
            .collect();
        if !same_region.is_empty() {
            return same_region
                [(Uuid::new_v4().as_u128() % same_region.len() as u128) as usize];
        }

        // Fallback: any
        (Uuid::new_v4().as_u128() % all_endpoints.len() as u128) as usize
    }
}

// ─────────────────────────────────────────────────────────────
// W3C Trace Context helpers
// ─────────────────────────────────────────────────────────────

/// Propagate or generate a W3C traceparent header.
/// Format: 00-<trace-id>-<parent-id>-<flags>
pub fn propagate_traceparent(incoming: Option<&str>) -> String {
    if let Some(tp) = incoming {
        let parts: Vec<&str> = tp.splitn(4, '-').collect();
        if parts.len() == 4 {
            let trace_id = parts[1];
            let new_parent_id = &Uuid::new_v4().simple().to_string()[..16];
            return format!("00-{trace_id}-{new_parent_id}-01");
        }
    }
    let trace_id = Uuid::new_v4().simple().to_string();
    let parent_id = &Uuid::new_v4().simple().to_string()[..16];
    format!("00-{trace_id}-{parent_id}-01")
}

// ─────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────

fn hash_key(key: &str) -> usize {
    // FNV-1a hash (fast, no external dep)
    let mut hash: u64 = 14695981039346656037;
    for byte in key.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash as usize
}
