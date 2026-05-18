// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alert rule evaluation: pending/firing states, grouping.

use crate::models::{AlertRule, AlertState};
use crate::query::execute_query;
use crate::storage::TimeSeriesStore;
use chrono::Utc;
use std::collections::HashMap;

/// Evaluate all alert rules against the current store.
/// Updates state in-place (Inactive → Pending → Firing, or back to Inactive).
pub fn evaluate_alert_rules(rules: &mut Vec<AlertRule>, store: &TimeSeriesStore) {
    let now = Utc::now();

    for rule in rules.iter_mut() {
        rule.last_evaluated = Some(now);

        let result = execute_query(store, &rule.expr, now);
        let has_values = !result.data.result.is_empty()
            && result.data.result.iter().any(|r| {
                r.values.iter().any(|v| {
                    // value is index 1 in [timestamp, value]
                    v[1].as_str()
                        .and_then(|s| s.parse::<f64>().ok())
                        .map(|n| n != 0.0)
                        .unwrap_or(false)
                })
            });

        rule.state = match rule.state {
            AlertState::Inactive => {
                if has_values {
                    if rule.for_duration_seconds == 0 {
                        rule.fired_at = Some(now);
                        AlertState::Firing
                    } else {
                        AlertState::Pending
                    }
                } else {
                    AlertState::Inactive
                }
            }
            AlertState::Pending => {
                if has_values {
                    // Check if we've been pending long enough
                    let pending_since = rule.fired_at.unwrap_or(now);
                    let duration = (now - pending_since).num_seconds() as u64;
                    if duration >= rule.for_duration_seconds {
                        rule.fired_at = Some(now);
                        AlertState::Firing
                    } else {
                        if rule.fired_at.is_none() {
                            rule.fired_at = Some(now);
                        }
                        AlertState::Pending
                    }
                } else {
                    rule.fired_at = None;
                    AlertState::Inactive
                }
            }
            AlertState::Firing => {
                if has_values {
                    AlertState::Firing
                } else {
                    rule.fired_at = None;
                    AlertState::Inactive
                }
            }
        };
    }
}

/// Group alerts by their `alertname` label or rule name.
pub fn group_alerts(rules: &[AlertRule]) -> HashMap<String, Vec<&AlertRule>> {
    let mut groups: HashMap<String, Vec<&AlertRule>> = HashMap::new();
    for rule in rules {
        let group_key = rule
            .labels
            .get("alertname")
            .cloned()
            .unwrap_or_else(|| rule.group.clone());
        groups.entry(group_key).or_default().push(rule);
    }
    groups
}

/// Return only firing alerts.
pub fn firing_alerts(rules: &[AlertRule]) -> Vec<&AlertRule> {
    rules.iter().filter(|r| r.state == AlertState::Firing).collect()
}

/// Return only pending alerts.
pub fn pending_alerts(rules: &[AlertRule]) -> Vec<&AlertRule> {
    rules.iter().filter(|r| r.state == AlertState::Pending).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::TimeSeriesStore;

    fn make_rule(name: &str, expr: &str) -> AlertRule {
        AlertRule::new(name, "default", expr)
    }

    #[test]
    fn test_inactive_with_empty_store() {
        let mut rules = vec![make_rule("HighCPU", "cpu_usage")];
        let store = TimeSeriesStore::default();
        evaluate_alert_rules(&mut rules, &store);
        assert_eq!(rules[0].state, AlertState::Inactive);
    }

    #[test]
    fn test_group_alerts() {
        let rules = vec![
            make_rule("HighCPU", "cpu_usage"),
            make_rule("HighMem", "mem_usage"),
        ];
        let groups = group_alerts(&rules);
        // grouped by rule.group = "default"
        assert!(groups.contains_key("default"));
        assert_eq!(groups["default"].len(), 2);
    }

    #[test]
    fn test_firing_filter() {
        let mut rules = vec![make_rule("test", "metric")];
        rules[0].state = AlertState::Firing;
        assert_eq!(firing_alerts(&rules).len(), 1);
        assert_eq!(pending_alerts(&rules).len(), 0);
    }
}
