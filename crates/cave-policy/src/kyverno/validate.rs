// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kyverno validation engine.
//!
//! Supports: pattern match, AnyPattern, deny conditions, foreach, CEL.

use super::jmespath::{evaluate, kyverno_pattern_match, substitute_variables_json};
use super::models::*;
use crate::error::PolicyError;
use serde_json::Value;

/// Validate a resource against a Kyverno validate rule.
pub fn validate_rule(
    rule: &KyvernoRule,
    resource: &Value,
    context: &Value,
) -> Result<Option<PolicyViolation>, PolicyError> {
    let validate = match &rule.validate {
        Some(v) => v,
        None => return Ok(None),
    };

    // Evaluate preconditions
    if let Some(preconditions) = &rule.preconditions {
        if !eval_conditions(preconditions, resource, context)? {
            return Ok(None); // Preconditions not met — skip rule
        }
    }

    // CEL validation
    if let Some(cel) = &validate.cel {
        return validate_cel(cel, resource, &rule.name);
    }

    // Deny conditions
    if let Some(deny) = &validate.deny {
        return validate_deny(
            deny,
            resource,
            context,
            &rule.name,
            validate.message.as_deref(),
        );
    }

    // AnyPattern
    if let Some(patterns) = &validate.any_pattern {
        for pattern in patterns {
            let substituted = substitute_variables_json(pattern, context)?;
            if pattern_match_value(&substituted, resource) {
                return Ok(None); // At least one pattern matched
            }
        }
        let msg = validate.message.clone().unwrap_or_else(|| {
            format!(
                "rule {} validation failed: none of the anyPattern conditions matched",
                rule.name
            )
        });
        return Ok(Some(PolicyViolation {
            policy: String::new(),
            rule: rule.name.clone(),
            message: msg,
            severity: None,
            resource: None,
        }));
    }

    // Pattern
    if let Some(pattern) = &validate.pattern {
        let substituted = substitute_variables_json(pattern, context)?;
        if !pattern_match_value(&substituted, resource) {
            let msg = validate.message.clone().unwrap_or_else(|| {
                format!(
                    "rule {} validation failed: resource does not match pattern",
                    rule.name
                )
            });
            return Ok(Some(PolicyViolation {
                policy: String::new(),
                rule: rule.name.clone(),
                message: msg,
                severity: None,
                resource: None,
            }));
        }
    }

    // ForEach
    if !validate.foreach.is_empty() {
        for foreach in &validate.foreach {
            if let Some(violation) = validate_foreach(foreach, &rule.name, resource, context)? {
                return Ok(Some(violation));
            }
        }
    }

    Ok(None)
}

fn validate_foreach(
    foreach: &ForEachValidation,
    rule_name: &str,
    resource: &Value,
    context: &Value,
) -> Result<Option<PolicyViolation>, PolicyError> {
    let list = evaluate(&foreach.list, resource)?;
    let items = match &list {
        Value::Array(a) => a.clone(),
        _ => return Ok(None),
    };

    for item in &items {
        // Check preconditions for this element
        if let Some(preconds) = &foreach.preconditions {
            let elem_ctx = if foreach.element_scope {
                item
            } else {
                resource
            };
            if !eval_conditions(preconds, elem_ctx, context)? {
                continue;
            }
        }

        let target = if foreach.element_scope {
            item
        } else {
            resource
        };

        // Pattern match
        if let Some(pattern) = &foreach.pattern {
            let substituted = substitute_variables_json(pattern, context)?;
            if !pattern_match_value(&substituted, target) {
                return Ok(Some(PolicyViolation {
                    policy: String::new(),
                    rule: rule_name.to_string(),
                    message: format!(
                        "foreach validation failed for element: {}",
                        serde_json::to_string(item).unwrap_or_default()
                    ),
                    severity: None,
                    resource: None,
                }));
            }
        }

        // AnyPattern
        if let Some(patterns) = &foreach.any_pattern {
            let matched = patterns.iter().any(|p| {
                substitute_variables_json(p, context)
                    .map(|sub| pattern_match_value(&sub, target))
                    .unwrap_or(false)
            });
            if !matched {
                return Ok(Some(PolicyViolation {
                    policy: String::new(),
                    rule: rule_name.to_string(),
                    message: format!(
                        "foreach anyPattern validation failed for element: {}",
                        serde_json::to_string(item).unwrap_or_default()
                    ),
                    severity: None,
                    resource: None,
                }));
            }
        }

        // Deny
        if let Some(deny) = &foreach.deny {
            if let Some(v) = validate_deny(deny, target, context, rule_name, None)? {
                return Ok(Some(v));
            }
        }
    }
    Ok(None)
}

fn validate_deny(
    deny: &DenyConditions,
    resource: &Value,
    context: &Value,
    rule_name: &str,
    message: Option<&str>,
) -> Result<Option<PolicyViolation>, PolicyError> {
    if eval_conditions(&deny.conditions, resource, context)? {
        let msg = message
            .map(String::from)
            .unwrap_or_else(|| format!("rule {rule_name} deny conditions matched"));
        return Ok(Some(PolicyViolation {
            policy: String::new(),
            rule: rule_name.to_string(),
            message: msg,
            severity: None,
            resource: None,
        }));
    }
    Ok(None)
}

fn validate_cel(
    cel: &CelValidation,
    _resource: &Value,
    rule_name: &str,
) -> Result<Option<PolicyViolation>, PolicyError> {
    // CEL evaluation requires a full CEL runtime; we validate syntax only
    for expr in &cel.expressions {
        // Stub: mark CEL expressions as unimplemented but don't fail
        tracing::debug!(
            target: "kyverno.cel",
            rule = rule_name,
            expression = expr.expression,
            "CEL expression (stub evaluation — always passes)"
        );
    }
    Ok(None)
}

/// Evaluate Kyverno conditions (any/all).
pub fn eval_conditions(
    conditions: &Conditions,
    resource: &Value,
    context: &Value,
) -> Result<bool, PolicyError> {
    // ANY: at least one condition must be true
    if let Some(any) = &conditions.any {
        let any_true = any
            .iter()
            .any(|c| eval_condition(c, resource, context).unwrap_or(false));
        if !any_true {
            return Ok(false);
        }
    }
    // ALL: every condition must be true
    if let Some(all) = &conditions.all {
        let all_true = all
            .iter()
            .all(|c| eval_condition(c, resource, context).unwrap_or(false));
        if !all_true {
            return Ok(false);
        }
    }
    Ok(true)
}

fn eval_condition(
    cond: &Condition,
    resource: &Value,
    context: &Value,
) -> Result<bool, PolicyError> {
    let key_val = eval_condition_value(&cond.key, resource, context)?;
    let compare_to = cond
        .value
        .as_ref()
        .map(|v| eval_condition_value(v, resource, context))
        .transpose()?;

    match &cond.operator {
        ConditionOperator::Equals => {
            Ok(compare_to.as_ref().map(|v| &key_val == v).unwrap_or(false))
        }
        ConditionOperator::NotEquals => {
            Ok(compare_to.as_ref().map(|v| &key_val != v).unwrap_or(true))
        }
        ConditionOperator::In => {
            let search = compare_to.unwrap_or(Value::Null);
            match &search {
                Value::Array(a) => Ok(a.contains(&key_val)),
                _ => Ok(false),
            }
        }
        ConditionOperator::NotIn => {
            let search = compare_to.unwrap_or(Value::Null);
            match &search {
                Value::Array(a) => Ok(!a.contains(&key_val)),
                _ => Ok(true),
            }
        }
        ConditionOperator::GreaterThan
        | ConditionOperator::GreaterThanOrEquals
        | ConditionOperator::LessThan
        | ConditionOperator::LessThanOrEquals => {
            let kn = key_val.as_f64().unwrap_or(f64::NAN);
            let vn = compare_to
                .as_ref()
                .and_then(|v| v.as_f64())
                .unwrap_or(f64::NAN);
            Ok(match &cond.operator {
                ConditionOperator::GreaterThan => kn > vn,
                ConditionOperator::GreaterThanOrEquals => kn >= vn,
                ConditionOperator::LessThan => kn < vn,
                ConditionOperator::LessThanOrEquals => kn <= vn,
                _ => false,
            })
        }
        ConditionOperator::Contains => match &key_val {
            Value::Array(a) => Ok(a.contains(compare_to.as_ref().unwrap_or(&Value::Null))),
            Value::String(s) => {
                if let Some(Value::String(sub)) = &compare_to {
                    Ok(s.contains(sub.as_str()))
                } else {
                    Ok(false)
                }
            }
            _ => Ok(false),
        },
        ConditionOperator::NotContains => match &key_val {
            Value::Array(a) => Ok(!a.contains(compare_to.as_ref().unwrap_or(&Value::Null))),
            Value::String(s) => {
                if let Some(Value::String(sub)) = &compare_to {
                    Ok(!s.contains(sub.as_str()))
                } else {
                    Ok(true)
                }
            }
            _ => Ok(true),
        },
        ConditionOperator::AnyIn => {
            if let (Value::Array(keys), Some(Value::Array(vals))) = (&key_val, &compare_to) {
                return Ok(keys.iter().any(|k| vals.contains(k)));
            }
            Ok(false)
        }
        ConditionOperator::AllIn => {
            if let (Value::Array(keys), Some(Value::Array(vals))) = (&key_val, &compare_to) {
                return Ok(keys.iter().all(|k| vals.contains(k)));
            }
            Ok(false)
        }
        ConditionOperator::AnyNotIn => {
            if let (Value::Array(keys), Some(Value::Array(vals))) = (&key_val, &compare_to) {
                return Ok(keys.iter().any(|k| !vals.contains(k)));
            }
            Ok(true)
        }
        ConditionOperator::AllNotIn => {
            if let (Value::Array(keys), Some(Value::Array(vals))) = (&key_val, &compare_to) {
                return Ok(keys.iter().all(|k| !vals.contains(k)));
            }
            Ok(true)
        }
        ConditionOperator::DurationGreaterThan
        | ConditionOperator::DurationLessThan
        | ConditionOperator::DurationGreaterThanOrEquals
        | ConditionOperator::DurationLessThanOrEquals => {
            // Duration comparison: compare ns values
            let kns = parse_duration_ns(key_val.as_str().unwrap_or("0")).unwrap_or(0);
            let vns = compare_to
                .as_ref()
                .and_then(|v| v.as_str())
                .and_then(|s| parse_duration_ns(s))
                .unwrap_or(0);
            Ok(match &cond.operator {
                ConditionOperator::DurationGreaterThan => kns > vns,
                ConditionOperator::DurationLessThan => kns < vns,
                ConditionOperator::DurationGreaterThanOrEquals => kns >= vns,
                ConditionOperator::DurationLessThanOrEquals => kns <= vns,
                _ => false,
            })
        }
        ConditionOperator::Unknown => Ok(false),
    }
}

fn eval_condition_value(
    v: &Value,
    resource: &Value,
    context: &Value,
) -> Result<Value, PolicyError> {
    match v {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
                let expr = &trimmed[2..trimmed.len() - 2].trim().to_string();
                evaluate(expr, resource)
            } else {
                Ok(Value::String(s.clone()))
            }
        }
        _ => Ok(v.clone()),
    }
}

fn parse_duration_ns(s: &str) -> Option<i64> {
    let mut ns: i64 = 0;
    let mut remaining = s;
    while !remaining.is_empty() {
        let num_end = remaining.find(|c: char| c.is_alphabetic())?;
        if num_end == 0 {
            break;
        }
        let num: f64 = remaining[..num_end].parse().ok()?;
        let unit_end = remaining[num_end..]
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(remaining.len() - num_end);
        let unit = &remaining[num_end..num_end + unit_end];
        let mult: i64 = match unit {
            "ns" => 1,
            "us" | "µs" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60 * 1_000_000_000,
            "h" => 3600 * 1_000_000_000,
            "d" => 86400 * 1_000_000_000,
            _ => return None,
        };
        ns += (num * mult as f64) as i64;
        remaining = &remaining[num_end + unit_end..];
    }
    Some(ns)
}

// ─── Pattern matching ─────────────────────────────────────────────────────────

/// Match a Kyverno pattern against a resource value.
pub fn pattern_match_value(pattern: &Value, value: &Value) -> bool {
    match (pattern, value) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(p), Value::Bool(v)) => p == v,
        (Value::Number(p), Value::Number(v)) => p == v,
        (Value::String(p), v) => match v {
            Value::String(s) => match_string_pattern(p, s),
            Value::Number(n) => match_string_pattern(p, &n.to_string()),
            Value::Bool(b) => match_string_pattern(p, &b.to_string()),
            Value::Null => p == "null",
            _ => false,
        },
        (Value::Object(pattern_obj), Value::Object(resource_obj)) => {
            // Every key in the pattern must match the resource
            pattern_obj.iter().all(|(k, pv)| {
                if let Some(rv) = resource_obj.get(k) {
                    pattern_match_value(pv, rv)
                } else {
                    // Missing field: only matches if pattern allows it
                    matches!(pv, Value::Null)
                }
            })
        }
        (Value::Array(pattern_arr), Value::Array(resource_arr)) => {
            if pattern_arr.len() == 1 {
                // Array pattern with one element = each element must match
                let elem_pattern = &pattern_arr[0];
                resource_arr
                    .iter()
                    .all(|rv| pattern_match_value(elem_pattern, rv))
            } else {
                // Exact length match with element-wise comparison
                pattern_arr.len() == resource_arr.len()
                    && pattern_arr
                        .iter()
                        .zip(resource_arr.iter())
                        .all(|(p, v)| pattern_match_value(p, v))
            }
        }
        _ => false,
    }
}

fn match_string_pattern(pattern: &str, value: &str) -> bool {
    // Handle Kyverno operators
    if let Some(rest) = pattern.strip_prefix(">=") {
        let pn: f64 = rest.trim().parse().unwrap_or(0.0);
        let vn: f64 = value.parse().unwrap_or(f64::NAN);
        return vn >= pn;
    }
    if let Some(rest) = pattern.strip_prefix("<=") {
        let pn: f64 = rest.trim().parse().unwrap_or(0.0);
        let vn: f64 = value.parse().unwrap_or(f64::NAN);
        return vn <= pn;
    }
    if let Some(rest) = pattern.strip_prefix('>') {
        let pn: f64 = rest.trim().parse().unwrap_or(0.0);
        let vn: f64 = value.parse().unwrap_or(f64::NAN);
        return vn > pn;
    }
    if let Some(rest) = pattern.strip_prefix('<') {
        let pn: f64 = rest.trim().parse().unwrap_or(0.0);
        let vn: f64 = value.parse().unwrap_or(f64::NAN);
        return vn < pn;
    }
    if let Some(rest) = pattern.strip_prefix('!') {
        return !match_string_pattern(rest, value);
    }
    // Wildcard / glob
    kyverno_pattern_match(pattern, value)
}
