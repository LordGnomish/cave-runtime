//! In-memory store for cave-cost-alloc.

use crate::cost_model::CostCalculator;
use crate::models::*;
use crate::recommendations::RecommendationEngine;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub struct CostAllocStore {
    allocations: Arc<Mutex<Vec<CostAllocation>>>,
    budgets: Arc<Mutex<Vec<BudgetAlert>>>,
    cloud_costs: Arc<Mutex<Vec<CloudCostEntry>>>,
    recommendations: Arc<Mutex<Vec<Recommendation>>>,
    pub rates: CostRate,
}

impl Default for CostAllocStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CostAllocStore {
    pub fn new() -> Self {
        Self {
            allocations: Arc::new(Mutex::new(Vec::new())),
            budgets: Arc::new(Mutex::new(Vec::new())),
            cloud_costs: Arc::new(Mutex::new(Vec::new())),
            recommendations: Arc::new(Mutex::new(Vec::new())),
            rates: CostCalculator::default_rates(),
        }
    }

    // ─── Allocations ──────────────────────────────────────────────────────────

    pub fn list_allocations(&self, query: &AllocationQuery) -> Vec<CostAllocation> {
        self.allocations
            .lock()
            .unwrap()
            .iter()
            .filter(|a| {
                let ns_ok = query.namespace.as_deref().is_none_or(|ns| a.namespace == ns);
                let team_ok =
                    query.team.as_deref().is_none_or(|t| a.team.as_deref() == Some(t));
                let start_ok = query.start.is_none_or(|s| a.period_start >= s);
                let end_ok = query.end.is_none_or(|e| a.period_end <= e);
                ns_ok && team_ok && start_ok && end_ok
            })
            .cloned()
            .collect()
    }

    pub fn add_allocation(&self, req: CreateAllocationRequest) -> CostAllocation {
        let hours = {
            let dur = req.period_end - req.period_start;
            dur.num_minutes() as f64 / 60.0
        };
        let cost = CostCalculator::compute_cost(&req.usage, &self.rates, hours);
        let requested = req.requested_usage.as_ref().unwrap_or(&req.usage);
        let efficiency = CostCalculator::efficiency_score(requested, &req.usage);

        let alloc = CostAllocation {
            id: Uuid::new_v4(),
            namespace: req.namespace,
            deployment: req.deployment,
            pod: req.pod,
            container: req.container,
            labels: req.labels.unwrap_or_default(),
            team: req.team,
            cost_center: req.cost_center,
            usage: req.usage,
            cost,
            efficiency_score: efficiency,
            period_start: req.period_start,
            period_end: req.period_end,
        };
        self.allocations.lock().unwrap().push(alloc.clone());
        alloc
    }

    // ─── Showback ─────────────────────────────────────────────────────────────

    pub fn generate_showback_report(&self, group_by: &str) -> ShowbackReport {
        let allocations = self.allocations.lock().unwrap().clone();
        let grouped: HashMap<String, ResourceCost> = match group_by {
            "team" => CostCalculator::aggregate_by_team(&allocations),
            _ => CostCalculator::aggregate_by_namespace(&allocations),
        };

        let total_cost: f64 = grouped.values().map(|c| c.total_cost).sum();
        let mut line_items: Vec<ShowbackLineItem> = grouped
            .into_iter()
            .map(|(group, cost)| {
                let pct = if total_cost > 0.0 {
                    cost.total_cost / total_cost * 100.0
                } else {
                    0.0
                };
                ShowbackLineItem { group, cost, percentage: pct }
            })
            .collect();
        line_items.sort_by(|a, b| b.cost.total_cost.partial_cmp(&a.cost.total_cost).unwrap());

        ShowbackReport {
            group_by: group_by.into(),
            period_start: allocations.iter().map(|a| a.period_start).min(),
            period_end: allocations.iter().map(|a| a.period_end).max(),
            line_items,
            total_cost,
        }
    }

    // ─── Chargeback ───────────────────────────────────────────────────────────

    pub fn generate_chargeback_report(&self) -> ChargebackReport {
        let allocations = self.allocations.lock().unwrap().clone();
        let mut by_cost_center: HashMap<String, ResourceCost> = HashMap::new();
        let mut cc_teams: HashMap<String, Option<String>> = HashMap::new();

        for alloc in &allocations {
            let cc = alloc.cost_center.clone().unwrap_or_else(|| "unassigned".into());
            let entry = by_cost_center.entry(cc.clone()).or_default();
            *entry = entry.add(&alloc.cost);
            cc_teams.entry(cc).or_insert_with(|| alloc.team.clone());
        }

        let total_cost: f64 = by_cost_center.values().map(|c| c.total_cost).sum();
        let mut line_items: Vec<ChargebackLineItem> = by_cost_center
            .into_iter()
            .map(|(cc, cost)| {
                let pct = if total_cost > 0.0 {
                    cost.total_cost / total_cost * 100.0
                } else {
                    0.0
                };
                ChargebackLineItem {
                    cost_center: cc.clone(),
                    team: cc_teams.get(&cc).cloned().flatten(),
                    cost,
                    allocation_pct: pct,
                }
            })
            .collect();
        line_items.sort_by(|a, b| b.cost.total_cost.partial_cmp(&a.cost.total_cost).unwrap());

        ChargebackReport {
            period_start: allocations.iter().map(|a| a.period_start).min(),
            period_end: allocations.iter().map(|a| a.period_end).max(),
            line_items,
            total_cost,
        }
    }

    // ─── Budgets ──────────────────────────────────────────────────────────────

    pub fn list_budgets(&self) -> Vec<BudgetAlert> {
        self.budgets.lock().unwrap().clone()
    }

    pub fn create_budget(&self, req: CreateBudgetAlertRequest) -> BudgetAlert {
        let alert = BudgetAlert {
            id: Uuid::new_v4(),
            name: req.name,
            namespace: req.namespace,
            team: req.team,
            cost_center: req.cost_center,
            threshold_usd: req.threshold_usd,
            period: req.period,
            current_spend: 0.0,
            alert_fired: false,
            created_at: Utc::now(),
        };
        self.budgets.lock().unwrap().push(alert.clone());
        alert
    }

    pub fn check_budgets(&self) -> Vec<BudgetAlert> {
        let allocations = self.allocations.lock().unwrap().clone();
        let mut budgets = self.budgets.lock().unwrap();
        CostCalculator::check_budget_alerts(&mut budgets, &allocations);
        budgets.clone()
    }

    // ─── Cloud costs ──────────────────────────────────────────────────────────

    pub fn list_cloud_costs(&self, provider: Option<CloudProvider>) -> Vec<CloudCostEntry> {
        let guard = self.cloud_costs.lock().unwrap();
        match provider {
            Some(p) => guard.iter().filter(|e| e.provider == p).cloned().collect(),
            None => guard.clone(),
        }
    }

    pub fn add_cloud_cost(&self, req: AddCloudCostRequest) -> CloudCostEntry {
        let entry = CloudCostEntry {
            id: Uuid::new_v4(),
            provider: req.provider,
            account_id: req.account_id,
            service: req.service,
            region: req.region,
            resource_id: req.resource_id,
            tags: req.tags.unwrap_or_default(),
            cost_usd: req.cost_usd,
            usage_quantity: req.usage_quantity,
            usage_unit: req.usage_unit,
            period_start: req.period_start,
            period_end: req.period_end,
        };
        self.cloud_costs.lock().unwrap().push(entry.clone());
        entry
    }

    // ─── Recommendations ──────────────────────────────────────────────────────

    pub fn get_recommendations(&self) -> Vec<Recommendation> {
        self.recommendations.lock().unwrap().clone()
    }

    pub fn refresh_recommendations(&self) -> Vec<Recommendation> {
        let allocations = self.allocations.lock().unwrap().clone();
        let recs = RecommendationEngine::analyze(&allocations, &self.rates);
        let mut guard = self.recommendations.lock().unwrap();
        *guard = recs.clone();
        recs
    }

    // ─── Efficiency ───────────────────────────────────────────────────────────

    pub fn efficiency_report(&self, namespace: Option<&str>) -> Vec<EfficiencyReport> {
        let allocations: Vec<CostAllocation> = {
            let guard = self.allocations.lock().unwrap();
            guard
                .iter()
                .filter(|a| namespace.is_none_or(|ns| a.namespace == ns))
                .cloned()
                .collect()
        };

        // Group by namespace
        let mut by_ns: HashMap<String, Vec<&CostAllocation>> = HashMap::new();
        for alloc in &allocations {
            by_ns.entry(alloc.namespace.clone()).or_default().push(alloc);
        }

        by_ns
            .into_iter()
            .map(|(ns, allocs)| {
                let cpu_used: f64 = allocs.iter().map(|a| a.usage.cpu_cores).sum();
                let mem_used: f64 = allocs.iter().map(|a| a.usage.memory_gb).sum();
                // For requested we use the same values (since we store actual; a real
                // implementation would store requests separately).
                let avg_eff: f32 =
                    allocs.iter().map(|a| a.efficiency_score).sum::<f32>() / allocs.len() as f32;
                EfficiencyReport {
                    namespace: ns,
                    cpu_requested: cpu_used,
                    cpu_used,
                    memory_requested_gb: mem_used,
                    memory_used_gb: mem_used,
                    cpu_efficiency: avg_eff.min(1.0),
                    memory_efficiency: avg_eff.min(1.0),
                    overall_efficiency: avg_eff.min(1.0),
                }
            })
            .collect()
    }

    // ─── Summary ──────────────────────────────────────────────────────────────

    pub fn summary(&self) -> serde_json::Value {
        let (total_cost, total_namespaces_count, avg_efficiency) = {
            let allocations = self.allocations.lock().unwrap();
            let total_cost: f64 = allocations.iter().map(|a| a.cost.total_cost).sum();
            let total_namespaces: std::collections::HashSet<&str> =
                allocations.iter().map(|a| a.namespace.as_str()).collect();
            let ns_count = total_namespaces.len();
            let avg_efficiency: f32 = if allocations.is_empty() {
                0.0
            } else {
                allocations.iter().map(|a| a.efficiency_score).sum::<f32>()
                    / allocations.len() as f32
            };
            (total_cost, ns_count, avg_efficiency)
        };

        let budgets = self.budgets.lock().unwrap();
        let alerts_fired = budgets.iter().filter(|b| b.alert_fired).count();
        drop(budgets);

        let recs = self.recommendations.lock().unwrap();
        let total_potential_savings: f64 =
            recs.iter().map(|r| r.estimated_savings_usd_monthly).sum();
        drop(recs);

        serde_json::json!({
            "total_cost_usd": total_cost,
            "namespaces": total_namespaces_count,
            "avg_efficiency": avg_efficiency,
            "budget_alerts_fired": alerts_fired,
            "potential_monthly_savings_usd": total_potential_savings,
        })
    }
}
