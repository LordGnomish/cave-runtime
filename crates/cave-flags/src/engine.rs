//! Feature flag evaluation engine — full Unleash v6 strategy parity.
//!
//! ## Strategy dispatch table
//! | Unleash name              | Implementation                          |
//! |---------------------------|-----------------------------------------|
//! | `default`                 | Always enabled                          |
//! | `userWithId`              | Allowlist of user IDs                   |
//! | `gradualRolloutUserId`    | MurmurHash3(groupId:userId) % 100       |
//! | `gradualRolloutSessionId` | MurmurHash3(groupId:sessionId) % 100    |
//! | `gradualRolloutRandom`    | SystemTime subsec nanos % 100           |
//! | `flexibleRollout`         | Configurable stickiness + MurmurHash3   |
//! | `applicationHostname`     | Match against `properties["hostname"]`  |
//! | `remoteAddress`           | Exact IP or CIDR match                  |

use crate::models::{
    Constraint, ConstraintOperator, EvaluationContext, FeatureFlag, FeatureToggle, FlagEvaluation,
    Segment, StrategyConfig, UnleashContext, Variant, VariantResult,
};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

// ================================================================
// MurmurHash3 x86-32 (public-domain algorithm, Unleash-compatible)
// ================================================================

/// MurmurHash3 x86-32 — produces the same values as Unleash SDK clients.
fn murmurhash3_x86_32(data: &[u8], seed: u32) -> u32 {
    const C1: u32 = 0xcc9e2d51;
    const C2: u32 = 0x1b873593;

    let len = data.len();
    let nblocks = len / 4;
    let mut h1 = seed;

    // Body — 4-byte blocks
    for i in 0..nblocks {
        let b = i * 4;
        let mut k1 = u32::from_le_bytes([data[b], data[b + 1], data[b + 2], data[b + 3]]);
        k1 = k1.wrapping_mul(C1);
        k1 = k1.rotate_left(15);
        k1 = k1.wrapping_mul(C2);
        h1 ^= k1;
        h1 = h1.rotate_left(13);
        h1 = h1.wrapping_mul(5).wrapping_add(0xe6546b64);
    }

    // Tail — remaining bytes
    let tail = &data[nblocks * 4..];
    let mut k1: u32 = 0;
    match tail.len() {
        3 => {
            k1 ^= (tail[2] as u32) << 16;
            k1 ^= (tail[1] as u32) << 8;
            k1 ^= tail[0] as u32;
        }
        2 => {
            k1 ^= (tail[1] as u32) << 8;
            k1 ^= tail[0] as u32;
        }
        1 => {
            k1 ^= tail[0] as u32;
        }
        _ => {}
    }
    if !tail.is_empty() {
        k1 = k1.wrapping_mul(C1);
        k1 = k1.rotate_left(15);
        k1 = k1.wrapping_mul(C2);
        h1 ^= k1;
    }

    // Finalisation mix
    h1 ^= len as u32;
    h1 ^= h1 >> 16;
    h1 = h1.wrapping_mul(0x85ebca6b);
    h1 ^= h1 >> 13;
    h1 = h1.wrapping_mul(0xc2b2ae35);
    h1 ^= h1 >> 16;

    h1
}

/// Normalize an identifier for gradual-rollout strategies.
/// Returns a value in 1..=100 (Unleash convention).
fn normalize_identifier(identifier: &str, group_id: &str) -> u32 {
    let key = format!("{group_id}:{identifier}");
    let hash = murmurhash3_x86_32(key.as_bytes(), 0);
    (hash % 100) + 1
}

/// Normalize an identifier for variant selection.
/// Returns a value in 1..=total (weight-sum).
fn normalize_variant(identifier: &str, group_id: &str, total: u32) -> u32 {
    let key = format!("{group_id}:{identifier}");
    let hash = murmurhash3_x86_32(key.as_bytes(), 0);
    (hash % total) + 1
}

// ================================================================
// Context helpers
// ================================================================

/// Get a context field value as an owned String (supports all field names).
fn get_context_value(context: &UnleashContext, field: &str) -> Option<String> {
    match field {
        "userId" => context.user_id.clone(),
        "sessionId" => context.session_id.clone(),
        "remoteAddress" => context.remote_address.clone(),
        "environment" => context.environment.clone(),
        "appName" => context.app_name.clone(),
        "currentTime" => context
            .current_time
            .map(|dt| dt.to_rfc3339())
            .or_else(|| Some(chrono::Utc::now().to_rfc3339())),
        key => context.properties.get(key).cloned(),
    }
}

/// Get a context field as `&str` (for variant override matching).
fn get_context_field<'a>(context: &'a UnleashContext, field: &str) -> Option<&'a str> {
    match field {
        "userId" => context.user_id.as_deref(),
        "sessionId" => context.session_id.as_deref(),
        "remoteAddress" => context.remote_address.as_deref(),
        "environment" => context.environment.as_deref(),
        "appName" => context.app_name.as_deref(),
        key => context.properties.get(key).map(String::as_str),
    }
}

// ================================================================
// IP / CIDR matching (for remoteAddress strategy)
// ================================================================

fn matches_ip_or_cidr(addr: &str, ip_or_cidr: &str) -> bool {
    if ip_or_cidr.contains('/') {
        matches_cidr(addr, ip_or_cidr)
    } else {
        addr == ip_or_cidr.trim()
    }
}

fn matches_cidr(addr: &str, cidr: &str) -> bool {
    use std::net::IpAddr;
    let parts: Vec<&str> = cidr.splitn(2, '/').collect();
    if parts.len() != 2 {
        return false;
    }
    let prefix_len: u8 = match parts[1].trim().parse() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let network: IpAddr = match parts[0].trim().parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let target: IpAddr = match addr.trim().parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    match (network, target) {
        (IpAddr::V4(net), IpAddr::V4(tgt)) => {
            if prefix_len > 32 {
                return false;
            }
            let shift = 32u32.saturating_sub(prefix_len as u32);
            let mask = if shift == 32 { 0u32 } else { !0u32 << shift };
            u32::from(net) & mask == u32::from(tgt) & mask
        }
        (IpAddr::V6(net), IpAddr::V6(tgt)) => {
            if prefix_len > 128 {
                return false;
            }
            let shift = 128u32.saturating_sub(prefix_len as u32);
            let mask = if shift == 128 { 0u128 } else { !0u128 << shift };
            u128::from(net) & mask == u128::from(tgt) & mask
        }
        _ => false,
    }
}

// ================================================================
// Semver comparison (simple numeric tuple ordering)
// ================================================================

fn parse_semver(s: &str) -> (u64, u64, u64) {
    let parts: Vec<u64> = s
        .split('.')
        .filter_map(|p| p.split('-').next()?.parse().ok())
        .collect();
    (
        parts.first().copied().unwrap_or(0),
        parts.get(1).copied().unwrap_or(0),
        parts.get(2).copied().unwrap_or(0),
    )
}

fn compare_semver(a: &str, b: &str) -> Ordering {
    parse_semver(a).cmp(&parse_semver(b))
}

// ================================================================
// Constraint evaluation
// ================================================================

/// Evaluate a single constraint against the context.
pub fn evaluate_constraint(constraint: &Constraint, context: &UnleashContext) -> bool {
    let maybe_value = get_context_value(context, &constraint.context_name);

    let result = match &constraint.operator {
        ConstraintOperator::In => maybe_value.as_deref().map_or(false, |v| {
            let v_cmp = if constraint.case_insensitive {
                v.to_lowercase()
            } else {
                v.to_string()
            };
            constraint.values.iter().any(|cv| {
                let cv_cmp = if constraint.case_insensitive {
                    cv.to_lowercase()
                } else {
                    cv.clone()
                };
                v_cmp == cv_cmp
            })
        }),

        ConstraintOperator::NotIn => maybe_value.as_deref().map_or(true, |v| {
            let v_cmp = if constraint.case_insensitive {
                v.to_lowercase()
            } else {
                v.to_string()
            };
            !constraint.values.iter().any(|cv| {
                let cv_cmp = if constraint.case_insensitive {
                    cv.to_lowercase()
                } else {
                    cv.clone()
                };
                v_cmp == cv_cmp
            })
        }),

        ConstraintOperator::StrStartsWith => {
            maybe_value.as_deref().map_or(false, |v| {
                constraint.values.iter().any(|prefix| {
                    if constraint.case_insensitive {
                        v.to_lowercase().starts_with(&prefix.to_lowercase())
                    } else {
                        v.starts_with(prefix.as_str())
                    }
                })
            })
        }

        ConstraintOperator::StrEndsWith => {
            maybe_value.as_deref().map_or(false, |v| {
                constraint.values.iter().any(|suffix| {
                    if constraint.case_insensitive {
                        v.to_lowercase().ends_with(&suffix.to_lowercase())
                    } else {
                        v.ends_with(suffix.as_str())
                    }
                })
            })
        }

        ConstraintOperator::StrContains => {
            maybe_value.as_deref().map_or(false, |v| {
                constraint.values.iter().any(|needle| {
                    if constraint.case_insensitive {
                        v.to_lowercase().contains(&needle.to_lowercase())
                    } else {
                        v.contains(needle.as_str())
                    }
                })
            })
        }

        ConstraintOperator::NumEq => {
            let n = maybe_value.as_deref().and_then(|v| v.parse::<f64>().ok());
            let c = constraint
                .value
                .as_deref()
                .and_then(|v| v.parse::<f64>().ok());
            matches!((n, c), (Some(a), Some(b)) if (a - b).abs() < f64::EPSILON)
        }

        ConstraintOperator::NumGt => {
            let n = maybe_value.as_deref().and_then(|v| v.parse::<f64>().ok());
            let c = constraint
                .value
                .as_deref()
                .and_then(|v| v.parse::<f64>().ok());
            matches!((n, c), (Some(a), Some(b)) if a > b)
        }

        ConstraintOperator::NumGte => {
            let n = maybe_value.as_deref().and_then(|v| v.parse::<f64>().ok());
            let c = constraint
                .value
                .as_deref()
                .and_then(|v| v.parse::<f64>().ok());
            matches!((n, c), (Some(a), Some(b)) if a >= b)
        }

        ConstraintOperator::NumLt => {
            let n = maybe_value.as_deref().and_then(|v| v.parse::<f64>().ok());
            let c = constraint
                .value
                .as_deref()
                .and_then(|v| v.parse::<f64>().ok());
            matches!((n, c), (Some(a), Some(b)) if a < b)
        }

        ConstraintOperator::NumLte => {
            let n = maybe_value.as_deref().and_then(|v| v.parse::<f64>().ok());
            let c = constraint
                .value
                .as_deref()
                .and_then(|v| v.parse::<f64>().ok());
            matches!((n, c), (Some(a), Some(b)) if a <= b)
        }

        ConstraintOperator::DateBefore => {
            use chrono::DateTime;
            let d = maybe_value
                .as_deref()
                .and_then(|v| v.parse::<DateTime<chrono::Utc>>().ok());
            let c = constraint
                .value
                .as_deref()
                .and_then(|v| v.parse::<DateTime<chrono::Utc>>().ok());
            matches!((d, c), (Some(a), Some(b)) if a < b)
        }

        ConstraintOperator::DateAfter => {
            use chrono::DateTime;
            let d = maybe_value
                .as_deref()
                .and_then(|v| v.parse::<DateTime<chrono::Utc>>().ok());
            let c = constraint
                .value
                .as_deref()
                .and_then(|v| v.parse::<DateTime<chrono::Utc>>().ok());
            matches!((d, c), (Some(a), Some(b)) if a > b)
        }

        ConstraintOperator::SemverEq => maybe_value.as_deref().map_or(false, |v| {
            constraint
                .value
                .as_deref()
                .map_or(false, |c| compare_semver(v, c) == Ordering::Equal)
        }),

        ConstraintOperator::SemverGt => maybe_value.as_deref().map_or(false, |v| {
            constraint
                .value
                .as_deref()
                .map_or(false, |c| compare_semver(v, c) == Ordering::Greater)
        }),

        ConstraintOperator::SemverLt => maybe_value.as_deref().map_or(false, |v| {
            constraint
                .value
                .as_deref()
                .map_or(false, |c| compare_semver(v, c) == Ordering::Less)
        }),
    };

    if constraint.inverted { !result } else { result }
}

/// All constraints must pass (AND semantics).
pub fn evaluate_constraints(constraints: &[Constraint], context: &UnleashContext) -> bool {
    constraints.iter().all(|c| evaluate_constraint(c, context))
}

// ================================================================
// Strategy dispatch
// ================================================================

/// Returns a pseudo-random value in 1..=100 based on system time nanoseconds.
/// Used for non-sticky strategies (gradualRolloutRandom, flexibleRollout random stickiness).
fn random_1_to_100() -> u32 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        % 100)
        + 1
}

/// Dispatch a strategy by Unleash name and evaluate it.
///
/// All known Unleash built-in strategies are handled here.
/// Unknown strategies fall through to `false` (disabled).
pub fn dispatch_strategy(
    name: &str,
    params: &HashMap<String, String>,
    context: &UnleashContext,
) -> bool {
    match name {
        // Always enabled — no parameters needed.
        "default" => true,

        // userWithId: comma-separated list in params["userIds"]
        "userWithId" => {
            let ids: Vec<&str> = params
                .get("userIds")
                .map(|s| s.split(',').map(str::trim).collect())
                .unwrap_or_default();
            context
                .user_id
                .as_deref()
                .map_or(false, |uid| ids.contains(&uid))
        }

        // gradualRolloutUserId: MurmurHash3(groupId:userId) % 100 + 1 <= percentage
        "gradualRolloutUserId" => {
            let pct = params
                .get("percentage")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let group = params
                .get("groupId")
                .map(String::as_str)
                .unwrap_or_default();
            context
                .user_id
                .as_deref()
                .map_or(false, |uid| normalize_identifier(uid, group) <= pct)
        }

        // gradualRolloutSessionId: same but keyed on sessionId
        "gradualRolloutSessionId" => {
            let pct = params
                .get("percentage")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let group = params
                .get("groupId")
                .map(String::as_str)
                .unwrap_or_default();
            context
                .session_id
                .as_deref()
                .map_or(false, |sid| normalize_identifier(sid, group) <= pct)
        }

        // gradualRolloutRandom: non-deterministic, new roll each evaluation
        "gradualRolloutRandom" => {
            let pct = params
                .get("percentage")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            random_1_to_100() <= pct
        }

        // flexibleRollout: configurable stickiness (userId, sessionId, random, default)
        "flexibleRollout" => {
            let rollout = params
                .get("rollout")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let group = params
                .get("groupId")
                .map(String::as_str)
                .unwrap_or_default();
            let stickiness = params
                .get("stickiness")
                .map(String::as_str)
                .unwrap_or("default");

            match stickiness {
                "userId" => context
                    .user_id
                    .as_deref()
                    .map_or(false, |uid| normalize_identifier(uid, group) <= rollout),
                "sessionId" => context
                    .session_id
                    .as_deref()
                    .map_or(false, |sid| normalize_identifier(sid, group) <= rollout),
                "random" => random_1_to_100() <= rollout,
                // "default": userId first, then sessionId, then random
                _ => {
                    if let Some(uid) = context.user_id.as_deref() {
                        normalize_identifier(uid, group) <= rollout
                    } else if let Some(sid) = context.session_id.as_deref() {
                        normalize_identifier(sid, group) <= rollout
                    } else {
                        random_1_to_100() <= rollout
                    }
                }
            }
        }

        // applicationHostname: match against context.properties["hostname"]
        "applicationHostname" => {
            let host_names: Vec<&str> = params
                .get("hostNames")
                .map(|s| s.split(',').map(str::trim).collect())
                .unwrap_or_default();
            context
                .properties
                .get("hostname")
                .map_or(false, |h| host_names.contains(&h.as_str()))
        }

        // remoteAddress: exact IP or CIDR, comma-separated in params["IPs"]
        "remoteAddress" => {
            let ips: Vec<&str> = params
                .get("IPs")
                .map(|s| s.split(',').map(str::trim).collect())
                .unwrap_or_default();
            context.remote_address.as_deref().map_or(false, |addr| {
                ips.iter().any(|ip_or_cidr| matches_ip_or_cidr(addr, ip_or_cidr))
            })
        }

        // Unknown strategy: disabled
        _ => false,
    }
}

// ================================================================
// Strategy + Toggle evaluation (Unleash API)
// ================================================================

/// Evaluate a single strategy config (constraints → segments → dispatch).
/// Returns `true` if this strategy enables the toggle for the given context.
pub fn evaluate_strategy(
    strategy: &StrategyConfig,
    context: &UnleashContext,
    segments: &[Segment],
) -> bool {
    // All strategy-level constraints must pass
    if !evaluate_constraints(&strategy.constraints, context) {
        return false;
    }
    // All referenced segment constraints must pass
    for &seg_id in &strategy.segments {
        if let Some(seg) = segments.iter().find(|s| s.id == seg_id) {
            if !evaluate_constraints(&seg.constraints, context) {
                return false;
            }
        }
    }
    // Dispatch to the named strategy implementation
    dispatch_strategy(&strategy.name, &strategy.parameters, context)
}

/// Evaluate all strategies for a toggle (ANY-match semantics).
/// An empty strategy list means always enabled (Unleash default).
pub fn evaluate_strategies(
    strategies: &[StrategyConfig],
    context: &UnleashContext,
    segments: &[Segment],
) -> bool {
    if strategies.is_empty() {
        return true;
    }
    strategies
        .iter()
        .any(|s| evaluate_strategy(s, context, segments))
}

/// Full toggle evaluation → `(feature_enabled, variant)`.
///
/// Respects the toggle's `enabled` / `archived` gates before dispatching
/// to the strategy evaluator and selecting a variant.
pub fn evaluate_toggle(
    toggle: &FeatureToggle,
    context: &UnleashContext,
    segments: &[Segment],
) -> (bool, VariantResult) {
    if !toggle.enabled || toggle.archived {
        return (false, VariantResult::disabled(false));
    }
    let feature_enabled = evaluate_strategies(&toggle.strategies, context, segments);
    let variant = select_variant(&toggle.variants, context, &toggle.name, feature_enabled);
    (feature_enabled, variant)
}

// ================================================================
// Variant selection
// ================================================================

/// Select a variant for the given context using weight-based hashing.
///
/// Priority order:
/// 1. Overrides (context-field match)
/// 2. Weighted hash using stickiness identifier
pub fn select_variant(
    variants: &[Variant],
    context: &UnleashContext,
    toggle_name: &str,
    feature_enabled: bool,
) -> VariantResult {
    if variants.is_empty() || !feature_enabled {
        return VariantResult::disabled(feature_enabled);
    }

    // Override check: first variant whose override matches wins
    for variant in variants {
        for ov in &variant.overrides {
            if let Some(val) = get_context_field(context, &ov.context_name) {
                if ov.values.iter().any(|v| v == val) {
                    return VariantResult {
                        name: variant.name.clone(),
                        enabled: true,
                        payload: variant.payload.clone(),
                        feature_enabled,
                    };
                }
            }
        }
    }

    let total: u32 = variants.iter().map(|v| v.weight).sum();
    if total == 0 {
        return VariantResult::disabled(feature_enabled);
    }

    // Resolve stickiness identifier
    let stickiness = variants
        .first()
        .map(|v| v.stickiness.as_str())
        .unwrap_or("default");

    let normalized = match stickiness {
        "userId" => context
            .user_id
            .as_deref()
            .map(|uid| normalize_variant(uid, toggle_name, total))
            .unwrap_or_else(|| random_1_to_100().min(total).max(1)),
        "sessionId" => context
            .session_id
            .as_deref()
            .map(|sid| normalize_variant(sid, toggle_name, total))
            .unwrap_or_else(|| random_1_to_100().min(total).max(1)),
        "random" => (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
            % total)
            + 1,
        // "default": userId → sessionId → random
        _ => {
            if let Some(uid) = context.user_id.as_deref() {
                normalize_variant(uid, toggle_name, total)
            } else if let Some(sid) = context.session_id.as_deref() {
                normalize_variant(sid, toggle_name, total)
            } else {
                (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos()
                    % total)
                    + 1
            }
        }
    };

    // Walk cumulative weights
    let mut cumulative = 0u32;
    for variant in variants {
        cumulative += variant.weight;
        if normalized <= cumulative {
            return VariantResult {
                name: variant.name.clone(),
                enabled: true,
                payload: variant.payload.clone(),
                feature_enabled,
            };
        }
    }

    VariantResult::disabled(feature_enabled)
}

// ================================================================
// Legacy CAVE API (backward compat)
// ================================================================

/// Evaluate all flags for a given context (legacy /api/flags/evaluate).
pub fn evaluate_flags(
    flags: &[FeatureFlag],
    context: &EvaluationContext,
) -> Vec<FlagEvaluation> {
    flags
        .iter()
        .map(|flag| evaluate_single(flag, context))
        .collect()
}

/// Evaluate a single legacy flag against a context.
pub fn evaluate_single(flag: &FeatureFlag, context: &EvaluationContext) -> FlagEvaluation {
    // Kill switch always wins
    if flag.kill_switch {
        return FlagEvaluation {
            name: flag.name.clone(),
            enabled: false,
            variant: None,
        };
    }

    // Environment scope
    if !flag.environments.is_empty() && !flag.environments.contains(&context.environment) {
        return FlagEvaluation {
            name: flag.name.clone(),
            enabled: false,
            variant: None,
        };
    }

    // Tenant scope
    if let Some(ref flag_tenant) = flag.tenant_id {
        if context.tenant_id.as_ref() != Some(flag_tenant) {
            return FlagEvaluation {
                name: flag.name.clone(),
                enabled: false,
                variant: None,
            };
        }
    }

    // Global enable gate
    if !flag.enabled {
        return FlagEvaluation {
            name: flag.name.clone(),
            enabled: false,
            variant: None,
        };
    }

    // Strategy evaluation
    use crate::models::Strategy;
    let enabled = match &flag.strategy {
        Strategy::Default { enabled } => *enabled,
        Strategy::GradualRollout {
            percentage,
            group_id,
        } => {
            let key = format!(
                "{}:{}",
                group_id.as_deref().unwrap_or(&flag.name),
                context.user_id.as_deref().unwrap_or("anonymous")
            );
            normalize_hash(&key) < *percentage as u32
        }
        Strategy::UserIds { user_ids } => context
            .user_id
            .as_ref()
            .map_or(false, |uid| user_ids.contains(uid)),
        Strategy::TenantScope { tenant_ids } => context
            .tenant_id
            .as_ref()
            .map_or(false, |tid| tenant_ids.contains(tid)),
        Strategy::EnvironmentScope { environments } => {
            environments.contains(&context.environment)
        }
        Strategy::Custom { .. } => flag.enabled,
    };

    FlagEvaluation {
        name: flag.name.clone(),
        enabled,
        variant: None,
    }
}

/// Legacy hash normaliser (DefaultHasher, 0-99 range).
fn normalize_hash(key: &str) -> u32 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() % 100) as u32
}

// ================================================================
// Tests
// ================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    // ── Helpers ──────────────────────────────────────────────────

    fn make_flag(name: &str, strategy: Strategy) -> FeatureFlag {
        FeatureFlag {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: String::new(),
            enabled: true,
            flag_type: FlagType::Boolean,
            strategy,
            environments: vec![],
            tenant_id: None,
            kill_switch: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: Uuid::new_v4(),
        }
    }

    fn make_context(env: &str) -> EvaluationContext {
        EvaluationContext {
            user_id: Some("user-123".to_string()),
            tenant_id: Some("tenant-acme".to_string()),
            environment: env.to_string(),
            properties: None,
        }
    }

    fn unleash_ctx(user_id: Option<&str>) -> UnleashContext {
        UnleashContext {
            user_id: user_id.map(String::from),
            session_id: Some("sess-abc".to_string()),
            remote_address: Some("192.168.1.5".to_string()),
            environment: Some("production".to_string()),
            app_name: Some("test-app".to_string()),
            current_time: None,
            properties: HashMap::new(),
        }
    }

    fn strategy(name: &str, params: &[(&str, &str)]) -> StrategyConfig {
        StrategyConfig {
            name: name.to_string(),
            parameters: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            constraints: vec![],
            segments: vec![],
        }
    }

    fn make_toggle(name: &str, strategies: Vec<StrategyConfig>) -> FeatureToggle {
        let mut t = FeatureToggle::new(name, "test");
        t.strategies = strategies;
        t
    }

    fn constraint(
        context_name: &str,
        operator: ConstraintOperator,
        values: Vec<&str>,
        value: Option<&str>,
    ) -> Constraint {
        Constraint {
            context_name: context_name.to_string(),
            operator,
            values: values.iter().map(|v| v.to_string()).collect(),
            value: value.map(String::from),
            inverted: false,
            case_insensitive: false,
        }
    }

    // ── Legacy CAVE tests (original 5) ───────────────────────────

    #[test]
    fn test_default_strategy_enabled() {
        let flag = make_flag("test", Strategy::Default { enabled: true });
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(result.enabled);
    }

    #[test]
    fn test_kill_switch_overrides() {
        let mut flag = make_flag("test", Strategy::Default { enabled: true });
        flag.kill_switch = true;
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(!result.enabled);
    }

    #[test]
    fn test_environment_scope() {
        let mut flag = make_flag("test", Strategy::Default { enabled: true });
        flag.environments = vec!["staging".to_string()];
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(!result.enabled);
    }

    #[test]
    fn test_user_id_strategy() {
        let flag = make_flag(
            "test",
            Strategy::UserIds {
                user_ids: vec!["user-123".to_string()],
            },
        );
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(result.enabled);
    }

    #[test]
    fn test_gradual_rollout_deterministic() {
        let flag = make_flag(
            "test",
            Strategy::GradualRollout {
                percentage: 50,
                group_id: None,
            },
        );
        let ctx = make_context("prod");
        let r1 = evaluate_single(&flag, &ctx);
        let r2 = evaluate_single(&flag, &ctx);
        assert_eq!(r1.enabled, r2.enabled);
    }

    // ── Unleash: default strategy ─────────────────────────────────

    #[test]
    fn test_unleash_default_strategy_always_enabled() {
        let toggle = make_toggle("feat", vec![strategy("default", &[])]);
        let ctx = unleash_ctx(Some("u1"));
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    // ── Unleash: userWithId ───────────────────────────────────────

    #[test]
    fn test_unleash_user_with_id_match() {
        let toggle = make_toggle(
            "feat",
            vec![strategy("userWithId", &[("userIds", "alice,bob,charlie")])],
        );
        let ctx = UnleashContext {
            user_id: Some("bob".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    #[test]
    fn test_unleash_user_with_id_no_match() {
        let toggle = make_toggle(
            "feat",
            vec![strategy("userWithId", &[("userIds", "alice,bob")])],
        );
        let ctx = UnleashContext {
            user_id: Some("dave".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    #[test]
    fn test_unleash_user_with_id_no_user_disabled() {
        let toggle = make_toggle(
            "feat",
            vec![strategy("userWithId", &[("userIds", "alice")])],
        );
        let ctx = UnleashContext::default();
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    // ── Unleash: gradualRolloutUserId ─────────────────────────────

    #[test]
    fn test_gradual_rollout_user_id_100_percent() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "gradualRolloutUserId",
                &[("percentage", "100"), ("groupId", "mygroup")],
            )],
        );
        let ctx = unleash_ctx(Some("any-user"));
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled, "100% rollout must always be enabled");
    }

    #[test]
    fn test_gradual_rollout_user_id_0_percent() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "gradualRolloutUserId",
                &[("percentage", "0"), ("groupId", "mygroup")],
            )],
        );
        let ctx = unleash_ctx(Some("any-user"));
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled, "0% rollout must always be disabled");
    }

    #[test]
    fn test_gradual_rollout_user_id_deterministic() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "gradualRolloutUserId",
                &[("percentage", "50"), ("groupId", "g1")],
            )],
        );
        let ctx = unleash_ctx(Some("user-xyz"));
        let (r1, _) = evaluate_toggle(&toggle, &ctx, &[]);
        let (r2, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert_eq!(r1, r2, "same user/group must always get same result");
    }

    #[test]
    fn test_gradual_rollout_user_id_no_user_disabled() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "gradualRolloutUserId",
                &[("percentage", "100"), ("groupId", "g1")],
            )],
        );
        let ctx = UnleashContext::default(); // no userId
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    // ── Unleash: gradualRolloutSessionId ──────────────────────────

    #[test]
    fn test_gradual_rollout_session_id_100_percent() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "gradualRolloutSessionId",
                &[("percentage", "100"), ("groupId", "sg")],
            )],
        );
        let ctx = UnleashContext {
            session_id: Some("session-001".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    #[test]
    fn test_gradual_rollout_session_id_0_percent() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "gradualRolloutSessionId",
                &[("percentage", "0"), ("groupId", "sg")],
            )],
        );
        let ctx = UnleashContext {
            session_id: Some("session-001".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    // ── Unleash: gradualRolloutRandom ─────────────────────────────

    #[test]
    fn test_gradual_rollout_random_100_percent_always_enabled() {
        let toggle = make_toggle(
            "feat",
            vec![strategy("gradualRolloutRandom", &[("percentage", "100")])],
        );
        let ctx = UnleashContext::default();
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    #[test]
    fn test_gradual_rollout_random_0_percent_always_disabled() {
        let toggle = make_toggle(
            "feat",
            vec![strategy("gradualRolloutRandom", &[("percentage", "0")])],
        );
        let ctx = UnleashContext::default();
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    // ── Unleash: flexibleRollout ──────────────────────────────────

    #[test]
    fn test_flexible_rollout_user_id_100() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "flexibleRollout",
                &[
                    ("rollout", "100"),
                    ("groupId", "flex"),
                    ("stickiness", "userId"),
                ],
            )],
        );
        let ctx = unleash_ctx(Some("u1"));
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    #[test]
    fn test_flexible_rollout_user_id_0() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "flexibleRollout",
                &[
                    ("rollout", "0"),
                    ("groupId", "flex"),
                    ("stickiness", "userId"),
                ],
            )],
        );
        let ctx = unleash_ctx(Some("u1"));
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    #[test]
    fn test_flexible_rollout_session_stickiness() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "flexibleRollout",
                &[
                    ("rollout", "100"),
                    ("groupId", "flex"),
                    ("stickiness", "sessionId"),
                ],
            )],
        );
        let ctx = UnleashContext {
            session_id: Some("sess-xyz".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    // ── Unleash: applicationHostname ──────────────────────────────

    #[test]
    fn test_application_hostname_match() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "applicationHostname",
                &[("hostNames", "web-01,web-02,worker-01")],
            )],
        );
        let mut ctx = UnleashContext::default();
        ctx.properties.insert("hostname".to_string(), "web-02".to_string());
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    #[test]
    fn test_application_hostname_no_match() {
        let toggle = make_toggle(
            "feat",
            vec![strategy(
                "applicationHostname",
                &[("hostNames", "web-01,web-02")],
            )],
        );
        let mut ctx = UnleashContext::default();
        ctx.properties.insert("hostname".to_string(), "db-01".to_string());
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    // ── Unleash: remoteAddress ────────────────────────────────────

    #[test]
    fn test_remote_address_exact_match() {
        let toggle = make_toggle(
            "feat",
            vec![strategy("remoteAddress", &[("IPs", "10.0.0.1,10.0.0.2")])],
        );
        let ctx = UnleashContext {
            remote_address: Some("10.0.0.2".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    #[test]
    fn test_remote_address_no_match() {
        let toggle = make_toggle(
            "feat",
            vec![strategy("remoteAddress", &[("IPs", "10.0.0.1")])],
        );
        let ctx = UnleashContext {
            remote_address: Some("192.168.0.1".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    #[test]
    fn test_remote_address_cidr_match() {
        let toggle = make_toggle(
            "feat",
            vec![strategy("remoteAddress", &[("IPs", "192.168.1.0/24")])],
        );
        let ctx = UnleashContext {
            remote_address: Some("192.168.1.42".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    #[test]
    fn test_remote_address_cidr_no_match() {
        let toggle = make_toggle(
            "feat",
            vec![strategy("remoteAddress", &[("IPs", "10.0.0.0/8")])],
        );
        let ctx = UnleashContext {
            remote_address: Some("172.16.0.1".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    // ── Constraint tests ─────────────────────────────────────────

    #[test]
    fn test_constraint_in_match() {
        let c = constraint("userId", ConstraintOperator::In, vec!["alice", "bob"], None);
        let ctx = UnleashContext {
            user_id: Some("alice".to_string()),
            ..Default::default()
        };
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_in_no_match() {
        let c = constraint("userId", ConstraintOperator::In, vec!["alice", "bob"], None);
        let ctx = UnleashContext {
            user_id: Some("charlie".to_string()),
            ..Default::default()
        };
        assert!(!evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_not_in() {
        let c = constraint("userId", ConstraintOperator::NotIn, vec!["alice"], None);
        let ctx = UnleashContext {
            user_id: Some("bob".to_string()),
            ..Default::default()
        };
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_not_in_missing_field_passes() {
        // NOT_IN with missing context field should pass (Unleash spec)
        let c = constraint("userId", ConstraintOperator::NotIn, vec!["alice"], None);
        let ctx = UnleashContext::default(); // no userId
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_str_starts_with() {
        let c = constraint(
            "userId",
            ConstraintOperator::StrStartsWith,
            vec!["user-"],
            None,
        );
        let ctx = UnleashContext {
            user_id: Some("user-123".to_string()),
            ..Default::default()
        };
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_str_ends_with() {
        let c = constraint(
            "environment",
            ConstraintOperator::StrEndsWith,
            vec!["-prod"],
            None,
        );
        let ctx = UnleashContext {
            environment: Some("us-east-prod".to_string()),
            ..Default::default()
        };
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_str_contains() {
        let c = constraint(
            "appName",
            ConstraintOperator::StrContains,
            vec!["backend"],
            None,
        );
        let ctx = UnleashContext {
            app_name: Some("my-backend-service".to_string()),
            ..Default::default()
        };
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_num_gt() {
        let mut c = constraint("plan_seats", ConstraintOperator::NumGt, vec![], Some("10"));
        c.context_name = "plan_seats".to_string();
        let mut ctx = UnleashContext::default();
        ctx.properties
            .insert("plan_seats".to_string(), "25".to_string());
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_num_lte() {
        let mut c = constraint("tier", ConstraintOperator::NumLte, vec![], Some("3"));
        c.context_name = "tier".to_string();
        let mut ctx = UnleashContext::default();
        ctx.properties.insert("tier".to_string(), "3".to_string());
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_inverted() {
        let mut c = constraint("userId", ConstraintOperator::In, vec!["blocked"], None);
        c.inverted = true;
        let ctx = UnleashContext {
            user_id: Some("normal-user".to_string()),
            ..Default::default()
        };
        // "normal-user" NOT IN ["blocked"] → true after inversion
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_case_insensitive() {
        let mut c = constraint("userId", ConstraintOperator::In, vec!["Alice"], None);
        c.case_insensitive = true;
        let ctx = UnleashContext {
            user_id: Some("alice".to_string()),
            ..Default::default()
        };
        assert!(evaluate_constraint(&c, &ctx));
    }

    #[test]
    fn test_constraint_semver_gt() {
        let c = constraint(
            "appVersion",
            ConstraintOperator::SemverGt,
            vec![],
            Some("2.0.0"),
        );
        let mut ctx = UnleashContext::default();
        ctx.properties
            .insert("appVersion".to_string(), "2.1.0".to_string());
        assert!(evaluate_constraint(&c, &ctx));
    }

    // ── Strategy with constraints ─────────────────────────────────

    #[test]
    fn test_strategy_constraint_blocks_enabled_strategy() {
        // default strategy (always on) gated by a constraint that fails
        let mut strat = strategy("default", &[]);
        strat.constraints = vec![Constraint {
            context_name: "userId".to_string(),
            operator: ConstraintOperator::In,
            values: vec!["admin".to_string()],
            value: None,
            inverted: false,
            case_insensitive: false,
        }];
        let toggle = make_toggle("feat", vec![strat]);
        let ctx = UnleashContext {
            user_id: Some("regular-user".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    #[test]
    fn test_strategy_constraint_allows_when_satisfied() {
        let mut strat = strategy("default", &[]);
        strat.constraints = vec![Constraint {
            context_name: "userId".to_string(),
            operator: ConstraintOperator::In,
            values: vec!["admin".to_string()],
            value: None,
            inverted: false,
            case_insensitive: false,
        }];
        let toggle = make_toggle("feat", vec![strat]);
        let ctx = UnleashContext {
            user_id: Some("admin".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
    }

    // ── Multi-strategy (any-match) ────────────────────────────────

    #[test]
    fn test_multi_strategy_any_match_enables() {
        // Two strategies: first fails (wrong user), second passes (default)
        let mut s1 = strategy("userWithId", &[("userIds", "alice")]);
        let _ = &mut s1; // ensure s1 is used
        let s2 = strategy("default", &[]);
        let toggle = make_toggle("feat", vec![s1, s2]);
        let ctx = UnleashContext {
            user_id: Some("bob".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled, "second strategy (default) should enable the toggle");
    }

    #[test]
    fn test_toggle_disabled_globally() {
        let mut toggle = make_toggle("feat", vec![strategy("default", &[])]);
        toggle.enabled = false;
        let ctx = unleash_ctx(Some("u1"));
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    #[test]
    fn test_archived_toggle_disabled() {
        let mut toggle = make_toggle("feat", vec![strategy("default", &[])]);
        toggle.archived = true;
        let ctx = unleash_ctx(Some("u1"));
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(!enabled);
    }

    // ── Segment evaluation ────────────────────────────────────────

    #[test]
    fn test_segment_constraint_blocks_strategy() {
        let seg = Segment {
            id: 1,
            name: "internal-users".to_string(),
            description: None,
            constraints: vec![Constraint {
                context_name: "userId".to_string(),
                operator: ConstraintOperator::StrStartsWith,
                values: vec!["int-".to_string()],
                value: None,
                inverted: false,
                case_insensitive: false,
            }],
            created_at: Utc::now(),
            created_by: "admin".to_string(),
        };

        let mut strat = strategy("default", &[]);
        strat.segments = vec![1]; // references seg.id
        let toggle = make_toggle("feat", vec![strat]);

        // External user — segment constraint fails
        let ctx = UnleashContext {
            user_id: Some("ext-user-99".to_string()),
            ..Default::default()
        };
        let (enabled, _) = evaluate_toggle(&toggle, &ctx, &[seg.clone()]);
        assert!(!enabled);

        // Internal user — segment constraint passes
        let ctx2 = UnleashContext {
            user_id: Some("int-employee-42".to_string()),
            ..Default::default()
        };
        let (enabled2, _) = evaluate_toggle(&toggle, &ctx2, &[seg]);
        assert!(enabled2);
    }

    // ── Variant selection ─────────────────────────────────────────

    #[test]
    fn test_variant_no_variants_returns_disabled() {
        let toggle = make_toggle("feat", vec![strategy("default", &[])]);
        // toggle has no variants
        let ctx = unleash_ctx(Some("u1"));
        let (enabled, variant) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
        assert!(!variant.enabled);
        assert_eq!(variant.name, "disabled");
    }

    #[test]
    fn test_variant_selection_deterministic() {
        let mut toggle = make_toggle("feat", vec![strategy("default", &[])]);
        toggle.variants = vec![
            Variant {
                name: "red".to_string(),
                weight: 500,
                weight_type: WeightType::Variable,
                payload: None,
                overrides: vec![],
                stickiness: "userId".to_string(),
            },
            Variant {
                name: "blue".to_string(),
                weight: 500,
                weight_type: WeightType::Variable,
                payload: None,
                overrides: vec![],
                stickiness: "userId".to_string(),
            },
        ];
        let ctx = unleash_ctx(Some("u-stable"));
        let (_, v1) = evaluate_toggle(&toggle, &ctx, &[]);
        let (_, v2) = evaluate_toggle(&toggle, &ctx, &[]);
        assert_eq!(v1.name, v2.name, "same user must get same variant");
        assert!(v1.enabled);
    }

    #[test]
    fn test_variant_override_wins() {
        let mut toggle = make_toggle("feat", vec![strategy("default", &[])]);
        toggle.variants = vec![
            Variant {
                name: "standard".to_string(),
                weight: 1000,
                weight_type: WeightType::Variable,
                payload: None,
                overrides: vec![],
                stickiness: "userId".to_string(),
            },
            Variant {
                name: "vip".to_string(),
                weight: 0,
                weight_type: WeightType::Fix,
                payload: None,
                overrides: vec![VariantOverride {
                    context_name: "userId".to_string(),
                    values: vec!["vip-user".to_string()],
                }],
                stickiness: "userId".to_string(),
            },
        ];
        let ctx = UnleashContext {
            user_id: Some("vip-user".to_string()),
            ..Default::default()
        };
        let (enabled, variant) = evaluate_toggle(&toggle, &ctx, &[]);
        assert!(enabled);
        assert_eq!(variant.name, "vip", "override must win regardless of weight");
    }

    // ── MurmurHash3 sanity ────────────────────────────────────────

    #[test]
    fn test_murmurhash3_empty_input_stable() {
        let h1 = murmurhash3_x86_32(b"", 0);
        let h2 = murmurhash3_x86_32(b"", 0);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_normalize_identifier_bounds() {
        // Any identifier must produce 1..=100
        for uid in &["a", "hello", "user-9999", "aaaaaaaaaaaaaaaaaaaaaaaaa"] {
            let n = normalize_identifier(uid, "mygroup");
            assert!(n >= 1 && n <= 100, "normalize_identifier out of range: {n}");
        }
    }
}
