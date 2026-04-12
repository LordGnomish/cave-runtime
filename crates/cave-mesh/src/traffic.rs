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
}
