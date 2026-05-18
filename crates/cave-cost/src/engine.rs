// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{CostBudget, CostEntry, CostSummary};
use std::collections::HashMap;

pub fn summarize_by_team(entries: &[CostEntry]) -> Vec<CostSummary> {
    let mut by_team: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for entry in entries {
        let team = by_team.entry(entry.team.clone()).or_default();
        *team.entry(entry.service.clone()).or_insert(0.0) += entry.cost_usd;
    }
    by_team
        .into_iter()
        .map(|(team, by_service)| {
            let total = by_service.values().sum();
            CostSummary {
                team,
                total_usd: total,
                by_service,
            }
        })
        .collect()
}

pub fn is_over_budget(entries: &[CostEntry], budget: &CostBudget, team: &str) -> bool {
    let total: f64 = entries
        .iter()
        .filter(|e| e.team == team)
        .map(|e| e.cost_usd)
        .sum();
    total > budget.monthly_limit_usd
}

pub fn alert_threshold_usd(budget: &CostBudget) -> f64 {
    budget.monthly_limit_usd * budget.alert_threshold_percent / 100.0
}

pub fn is_near_budget(entries: &[CostEntry], budget: &CostBudget, team: &str) -> bool {
    let total: f64 = entries
        .iter()
        .filter(|e| e.team == team)
        .map(|e| e.cost_usd)
        .sum();
    total >= alert_threshold_usd(budget)
}

pub fn top_services_by_cost(entries: &[CostEntry], n: usize) -> Vec<(String, f64)> {
    let mut by_service: HashMap<String, f64> = HashMap::new();
    for e in entries {
        *by_service.entry(e.service.clone()).or_insert(0.0) += e.cost_usd;
    }
    let mut sorted: Vec<(String, f64)> = by_service.into_iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    sorted.truncate(n);
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn make_entry(team: &str, service: &str, cost: f64) -> CostEntry {
        CostEntry {
            id: Uuid::new_v4(),
            service: service.to_string(),
            resource_id: "res-1".to_string(),
            team: team.to_string(),
            environment: "prod".to_string(),
            cost_usd: cost,
            date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            tags: std::collections::HashMap::new(),
        }
    }

    fn make_budget(team: &str, limit: f64, threshold_pct: f64) -> CostBudget {
        CostBudget {
            id: Uuid::new_v4(),
            name: format!("{}-budget", team),
            team: team.to_string(),
            monthly_limit_usd: limit,
            alert_threshold_percent: threshold_pct,
        }
    }

    #[test]
    fn test_summarize_by_team_groups_correctly() {
        let entries = vec![
            make_entry("team-a", "ec2", 100.0),
            make_entry("team-a", "s3", 50.0),
            make_entry("team-b", "rds", 200.0),
        ];
        let mut summaries = summarize_by_team(&entries);
        summaries.sort_by(|a, b| a.team.cmp(&b.team));
        assert_eq!(summaries.len(), 2);
        let a = &summaries[0];
        assert_eq!(a.team, "team-a");
        assert!((a.total_usd - 150.0).abs() < 0.001);
        assert_eq!(a.by_service.get("ec2"), Some(&100.0));
        let b = &summaries[1];
        assert_eq!(b.team, "team-b");
        assert!((b.total_usd - 200.0).abs() < 0.001);
    }

    #[test]
    fn test_is_over_budget_true() {
        let entries = vec![make_entry("team-a", "ec2", 1100.0)];
        let budget = make_budget("team-a", 1000.0, 80.0);
        assert!(is_over_budget(&entries, &budget, "team-a"));
    }

    #[test]
    fn test_is_over_budget_false() {
        let entries = vec![make_entry("team-a", "ec2", 500.0)];
        let budget = make_budget("team-a", 1000.0, 80.0);
        assert!(!is_over_budget(&entries, &budget, "team-a"));
    }

    #[test]
    fn test_alert_threshold_calculation() {
        let budget = make_budget("team-a", 1000.0, 80.0);
        let threshold = alert_threshold_usd(&budget);
        assert!((threshold - 800.0).abs() < 0.001);
    }

    #[test]
    fn test_is_near_budget_true() {
        // spending above threshold (850 >= 800)
        let entries = vec![make_entry("team-a", "ec2", 850.0)];
        let budget = make_budget("team-a", 1000.0, 80.0);
        assert!(is_near_budget(&entries, &budget, "team-a"));
    }

    #[test]
    fn test_top_services_returns_n() {
        let entries = vec![
            make_entry("team-a", "ec2", 300.0),
            make_entry("team-a", "s3", 100.0),
            make_entry("team-a", "rds", 200.0),
        ];
        let top = top_services_by_cost(&entries, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "ec2");
        assert_eq!(top[1].0, "rds");
    }
}
