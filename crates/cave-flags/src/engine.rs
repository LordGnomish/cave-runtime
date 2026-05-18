// SPDX-License-Identifier: AGPL-3.0-or-later
//! Unleash-compatible feature flag evaluation engine.
//!
//! Evaluates all activation strategies, constraints, segments, and variants
//! exactly as specified in the Unleash client specification.

use crate::models::{
    Constraint, ConstraintOperator, EvaluatedVariant, FeatureFlag, FeatureStrategy, Segment,
    UnleashContext, Variant, WeightType,
};
use std::collections::HashMap;
use tracing::trace;

// ── Murmur3 hash (seed 0, 32-bit) ────────────────────────────────────────────
// Matches Unleash's JS implementation exactly.

fn murmur3_32(data: &[u8]) -> u32 {
    const C1: u32 = 0xcc9e2d51;
    const C2: u32 = 0x1b873593;
    let mut hash: u32 = 0;
    let nblocks = data.len() / 4;

    for i in 0..nblocks {
        let mut k = u32::from_le_bytes([
            data[i * 4],
            data[i * 4 + 1],
            data[i * 4 + 2],
            data[i * 4 + 3],
        ]);
        k = k.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        hash ^= k;
        hash = hash.rotate_left(13);
        hash = hash.wrapping_mul(5).wrapping_add(0xe6546b64);
    }

    let tail = &data[nblocks * 4..];
    let mut k1: u32 = 0;
    match tail.len() {
        3 => {
            k1 ^= (tail[2] as u32) << 16;
            k1 ^= (tail[1] as u32) << 8;
            k1 ^= tail[0] as u32;
            k1 = k1.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
            hash ^= k1;
        }
        2 => {
            k1 ^= (tail[1] as u32) << 8;
            k1 ^= tail[0] as u32;
            k1 = k1.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
            hash ^= k1;
        }
        1 => {
            k1 ^= tail[0] as u32;
            k1 = k1.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
            hash ^= k1;
        }
        _ => {}
    }

    hash ^= data.len() as u32;
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85ebca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2ae35);
    hash ^= hash >> 16;
    hash
}

/// Returns 1..=100 (gradual rollout percentage check).
pub fn normalized_value_100(id: &str, group_id: &str) -> u32 {
    let key = format!("{}:{}", group_id, id);
    let h = murmur3_32(key.as_bytes()) % 100;
    if h == 0 { 100 } else { h }
}

/// Returns 1..=1000 (variant weight selection).
pub fn normalized_value_1000(id: &str, group_id: &str) -> u32 {
    let key = format!("{}:{}", group_id, id);
    let h = murmur3_32(key.as_bytes()) % 1000;
    if h == 0 { 1000 } else { h }
}

// ── Top-level evaluation ──────────────────────────────────────────────────────

pub struct EvalResult {
    pub enabled: bool,
    pub variant: EvaluatedVariant,
}

pub fn evaluate_flag(
    flag: &FeatureFlag,
    env: &str,
    ctx: &UnleashContext,
    segments: &HashMap<i64, &Segment>,
) -> EvalResult {
    if matches!(flag.feature_type, crate::models::FeatureType::KillSwitch) && !flag.enabled {
        return disabled_result();
    }

    let env_cfg = flag.environments.iter().find(|e| e.name == env);
    let env_enabled = env_cfg.map(|e| e.enabled).unwrap_or(false);

    if !flag.enabled || !env_enabled {
        return disabled_result();
    }

    let strategies = env_cfg
        .map(|e| e.strategies.as_slice())
        .filter(|s| !s.is_empty())
        .unwrap_or(&flag.strategies);

    let enabled = if strategies.is_empty() {
        true
    } else {
        strategies
            .iter()
            .filter(|s| !s.disabled)
            .any(|s| evaluate_strategy(s, ctx, segments))
    };

    if !enabled {
        return disabled_result();
    }

    let variants = env_cfg
        .map(|e| e.variants.as_slice())
        .filter(|v| !v.is_empty())
        .unwrap_or(&flag.variants);

    let variant = select_variant(variants, &flag.name, ctx, true);
    EvalResult { enabled, variant }
}

fn disabled_result() -> EvalResult {
    EvalResult {
        enabled: false,
        variant: EvaluatedVariant::disabled(),
    }
}

pub fn evaluate_all(
    flags: &[FeatureFlag],
    env: &str,
    ctx: &UnleashContext,
    segments: &[Segment],
) -> Vec<(String, bool, EvaluatedVariant)> {
    let seg_map: HashMap<i64, &Segment> = segments.iter().map(|s| (s.id, s)).collect();
    flags
        .iter()
        .map(|flag| {
            let r = evaluate_flag(flag, env, ctx, &seg_map);
            (flag.name.clone(), r.enabled, r.variant)
        })
        .collect()
}

// ── Strategy evaluation ───────────────────────────────────────────────────────

fn evaluate_strategy(
    strategy: &FeatureStrategy,
    ctx: &UnleashContext,
    segments: &HashMap<i64, &Segment>,
) -> bool {
    for seg_id in &strategy.segments {
        if let Some(seg) = segments.get(seg_id) {
            if !evaluate_constraints(&seg.constraints, ctx) {
                trace!(strategy = %strategy.name, segment = seg_id, "segment constraint failed");
                return false;
            }
        }
    }

    if !evaluate_constraints(&strategy.constraints, ctx) {
        trace!(strategy = %strategy.name, "inline constraint failed");
        return false;
    }

    match strategy.name.as_str() {
        "default" => true,

        "userWithId" => {
            let user_ids = strategy.parameters.get("userIds").map(|s| s.as_str()).unwrap_or("");
            ctx.user_id
                .as_deref()
                .map(|uid| user_ids.split(',').map(|s| s.trim()).any(|id| id == uid))
                .unwrap_or(false)
        }

        "remoteAddress" => {
            let ips = strategy.parameters.get("IPs").map(|s| s.as_str()).unwrap_or("");
            ctx.remote_address
                .as_deref()
                .map(|addr| ips.split(',').map(|s| s.trim()).any(|ip| ip == addr))
                .unwrap_or(false)
        }

        "applicationHostname" => {
            let hosts = strategy
                .parameters
                .get("hostNames")
                .map(|s| s.as_str())
                .unwrap_or("");
            ctx.properties
                .get("hostname")
                .map(|h| hosts.split(',').map(|s| s.trim()).any(|host| host == h))
                .unwrap_or(false)
        }

        "gradualRolloutRandom" => {
            let pct: u32 = strategy
                .parameters
                .get("percentage")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let random_val = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
                % 100) + 1;
            random_val <= pct
        }

        "gradualRolloutSessionId" => {
            let pct: u32 = strategy
                .parameters
                .get("percentage")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let group_id = strategy
                .parameters
                .get("groupId")
                .map(|s| s.as_str())
                .unwrap_or("default");
            ctx.session_id
                .as_deref()
                .map(|sid| normalized_value_100(sid, group_id) <= pct)
                .unwrap_or(false)
        }

        "gradualRolloutUserId" => {
            let pct: u32 = strategy
                .parameters
                .get("percentage")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let group_id = strategy
                .parameters
                .get("groupId")
                .map(|s| s.as_str())
                .unwrap_or("default");
            ctx.user_id
                .as_deref()
                .map(|uid| normalized_value_100(uid, group_id) <= pct)
                .unwrap_or(false)
        }

        "flexibleRollout" => {
            let rollout: u32 = strategy
                .parameters
                .get("rollout")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let stickiness = strategy
                .parameters
                .get("stickiness")
                .map(|s| s.as_str())
                .unwrap_or("default");
            let group_id = strategy
                .parameters
                .get("groupId")
                .map(|s| s.as_str())
                .unwrap_or("default");

            match resolve_stickiness(stickiness, ctx) {
                Some(val) => normalized_value_100(&val, group_id) <= rollout,
                None => {
                    let r = (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_nanos()
                        % 100) + 1;
                    r <= rollout
                }
            }
        }

        // Custom strategies: client-side eval (server falls back to enabled)
        _ => {
            trace!(strategy = %strategy.name, "custom strategy — defaulting to enabled");
            true
        }
    }
}

fn resolve_stickiness(stickiness: &str, ctx: &UnleashContext) -> Option<String> {
    match stickiness {
        "default" | "userId" => ctx.user_id.clone().or_else(|| ctx.session_id.clone()),
        "sessionId" => ctx.session_id.clone(),
        "random" => None,
        other => ctx.properties.get(other).cloned(),
    }
}

// ── Constraint evaluation ─────────────────────────────────────────────────────

pub fn evaluate_constraints(constraints: &[Constraint], ctx: &UnleashContext) -> bool {
    constraints.iter().all(|c| evaluate_constraint(c, ctx))
}

fn evaluate_constraint(c: &Constraint, ctx: &UnleashContext) -> bool {
    let ctx_value = ctx.get_field(&c.context_name);
    let result = check_constraint(c, ctx_value.as_deref());
    if c.inverted { !result } else { result }
}

fn check_constraint(c: &Constraint, value: Option<&str>) -> bool {
    match &c.operator {
        ConstraintOperator::In => value.map(|v| {
            if c.case_insensitive {
                let vl = v.to_lowercase();
                c.values.iter().any(|s| s.to_lowercase() == vl)
            } else {
                c.values.iter().any(|s| s == v)
            }
        }).unwrap_or(false),

        ConstraintOperator::NotIn => value.map(|v| {
            if c.case_insensitive {
                let vl = v.to_lowercase();
                !c.values.iter().any(|s| s.to_lowercase() == vl)
            } else {
                !c.values.iter().any(|s| s == v)
            }
        }).unwrap_or(true),

        ConstraintOperator::StrStartsWith => value.map(|v| {
            c.values.iter().any(|p| {
                if c.case_insensitive {
                    v.to_lowercase().starts_with(&p.to_lowercase())
                } else {
                    v.starts_with(p.as_str())
                }
            })
        }).unwrap_or(false),

        ConstraintOperator::StrEndsWith => value.map(|v| {
            c.values.iter().any(|p| {
                if c.case_insensitive {
                    v.to_lowercase().ends_with(&p.to_lowercase())
                } else {
                    v.ends_with(p.as_str())
                }
            })
        }).unwrap_or(false),

        ConstraintOperator::StrContains => value.map(|v| {
            c.values.iter().any(|p| {
                if c.case_insensitive {
                    v.to_lowercase().contains(&p.to_lowercase())
                } else {
                    v.contains(p.as_str())
                }
            })
        }).unwrap_or(false),

        ConstraintOperator::NumEq => {
            compare_f64(value, c.value.as_deref(), |a, b| (a - b).abs() < f64::EPSILON)
        }
        ConstraintOperator::NumGt => compare_f64(value, c.value.as_deref(), |a, b| a > b),
        ConstraintOperator::NumGte => compare_f64(value, c.value.as_deref(), |a, b| a >= b),
        ConstraintOperator::NumLt => compare_f64(value, c.value.as_deref(), |a, b| a < b),
        ConstraintOperator::NumLte => compare_f64(value, c.value.as_deref(), |a, b| a <= b),

        ConstraintOperator::DateBefore => compare_date(value, c.value.as_deref(), |a, b| a < b),
        ConstraintOperator::DateAfter => compare_date(value, c.value.as_deref(), |a, b| a > b),

        ConstraintOperator::SemverEq => {
            compare_semver(value, c.value.as_deref(), std::cmp::Ordering::Equal)
        }
        ConstraintOperator::SemverGt => {
            compare_semver(value, c.value.as_deref(), std::cmp::Ordering::Greater)
        }
        ConstraintOperator::SemverLt => {
            compare_semver(value, c.value.as_deref(), std::cmp::Ordering::Less)
        }
    }
}

fn compare_f64(a: Option<&str>, b: Option<&str>, pred: impl Fn(f64, f64) -> bool) -> bool {
    match (
        a.and_then(|v| v.parse::<f64>().ok()),
        b.and_then(|v| v.parse::<f64>().ok()),
    ) {
        (Some(a), Some(b)) => pred(a, b),
        _ => false,
    }
}

fn compare_date(
    a: Option<&str>,
    b: Option<&str>,
    pred: impl Fn(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) -> bool,
) -> bool {
    let parse = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .ok()
    };
    match (a.and_then(parse), b.and_then(parse)) {
        (Some(a), Some(b)) => pred(a, b),
        _ => false,
    }
}

fn compare_semver(
    ctx_val: Option<&str>,
    constraint_val: Option<&str>,
    expected_ord: std::cmp::Ordering,
) -> bool {
    fn parse(s: &str) -> Option<(u64, u64, u64)> {
        let s = s.trim_start_matches('v');
        let mut parts = s.splitn(3, '.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts
            .next()
            .unwrap_or("0")
            .split('-')
            .next()
            .unwrap_or("0")
            .parse()
            .ok()?;
        Some((major, minor, patch))
    }
    match (ctx_val.and_then(parse), constraint_val.and_then(parse)) {
        (Some(a), Some(b)) => a.cmp(&b) == expected_ord,
        _ => false,
    }
}

// ── Variant selection ─────────────────────────────────────────────────────────

pub fn select_variant(
    variants: &[Variant],
    feature_name: &str,
    ctx: &UnleashContext,
    feature_enabled: bool,
) -> EvaluatedVariant {
    if variants.is_empty() || variants.iter().all(|v| v.weight == 0) {
        return EvaluatedVariant {
            name: "disabled".to_string(),
            enabled: false,
            payload: None,
            feature_enabled,
        };
    }

    // Override check
    for variant in variants {
        for ov in &variant.overrides {
            if let Some(ctx_val) = ctx.get_field(&ov.context_name) {
                if ov.values.iter().any(|v| v == &ctx_val) {
                    return EvaluatedVariant {
                        name: variant.name.clone(),
                        enabled: true,
                        payload: variant.payload.clone(),
                        feature_enabled,
                    };
                }
            }
        }
    }

    // Stickiness
    let stickiness = variants.first().map(|v| v.stickiness.as_str()).unwrap_or("default");
    let norm = match resolve_stickiness(stickiness, ctx) {
        Some(val) => normalized_value_1000(&val, feature_name),
        None => {
            (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
                % 1000) + 1
        }
    };

    let mut cumulative = 0u32;
    for variant in variants {
        cumulative += variant.weight;
        if norm <= cumulative {
            return EvaluatedVariant {
                name: variant.name.clone(),
                enabled: true,
                payload: variant.payload.clone(),
                feature_enabled,
            };
        }
    }

    EvaluatedVariant::disabled()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_strategy(name: &str, params: Vec<(&str, &str)>) -> FeatureStrategy {
        FeatureStrategy {
            id: Uuid::new_v4(),
            name: name.to_string(),
            parameters: params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            constraints: vec![],
            segments: vec![],
            sort_order: 0,
            disabled: false,
            variants: vec![],
        }
    }

    fn make_flag(name: &str, strategies: Vec<FeatureStrategy>) -> FeatureFlag {
        FeatureFlag {
            name: name.to_string(),
            feature_type: FeatureType::Release,
            description: String::new(),
            enabled: true,
            stale: false,
            impression_data: false,
            project: "default".to_string(),
            created_at: Utc::now(),
            last_seen_at: None,
            strategies: strategies.clone(),
            variants: vec![],
            environments: vec![FeatureEnvironment {
                name: "production".to_string(),
                enabled: true,
                strategies,
                variants: vec![],
            }],
            tags: vec![],
        }
    }

    fn ctx(user_id: &str) -> UnleashContext {
        UnleashContext {
            user_id: Some(user_id.to_string()),
            session_id: Some("sess-abc".to_string()),
            environment: Some("production".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn default_strategy_enabled() {
        let flag = make_flag("f", vec![make_strategy("default", vec![])]);
        assert!(evaluate_flag(&flag, "production", &ctx("u1"), &HashMap::new()).enabled);
    }

    #[test]
    fn user_with_id_match_and_miss() {
        let flag = make_flag(
            "f",
            vec![make_strategy("userWithId", vec![("userIds", "alice, bob")])],
        );
        assert!(evaluate_flag(&flag, "production", &ctx("alice"), &HashMap::new()).enabled);
        assert!(!evaluate_flag(&flag, "production", &ctx("charlie"), &HashMap::new()).enabled);
    }

    #[test]
    fn flexible_rollout_100() {
        let flag = make_flag(
            "f",
            vec![make_strategy(
                "flexibleRollout",
                vec![("rollout", "100"), ("stickiness", "userId"), ("groupId", "g")],
            )],
        );
        assert!(evaluate_flag(&flag, "production", &ctx("any"), &HashMap::new()).enabled);
    }

    #[test]
    fn flexible_rollout_0() {
        let flag = make_flag(
            "f",
            vec![make_strategy(
                "flexibleRollout",
                vec![("rollout", "0"), ("stickiness", "userId"), ("groupId", "g")],
            )],
        );
        assert!(!evaluate_flag(&flag, "production", &ctx("any"), &HashMap::new()).enabled);
    }

    #[test]
    fn gradual_rollout_user_id_deterministic() {
        let flag = make_flag(
            "f",
            vec![make_strategy(
                "gradualRolloutUserId",
                vec![("percentage", "50"), ("groupId", "g")],
            )],
        );
        let r1 = evaluate_flag(&flag, "production", &ctx("stable-user"), &HashMap::new()).enabled;
        let r2 = evaluate_flag(&flag, "production", &ctx("stable-user"), &HashMap::new()).enabled;
        assert_eq!(r1, r2);
    }

    #[test]
    fn constraint_in_pass_and_fail() {
        let mut strat = make_strategy("default", vec![]);
        strat.constraints = vec![Constraint {
            context_name: "userId".to_string(),
            operator: ConstraintOperator::In,
            values: vec!["alice".to_string()],
            value: None,
            inverted: false,
            case_insensitive: false,
        }];
        let flag = make_flag("f", vec![strat]);
        assert!(evaluate_flag(&flag, "production", &ctx("alice"), &HashMap::new()).enabled);
        assert!(!evaluate_flag(&flag, "production", &ctx("bob"), &HashMap::new()).enabled);
    }

    #[test]
    fn constraint_inverted_not_in() {
        let mut strat = make_strategy("default", vec![]);
        strat.constraints = vec![Constraint {
            context_name: "userId".to_string(),
            operator: ConstraintOperator::In,
            values: vec!["blocked".to_string()],
            value: None,
            inverted: true,
            case_insensitive: false,
        }];
        let flag = make_flag("f", vec![strat]);
        assert!(evaluate_flag(&flag, "production", &ctx("normal"), &HashMap::new()).enabled);
        assert!(!evaluate_flag(&flag, "production", &ctx("blocked"), &HashMap::new()).enabled);
    }

    #[test]
    fn constraint_str_starts_with() {
        let mut strat = make_strategy("default", vec![]);
        strat.constraints = vec![Constraint {
            context_name: "userId".to_string(),
            operator: ConstraintOperator::StrStartsWith,
            values: vec!["admin-".to_string()],
            value: None,
            inverted: false,
            case_insensitive: false,
        }];
        let flag = make_flag("f", vec![strat]);
        assert!(evaluate_flag(&flag, "production", &ctx("admin-alice"), &HashMap::new()).enabled);
        assert!(!evaluate_flag(&flag, "production", &ctx("user-bob"), &HashMap::new()).enabled);
    }

    #[test]
    fn constraint_num_gte() {
        let mut strat = make_strategy("default", vec![]);
        strat.constraints = vec![Constraint {
            context_name: "score".to_string(),
            operator: ConstraintOperator::NumGte,
            values: vec![],
            value: Some("100".to_string()),
            inverted: false,
            case_insensitive: false,
        }];
        let flag = make_flag("f", vec![strat]);

        let mut c_high = UnleashContext::default();
        c_high.properties.insert("score".to_string(), "150".to_string());
        assert!(evaluate_flag(&flag, "production", &c_high, &HashMap::new()).enabled);

        let mut c_low = UnleashContext::default();
        c_low.properties.insert("score".to_string(), "50".to_string());
        assert!(!evaluate_flag(&flag, "production", &c_low, &HashMap::new()).enabled);
    }

    #[test]
    fn constraint_semver_gt() {
        let mut strat = make_strategy("default", vec![]);
        strat.constraints = vec![Constraint {
            context_name: "appVersion".to_string(),
            operator: ConstraintOperator::SemverGt,
            values: vec![],
            value: Some("2.0.0".to_string()),
            inverted: false,
            case_insensitive: false,
        }];
        let flag = make_flag("f", vec![strat]);

        let mut c_new = UnleashContext::default();
        c_new.properties.insert("appVersion".to_string(), "2.1.0".to_string());
        assert!(evaluate_flag(&flag, "production", &c_new, &HashMap::new()).enabled);

        let mut c_old = UnleashContext::default();
        c_old.properties.insert("appVersion".to_string(), "1.9.0".to_string());
        assert!(!evaluate_flag(&flag, "production", &c_old, &HashMap::new()).enabled);
    }

    #[test]
    fn kill_switch_when_disabled() {
        let flag = FeatureFlag {
            name: "ks".to_string(),
            feature_type: FeatureType::KillSwitch,
            enabled: false,
            stale: false,
            impression_data: false,
            description: String::new(),
            project: "default".to_string(),
            created_at: Utc::now(),
            last_seen_at: None,
            strategies: vec![make_strategy("default", vec![])],
            variants: vec![],
            environments: vec![],
            tags: vec![],
        };
        assert!(!evaluate_flag(&flag, "production", &ctx("alice"), &HashMap::new()).enabled);
    }

    #[test]
    fn variant_selection_deterministic() {
        let variants = vec![
            Variant {
                name: "A".to_string(),
                weight: 500,
                weight_type: WeightType::Variable,
                stickiness: "userId".to_string(),
                payload: None,
                overrides: vec![],
            },
            Variant {
                name: "B".to_string(),
                weight: 500,
                weight_type: WeightType::Variable,
                stickiness: "userId".to_string(),
                payload: None,
                overrides: vec![],
            },
        ];
        let c = ctx("user-xyz");
        let v1 = select_variant(&variants, "f", &c, true);
        let v2 = select_variant(&variants, "f", &c, true);
        assert_eq!(v1.name, v2.name);
    }

    #[test]
    fn murmur3_deterministic() {
        assert_eq!(
            normalized_value_100("user1", "groupId"),
            normalized_value_100("user1", "groupId")
        );
        let v = normalized_value_100("user1", "groupId");
        assert!(v >= 1 && v <= 100);
    }

    #[test]
    fn segment_constraint() {
        let seg = Segment {
            id: 1,
            name: "beta".to_string(),
            description: None,
            constraints: vec![Constraint {
                context_name: "userId".to_string(),
                operator: ConstraintOperator::In,
                values: vec!["beta-user".to_string()],
                value: None,
                inverted: false,
                case_insensitive: false,
            }],
            created_at: Utc::now(),
            created_by: None,
            project: None,
        };
        let mut strat = make_strategy("default", vec![]);
        strat.segments = vec![1];
        let flag = make_flag("f", vec![strat]);
        let seg_map: HashMap<i64, &Segment> = [(1, &seg)].into_iter().collect();

        assert!(evaluate_flag(&flag, "production", &ctx("beta-user"), &seg_map).enabled);
        assert!(!evaluate_flag(&flag, "production", &ctx("normal-user"), &seg_map).enabled);
    }

    #[test]
    fn environment_disabled() {
        let flag = FeatureFlag {
            name: "f".to_string(),
            feature_type: FeatureType::Release,
            enabled: true,
            stale: false,
            impression_data: false,
            description: String::new(),
            project: "default".to_string(),
            created_at: Utc::now(),
            last_seen_at: None,
            strategies: vec![],
            variants: vec![],
            environments: vec![FeatureEnvironment {
                name: "production".to_string(),
                enabled: false,
                strategies: vec![make_strategy("default", vec![])],
                variants: vec![],
            }],
            tags: vec![],
        };
        assert!(!evaluate_flag(&flag, "production", &ctx("u"), &HashMap::new()).enabled);
    }
}
