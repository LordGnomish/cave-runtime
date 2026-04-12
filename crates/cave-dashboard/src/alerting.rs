//! Unified Alerting engine — rule evaluation, contact points, notification routing,
//! silences, mute timings, and alert group aggregation.

use crate::models::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

// ─── Evaluation ───────────────────────────────────────────────────────────────

/// Result of evaluating a single alert rule against a set of data.
#[derive(Debug, Clone)]
pub struct EvalResult {
    pub rule_uid: String,
    pub state: AlertState,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub value: Option<f64>,
    pub evaluated_at: DateTime<Utc>,
    pub error: Option<String>,
}

/// Evaluate a single numeric value against a threshold-style condition.
/// This mirrors Grafana's classic condition evaluation.
pub fn eval_threshold(value: f64, eval_type: &str, params: &[f64]) -> bool {
    match eval_type {
        "gt" => params.first().map_or(false, |&t| value > t),
        "lt" => params.first().map_or(false, |&t| value < t),
        "gte" => params.first().map_or(false, |&t| value >= t),
        "lte" => params.first().map_or(false, |&t| value <= t),
        "eq" => params.first().map_or(false, |&t| (value - t).abs() < f64::EPSILON),
        "within_range" => {
            if params.len() >= 2 {
                value >= params[0] && value <= params[1]
            } else {
                false
            }
        }
        "outside_range" => {
            if params.len() >= 2 {
                value < params[0] || value > params[1]
            } else {
                false
            }
        }
        "no_value" => false,
        _ => false,
    }
}

/// Apply a reducer function over a series of f64 values.
pub fn apply_reducer(reducer_type: &str, values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    match reducer_type {
        "avg" | "mean" => Some(values.iter().sum::<f64>() / values.len() as f64),
        "min" => values.iter().cloned().reduce(f64::min),
        "max" => values.iter().cloned().reduce(f64::max),
        "sum" => Some(values.iter().sum()),
        "count" => Some(values.len() as f64),
        "last" => values.last().copied(),
        "first" => values.first().copied(),
        "diff" => {
            if values.len() >= 2 {
                Some(values.last().unwrap() - values.first().unwrap())
            } else {
                None
            }
        }
        "diff_abs" => {
            if values.len() >= 2 {
                Some((values.last().unwrap() - values.first().unwrap()).abs())
            } else {
                None
            }
        }
        "percent_diff" => {
            if values.len() >= 2 {
                let first = values.first().unwrap();
                if *first == 0.0 {
                    None
                } else {
                    Some((values.last().unwrap() - first) / first * 100.0)
                }
            } else {
                None
            }
        }
        "range" => {
            let min = values.iter().cloned().reduce(f64::min)?;
            let max = values.iter().cloned().reduce(f64::max)?;
            Some(max - min)
        }
        _ => values.last().copied(),
    }
}

/// Evaluate legacy alert conditions against a map of series values.
/// Returns the resulting AlertState.
pub fn evaluate_alert_conditions(
    conditions: &[AlertCondition],
    series_values: &HashMap<String, Vec<f64>>,
) -> AlertState {
    if conditions.is_empty() {
        return AlertState::Normal;
    }

    let mut overall = false;
    let mut first = true;

    for (i, cond) in conditions.iter().enumerate() {
        let ref_id = cond.query.params.first().map(|s| s.as_str()).unwrap_or("A");
        let values = series_values.get(ref_id).map(|v| v.as_slice()).unwrap_or(&[]);

        let reduced = apply_reducer(&cond.reducer.reducer_type, values);
        let firing = match reduced {
            Some(v) => eval_threshold(v, &cond.evaluator.eval_type, &cond.evaluator.params),
            None => cond.evaluator.eval_type == "no_value",
        };

        if first {
            overall = firing;
            first = false;
        } else {
            overall = match cond.operator.op_type.as_str() {
                "and" => overall && firing,
                "or" => overall || firing,
                _ => overall && firing,
            };
        }
    }

    if overall { AlertState::Firing } else { AlertState::Normal }
}

// ─── Alert Rule Evaluator (Unified Alerting) ──────────────────────────────────

/// Evaluate a Grafana Unified Alerting rule.
/// In a full implementation this would execute the queries against real datasources.
/// Here we provide the plumbing and return `Normal` for rules without data.
pub fn evaluate_alert_rule(rule: &AlertRule, data: &HashMap<String, Vec<f64>>) -> EvalResult {
    let now = Utc::now();

    // Find the condition query
    let condition_ref = &rule.condition;
    let values = data.get(condition_ref).map(|v| v.as_slice()).unwrap_or(&[]);

    // Simple threshold eval based on the condition query model
    let state = if let Some(model) = rule.data.iter().find(|q| &q.ref_id == condition_ref) {
        if let Some(conditions) = model.model.get("conditions").and_then(|c| c.as_array()) {
            let mut legacy_conditions = Vec::new();
            for cond in conditions {
                let evaluator = cond.get("evaluator").and_then(|e| serde_json::from_value::<AlertEvaluator>(e.clone()).ok()).unwrap_or_default();
                let reducer = cond.get("reducer").and_then(|r| serde_json::from_value::<AlertReducer>(r.clone()).ok()).unwrap_or_default();
                let operator = cond.get("operator").and_then(|o| serde_json::from_value::<AlertOperator>(o.clone()).ok()).unwrap_or_default();
                let query = cond.get("query").and_then(|q| serde_json::from_value::<AlertConditionQuery>(q.clone()).ok()).unwrap_or_default();
                legacy_conditions.push(AlertCondition {
                    condition_type: "query".into(),
                    query,
                    reducer,
                    evaluator,
                    operator,
                });
            }
            let mut values_map = HashMap::new();
            values_map.insert(condition_ref.clone(), values.to_vec());
            evaluate_alert_conditions(&legacy_conditions, &values_map)
        } else {
            // Expression evaluator — no_value or Normal
            if values.is_empty() {
                match rule.no_data_state {
                    NoDataState::Alerting => AlertState::Firing,
                    NoDataState::Ok => AlertState::Normal,
                    NoDataState::NoData => AlertState::NoData,
                    NoDataState::KeepState => rule.state,
                }
            } else {
                AlertState::Normal
            }
        }
    } else {
        AlertState::NoData
    };

    let reduced_value = apply_reducer("last", values);

    EvalResult {
        rule_uid: rule.uid.clone(),
        state,
        labels: rule.labels.clone(),
        annotations: rule.annotations.clone(),
        value: reduced_value,
        evaluated_at: now,
        error: None,
    }
}

// ─── Silence matching ─────────────────────────────────────────────────────────

/// Check whether an alert instance is silenced by any active silence.
pub fn is_silenced(
    alert_labels: &HashMap<String, String>,
    silences: &[Silence],
) -> bool {
    let now = Utc::now();
    for silence in silences {
        if silence.starts_at > now || silence.ends_at < now {
            continue;
        }
        if silence.status.state == "expired" {
            continue;
        }
        if matches_silence(alert_labels, &silence.matchers) {
            return true;
        }
    }
    false
}

fn matches_silence(labels: &HashMap<String, String>, matchers: &[SilenceMatcher]) -> bool {
    for m in matchers {
        let label_val = labels.get(&m.name).map(|s| s.as_str()).unwrap_or("");
        let matches = if m.is_regex {
            regex::Regex::new(&m.value).map(|re| re.is_match(label_val)).unwrap_or(false)
        } else if m.is_equal {
            label_val == m.value
        } else {
            label_val != m.value
        };
        if !matches {
            return false;
        }
    }
    true
}

// ─── Notification routing ─────────────────────────────────────────────────────

/// Walk the notification policy tree to find the receiver for a set of labels.
pub fn route_alert(
    policy: &NotificationPolicy,
    labels: &HashMap<String, String>,
) -> String {
    // Try sub-routes first
    for route in &policy.routes {
        if route_matches(route, labels) {
            let receiver = route_alert(route, labels);
            if !receiver.is_empty() {
                return receiver;
            }
            if !route.continue_policy {
                return route.receiver.clone();
            }
        }
    }
    policy.receiver.clone()
}

fn route_matches(policy: &NotificationPolicy, labels: &HashMap<String, String>) -> bool {
    for matcher in &policy.matchers {
        let label_val = labels.get(&matcher.name).map(|s| s.as_str()).unwrap_or("");
        let matches = if matcher.is_regex {
            regex::Regex::new(&matcher.value).map(|re| re.is_match(label_val)).unwrap_or(false)
        } else if matcher.is_equal {
            label_val == matcher.value
        } else {
            label_val != matcher.value
        };
        if !matches {
            return false;
        }
    }
    true
}

// ─── Alert group builder ──────────────────────────────────────────────────────

/// Group alert instances by their label sets according to `group_by`.
pub fn build_alert_groups(
    instances: Vec<AlertInstance>,
    policy: &NotificationPolicy,
) -> Vec<AlertGroup> {
    let group_by = &policy.group_by;
    let mut groups: HashMap<Vec<(String, String)>, Vec<AlertInstance>> = HashMap::new();

    for inst in instances {
        let key: Vec<(String, String)> = if group_by.iter().any(|g| g == "...") {
            // Group by all labels
            let mut kv: Vec<_> = inst.labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            kv.sort();
            kv
        } else {
            group_by.iter()
                .filter_map(|g| inst.labels.get(g).map(|v| (g.clone(), v.clone())))
                .collect()
        };
        groups.entry(key).or_default().push(inst);
    }

    groups.into_iter().map(|(key, alerts)| {
        let labels: HashMap<String, String> = key.into_iter().collect();
        let receiver_name = route_alert(policy, &labels);
        AlertGroup {
            labels,
            receiver: AlertReceiver { name: receiver_name },
            alerts,
        }
    }).collect()
}

// ─── Mute timing check ────────────────────────────────────────────────────────

/// Check whether the current time falls within any mute timing interval.
pub fn is_muted(mute_timing: &MuteTiming, now: &DateTime<Utc>) -> bool {
    for interval in &mute_timing.time_intervals {
        if interval_matches(interval, now) {
            return true;
        }
    }
    false
}

fn interval_matches(interval: &TimeInterval, now: &DateTime<Utc>) -> bool {
    use chrono::Datelike;
    use chrono::Timelike;

    // Check weekday
    if !interval.weekdays.is_empty() {
        let wd = now.weekday().to_string().to_lowercase();
        let day_abbr = &wd[..3];
        if !interval.weekdays.iter().any(|w| w.to_lowercase().starts_with(day_abbr)) {
            return false;
        }
    }

    // Check day of month
    if !interval.days_of_month.is_empty() {
        let dom = now.day() as i32;
        let in_range = interval.days_of_month.iter().any(|d| {
            if let Some((start, end)) = d.split_once(':') {
                let s: i32 = start.parse().unwrap_or(1);
                let e: i32 = end.parse().unwrap_or(31);
                dom >= s && dom <= e
            } else {
                d.parse::<i32>().map(|v| v == dom).unwrap_or(false)
            }
        });
        if !in_range {
            return false;
        }
    }

    // Check months
    if !interval.months.is_empty() {
        let month = now.month() as i32;
        let in_range = interval.months.iter().any(|m| {
            if let Some((start, end)) = m.split_once(':') {
                let s: i32 = start.parse().unwrap_or(1);
                let e: i32 = end.parse().unwrap_or(12);
                month >= s && month <= e
            } else {
                m.parse::<i32>().map(|v| v == month).unwrap_or(false)
            }
        });
        if !in_range {
            return false;
        }
    }

    // Check time ranges (minutes since midnight)
    if !interval.times.is_empty() {
        let minutes_now = now.hour() as i32 * 60 + now.minute() as i32;
        let in_range = interval.times.iter().any(|t| {
            minutes_now >= t.start_minute && minutes_now <= t.end_minute
        });
        if !in_range {
            return false;
        }
    }

    true
}
