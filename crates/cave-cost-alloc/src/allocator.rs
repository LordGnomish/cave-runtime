// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use crate::models::{
    AnomalySeverity, AnomalyStatus, CostAllocation, CostAnomaly, CostCenter, CostReport,
    IdleResource, ResourceType, SplitStrategy,
};

/// A raw spend entry from a cloud provider or cluster metrics scrape.
#[derive(Debug, Clone)]
pub struct RawSpendEntry {
    pub resource_id: String,
    pub resource_type: ResourceType,
    pub cost_usd: f64,
    /// Resource tags (e.g. "team", "project", "environment").
    pub tags: HashMap<String, String>,
}

/// Distribute raw cloud spend to cost centers based on resource tags.
///
/// Matches on `team` or `project` tags against `CostCenter` fields.
/// Unmatched resources are dropped (no default cost center assumed).
pub fn allocate_costs(
    cost_centers: &[CostCenter],
    raw_entries: &[RawSpendEntry],
) -> Vec<CostAllocation> {
    let now = Utc::now();

    raw_entries
        .iter()
        .filter_map(|entry| {
            find_cost_center(cost_centers, &entry.tags).map(|cc| CostAllocation {
                id: Uuid::new_v4(),
                resource_id: entry.resource_id.clone(),
                resource_type: entry.resource_type.clone(),
                cost_center_id: cc.id,
                split_percentage: 100.0,
                effective_from: now,
                effective_to: None,
                created_at: now,
            })
        })
        .collect()
}

/// Proportional split of shared infrastructure costs across cost centers.
///
/// Returns `(cost_center_id, allocated_usd)` pairs.
pub fn split_shared_costs(
    shared_cost_usd: f64,
    strategy: &SplitStrategy,
    cost_centers: &[CostCenter],
    // (cost_center_id, usage_value) — interpretation depends on strategy.
    usage: &[(Uuid, f64)],
) -> Vec<(Uuid, f64)> {
    if cost_centers.is_empty() {
        return vec![];
    }

    match strategy {
        SplitStrategy::Equal => {
            let share = shared_cost_usd / cost_centers.len() as f64;
            cost_centers.iter().map(|cc| (cc.id, share)).collect()
        }

        SplitStrategy::ByCpu | SplitStrategy::ByMemory | SplitStrategy::ByRequestCount => {
            let total: f64 = usage.iter().map(|(_, v)| v).sum();
            if total == 0.0 {
                let share = shared_cost_usd / cost_centers.len() as f64;
                return cost_centers.iter().map(|cc| (cc.id, share)).collect();
            }
            usage
                .iter()
                .map(|(id, u)| (*id, shared_cost_usd * (u / total)))
                .collect()
        }

        SplitStrategy::ByCustomWeights { weights } => {
            let total_weight: f64 = weights.values().sum();
            if total_weight == 0.0 {
                return vec![];
            }
            cost_centers
                .iter()
                .filter_map(|cc| {
                    weights
                        .get(&cc.id.to_string())
                        .map(|w| (cc.id, shared_cost_usd * (w / total_weight)))
                })
                .collect()
        }
    }
}

/// Identify idle/wasted resources with utilization below `idle_threshold_pct`.
///
/// Input tuples: `(resource_id, resource_type, cost_center_id, utilization_pct, hourly_cost_usd)`.
pub fn calculate_idle_costs(
    resource_utilization: &[(String, ResourceType, Option<Uuid>, f64, f64)],
    idle_threshold_pct: f64,
) -> Vec<IdleResource> {
    resource_utilization
        .iter()
        .filter(|(_, _, _, util, _)| *util < idle_threshold_pct)
        .map(|(id, rtype, cc_id, util, hourly_cost)| {
            let monthly_waste = hourly_cost * 24.0 * 30.0 * (1.0 - util / 100.0);
            IdleResource {
                resource_id: id.clone(),
                resource_type: rtype.clone(),
                cost_center_id: *cc_id,
                utilization_pct: *util,
                wasted_cost_usd: monthly_waste,
                recommendation: idle_recommendation(rtype, *util),
            }
        })
        .collect()
}

/// Detect cost anomalies by comparing each report's spend to the historical mean.
///
/// Reports that deviate more than `deviation_threshold_pct` from the per-cost-center
/// mean are flagged as anomalies.
pub fn detect_anomalies(reports: &[CostReport], deviation_threshold_pct: f64) -> Vec<CostAnomaly> {
    if reports.len() < 2 {
        return vec![];
    }

    // Compute per-cost-center mean across all historical reports.
    let mut by_cc: HashMap<Uuid, Vec<f64>> = HashMap::new();
    for r in reports {
        by_cc
            .entry(r.cost_center_id)
            .or_default()
            .push(r.total_cost_usd);
    }

    let now = Utc::now();
    let mut anomalies = Vec::new();

    for r in reports {
        let costs = match by_cc.get(&r.cost_center_id) {
            Some(c) if c.len() >= 2 => c,
            _ => continue,
        };

        let mean = costs.iter().sum::<f64>() / costs.len() as f64;
        if mean == 0.0 {
            continue;
        }

        let deviation = ((r.total_cost_usd - mean) / mean) * 100.0;
        if deviation.abs() <= deviation_threshold_pct {
            continue;
        }

        let severity = match deviation.abs() as u32 {
            d if d > 200 => AnomalySeverity::Critical,
            d if d > 100 => AnomalySeverity::High,
            d if d > 50 => AnomalySeverity::Medium,
            _ => AnomalySeverity::Low,
        };

        anomalies.push(CostAnomaly {
            id: Uuid::new_v4(),
            cost_center_id: r.cost_center_id,
            resource_id: r.id.to_string(),
            detected_at: now,
            expected_cost_usd: mean,
            actual_cost_usd: r.total_cost_usd,
            deviation_pct: deviation,
            severity,
            status: AnomalyStatus::Open,
        });
    }

    anomalies
}

// --- helpers ---

fn find_cost_center<'a>(
    cost_centers: &'a [CostCenter],
    tags: &HashMap<String, String>,
) -> Option<&'a CostCenter> {
    cost_centers.iter().find(|cc| {
        tags.get("team").is_some_and(|t| t == &cc.team)
            || tags.get("project").is_some_and(|p| p == &cc.project)
    })
}

fn idle_recommendation(resource_type: &ResourceType, utilization_pct: f64) -> String {
    let action = if utilization_pct < 5.0 {
        "terminate"
    } else if utilization_pct < 20.0 {
        "downsize"
    } else {
        "review"
    };
    format!(
        "Consider {} this {:?} (utilization {:.1}%)",
        action, resource_type, utilization_pct
    )
}
