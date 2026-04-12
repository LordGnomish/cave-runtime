//! Alert evaluation engine for CAVE Dashboard.

use crate::models::{AlertRule, AlertState};

/// Result of evaluating an alert rule against a single value.
#[derive(Debug, Clone)]
pub struct AlertEvalResult {
    pub rule_id: u64,
    pub rule_name: String,
    pub state: AlertState,
    pub value: f64,
    pub message: String,
}

/// Evaluate a single alert rule against the provided metric value.
///
/// The first condition's evaluator determines the firing state.
/// Multiple conditions are ANDed together.
pub fn evaluate_alert(rule: &AlertRule, value: f64) -> AlertEvalResult {
    if rule.conditions.is_empty() {
        return AlertEvalResult {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            state: AlertState::NoData,
            value,
            message: "No conditions defined".to_string(),
        };
    }

    let mut all_firing = true;

    for condition in &rule.conditions {
        let firing = eval_condition(&condition.evaluator.evaluator_type, &condition.evaluator.params, value);
        // "and" operator (default): all conditions must fire
        if condition.operator.op_type == "or" {
            if firing {
                return build_result(rule, AlertState::Alerting, value);
            }
            // keep all_firing = false for OR logic unless any fires
        } else {
            // AND logic
            if !firing {
                all_firing = false;
                break;
            }
        }
    }

    let state = if all_firing { AlertState::Alerting } else { AlertState::Ok };
    build_result(rule, state, value)
}

fn build_result(rule: &AlertRule, state: AlertState, value: f64) -> AlertEvalResult {
    let message = match &state {
        AlertState::Alerting => format!("{}: value {:.4} triggered alert", rule.name, value),
        AlertState::Ok => format!("{}: value {:.4} is within normal range", rule.name, value),
        AlertState::NoData => format!("{}: no data", rule.name),
        _ => rule.message.clone(),
    };
    AlertEvalResult { rule_id: rule.id, rule_name: rule.name.clone(), state, value, message }
}

fn eval_condition(evaluator_type: &str, params: &[f64], value: f64) -> bool {
    match evaluator_type {
        "gt" => params.first().map(|&t| value > t).unwrap_or(false),
        "lt" => params.first().map(|&t| value < t).unwrap_or(false),
        "eq" => params.first().map(|&t| (value - t).abs() < f64::EPSILON).unwrap_or(false),
        "gte" => params.first().map(|&t| value >= t).unwrap_or(false),
        "lte" => params.first().map(|&t| value <= t).unwrap_or(false),
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
        "no_value" => value.is_nan(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AlertCondition, AlertEvaluator, AlertOperator, AlertQuery, AlertReducer};

    fn make_rule(evaluator_type: &str, threshold: f64) -> AlertRule {
        AlertRule {
            id: 1,
            name: "test-rule".to_string(),
            message: "Alert!".to_string(),
            frequency: "10s".to_string(),
            for_duration: "0s".to_string(),
            conditions: vec![AlertCondition {
                ref_id: "A".to_string(),
                evaluator: AlertEvaluator {
                    evaluator_type: evaluator_type.to_string(),
                    params: vec![threshold],
                },
                operator: AlertOperator { op_type: "and".to_string() },
                reducer: AlertReducer { reducer_type: "avg".to_string() },
                query: AlertQuery { params: vec!["A".to_string(), "5m".to_string(), "now".to_string()] },
            }],
            notifications: vec![],
            state: AlertState::Ok,
            no_data_state: Default::default(),
            exec_err_state: Default::default(),
        }
    }

    #[test]
    fn test_alert_gt_firing() {
        let rule = make_rule("gt", 90.0);
        let result = evaluate_alert(&rule, 95.0);
        assert_eq!(result.state, AlertState::Alerting);
    }

    #[test]
    fn test_alert_gt_ok() {
        let rule = make_rule("gt", 90.0);
        let result = evaluate_alert(&rule, 80.0);
        assert_eq!(result.state, AlertState::Ok);
    }

    #[test]
    fn test_alert_lt_firing() {
        let rule = make_rule("lt", 10.0);
        let result = evaluate_alert(&rule, 5.0);
        assert_eq!(result.state, AlertState::Alerting);
    }

    #[test]
    fn test_alert_no_conditions() {
        let rule = AlertRule { id: 2, name: "empty".to_string(), conditions: vec![], ..Default::default() };
        let result = evaluate_alert(&rule, 0.0);
        assert_eq!(result.state, AlertState::NoData);
    }

    #[test]
    fn test_alert_within_range() {
        let mut rule = AlertRule {
            id: 3,
            name: "range-rule".to_string(),
            conditions: vec![AlertCondition {
                ref_id: "A".to_string(),
                evaluator: AlertEvaluator {
                    evaluator_type: "within_range".to_string(),
                    params: vec![50.0, 100.0],
                },
                operator: AlertOperator { op_type: "and".to_string() },
                reducer: AlertReducer { reducer_type: "avg".to_string() },
                query: AlertQuery { params: vec![] },
            }],
            ..Default::default()
        };
        assert_eq!(evaluate_alert(&rule, 75.0).state, AlertState::Alerting);
        rule.conditions[0].evaluator.evaluator_type = "outside_range".to_string();
        assert_eq!(evaluate_alert(&rule, 75.0).state, AlertState::Ok);
        assert_eq!(evaluate_alert(&rule, 25.0).state, AlertState::Alerting);
    }
}
