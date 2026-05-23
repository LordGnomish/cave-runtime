// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Component-age policy evaluator.  Mirrors `ComponentAgePolicyEvaluator`.

use super::engine::{PolicyCondition, PolicyOperator, PolicyResult, Subject, ViolationKind};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// Parses an ISO-8601 duration in the upstream subset: `P[n]D` or `P[n]Y[n]M[n]D`.
pub fn parse_iso_duration(s: &str) -> Option<Duration> {
    let mut days_total: i64 = 0;
    let rest = s.strip_prefix('P')?;
    let mut buf = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() {
            buf.push(c);
            continue;
        }
        let n: i64 = buf.parse().ok()?;
        buf.clear();
        match c {
            'Y' => days_total += n * 365,
            'M' => days_total += n * 30,
            'W' => days_total += n * 7,
            'D' => days_total += n,
            _ => return None,
        }
    }
    if !buf.is_empty() {
        return None;
    }
    Some(Duration::days(days_total))
}

/// Returns one violation when the component's published date is older than the
/// configured duration *and* the operator demands it.
pub fn evaluate_age(
    policy: Uuid,
    conditions: &[PolicyCondition],
    component: Uuid,
    component_published: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Vec<PolicyResult> {
    let mut out = Vec::new();
    let Some(pub_at) = component_published else {
        return out;
    };
    let age = now - pub_at;
    for (i, c) in conditions.iter().enumerate() {
        if c.subject != Subject::ComponentAge {
            continue;
        }
        let Some(threshold) = parse_iso_duration(&c.value) else {
            continue;
        };
        let hit = match c.operator {
            PolicyOperator::NumericGreaterThanOrEqual => age >= threshold,
            PolicyOperator::NumericGreaterThan => age > threshold,
            PolicyOperator::NumericLessThan => age < threshold,
            PolicyOperator::NumericLessThanOrEqual => age <= threshold,
            _ => false,
        };
        if hit {
            out.push(PolicyResult {
                policy,
                component,
                kind: ViolationKind::Operational,
                condition_index: i,
                reason: format!("age >= {} (component {} days)", c.value, age.num_days()),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iso_durations() {
        assert_eq!(parse_iso_duration("P30D"), Some(Duration::days(30)));
        assert_eq!(parse_iso_duration("P1Y"), Some(Duration::days(365)));
        assert_eq!(parse_iso_duration("P2W"), Some(Duration::days(14)));
        assert_eq!(parse_iso_duration("P1Y6M"), Some(Duration::days(365 + 180)));
    }

    #[test]
    fn rejects_malformed_duration() {
        assert!(parse_iso_duration("30D").is_none());
        assert!(parse_iso_duration("PX").is_none());
        assert!(parse_iso_duration("P30").is_none());
    }

    #[test]
    fn old_component_violates() {
        let pub_at = Utc::now() - Duration::days(400);
        let cond = PolicyCondition {
            subject: Subject::ComponentAge,
            operator: PolicyOperator::NumericGreaterThanOrEqual,
            value: "P1Y".into(),
        };
        let v = evaluate_age(Uuid::new_v4(), &[cond], Uuid::new_v4(), Some(pub_at), Utc::now());
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn young_component_no_violation() {
        let pub_at = Utc::now() - Duration::days(7);
        let cond = PolicyCondition {
            subject: Subject::ComponentAge,
            operator: PolicyOperator::NumericGreaterThanOrEqual,
            value: "P30D".into(),
        };
        let v = evaluate_age(Uuid::new_v4(), &[cond], Uuid::new_v4(), Some(pub_at), Utc::now());
        assert!(v.is_empty());
    }

    #[test]
    fn missing_published_no_violation() {
        let cond = PolicyCondition {
            subject: Subject::ComponentAge,
            operator: PolicyOperator::NumericGreaterThanOrEqual,
            value: "P30D".into(),
        };
        let v = evaluate_age(Uuid::new_v4(), &[cond], Uuid::new_v4(), None, Utc::now());
        assert!(v.is_empty());
    }
}
