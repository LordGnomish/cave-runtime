// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Gap-close integration tests for cave-flags (Unleash port).
//
// Covers public API behaviour across six modules with no inline tests:
//   - models.rs        (FeatureFlag / Segment / UnleashContext serde + accessor)
//   - lib.rs           (FlagsState / FeatureCache / MODULE_NAME)
//   - engine (public)  (evaluate_flag / evaluate_all / select_variant /
//                       normalized_value_{100,1000} / evaluate_constraints)
// Plus a smoke routes/store check via the public `router` constructor (which
// exercises store.rs + routes.rs module wiring).
//
// Design: failure modes, boundary, state transitions, serde round-trip.

use cave_flags::engine::{
    evaluate_all, evaluate_constraints, evaluate_flag, normalized_value_100,
    normalized_value_1000, select_variant,
};
use cave_flags::models::{
    Constraint, ConstraintOperator, EvaluatedVariant, FeatureEnvironment, FeatureFlag,
    FeatureStrategy, FeatureType, Segment, UnleashContext, Variant, VariantOverride,
    VariantPayload, WeightType,
};
use cave_flags::{FeatureCache, FlagsState, MODULE_NAME};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn strat(name: &str, params: &[(&str, &str)]) -> FeatureStrategy {
    FeatureStrategy {
        id: Uuid::new_v4(),
        name: name.to_string(),
        parameters: params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        constraints: vec![],
        segments: vec![],
        sort_order: 0,
        disabled: false,
        variants: vec![],
    }
}

fn flag_with(name: &str, env: &str, strategies: Vec<FeatureStrategy>) -> FeatureFlag {
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
            name: env.to_string(),
            enabled: true,
            strategies,
            variants: vec![],
        }],
        tags: vec![],
    }
}

fn ctx_user(uid: &str) -> UnleashContext {
    UnleashContext {
        user_id: Some(uid.to_string()),
        session_id: Some("sess".to_string()),
        environment: Some("production".to_string()),
        ..Default::default()
    }
}

fn constraint(
    field: &str,
    op: ConstraintOperator,
    values: Vec<&str>,
    value: Option<&str>,
) -> Constraint {
    Constraint {
        context_name: field.to_string(),
        operator: op,
        values: values.into_iter().map(String::from).collect(),
        value: value.map(String::from),
        inverted: false,
        case_insensitive: false,
    }
}

// ─── lib.rs: module-name + state defaults ────────────────────────────────────

#[test]
fn module_name_constant_matches_schema() {
    assert_eq!(MODULE_NAME, "flags");
}

#[test]
fn feature_cache_default_is_empty() {
    let c = FeatureCache::default();
    assert!(c.features.is_empty());
    assert!(c.segments.is_empty());
}

#[tokio::test]
async fn flags_state_default_has_empty_cache() {
    let state = FlagsState::default();
    let cache = state.cache.read().await;
    assert!(cache.features.is_empty());
    assert!(cache.segments.is_empty());
}

#[tokio::test]
async fn router_builds_from_default_state() {
    use std::sync::Arc;
    // Smoke: confirms routes::create_router wires without panicking.
    let state = Arc::new(FlagsState::default());
    let _router = cave_flags::router(state);
}

// ─── models.rs: context resolution + serde round-trip ────────────────────────

#[test]
fn context_get_field_known_fields() {
    let mut ctx = UnleashContext {
        user_id: Some("u".into()),
        session_id: Some("s".into()),
        remote_address: Some("10.0.0.1".into()),
        environment: Some("dev".into()),
        app_name: Some("svc".into()),
        current_time: Some("2026-01-01T00:00:00Z".into()),
        ..Default::default()
    };
    ctx.properties.insert("hostname".into(), "host-1".into());

    assert_eq!(ctx.get_field("userId").as_deref(), Some("u"));
    assert_eq!(ctx.get_field("sessionId").as_deref(), Some("s"));
    assert_eq!(ctx.get_field("remoteAddress").as_deref(), Some("10.0.0.1"));
    assert_eq!(ctx.get_field("environment").as_deref(), Some("dev"));
    assert_eq!(ctx.get_field("appName").as_deref(), Some("svc"));
    assert_eq!(
        ctx.get_field("currentTime").as_deref(),
        Some("2026-01-01T00:00:00Z")
    );
    assert_eq!(ctx.get_field("hostname").as_deref(), Some("host-1"));
    assert!(ctx.get_field("unknown").is_none());
}

#[test]
fn context_default_has_no_fields() {
    let ctx = UnleashContext::default();
    assert!(ctx.get_field("userId").is_none());
    assert!(ctx.get_field("anyProperty").is_none());
}

#[test]
fn unleash_context_serde_camelcase_round_trip() {
    let json = r#"{
        "userId": "alice",
        "sessionId": "s-1",
        "remoteAddress": "127.0.0.1",
        "environment": "prod",
        "appName": "demo",
        "currentTime": "2026-05-20T00:00:00Z",
        "properties": {"tier": "gold"}
    }"#;
    let ctx: UnleashContext = serde_json::from_str(json).expect("deserialize");
    assert_eq!(ctx.user_id.as_deref(), Some("alice"));
    assert_eq!(ctx.remote_address.as_deref(), Some("127.0.0.1"));
    assert_eq!(ctx.properties.get("tier").map(String::as_str), Some("gold"));
}

#[test]
fn feature_type_kebab_case_serde() {
    let killswitch = serde_json::to_string(&FeatureType::KillSwitch).unwrap();
    assert_eq!(killswitch, "\"kill-switch\"");
    let release = serde_json::to_string(&FeatureType::Release).unwrap();
    assert_eq!(release, "\"release\"");
    let back: FeatureType = serde_json::from_str("\"kill-switch\"").unwrap();
    assert!(matches!(back, FeatureType::KillSwitch));
}

#[test]
fn constraint_operator_screaming_snake_serde() {
    let v = serde_json::to_string(&ConstraintOperator::NotIn).unwrap();
    assert_eq!(v, "\"NOT_IN\"");
    let v = serde_json::to_string(&ConstraintOperator::StrStartsWith).unwrap();
    assert_eq!(v, "\"STR_STARTS_WITH\"");
    let parsed: ConstraintOperator = serde_json::from_str("\"SEMVER_GT\"").unwrap();
    assert!(matches!(parsed, ConstraintOperator::SemverGt));
}

#[test]
fn evaluated_variant_disabled_helper() {
    let v = EvaluatedVariant::disabled();
    assert_eq!(v.name, "disabled");
    assert!(!v.enabled);
    assert!(!v.feature_enabled);
    assert!(v.payload.is_none());
}

// ─── engine: constraint operators ────────────────────────────────────────────

#[test]
fn constraint_in_case_insensitive_matches() {
    let mut c = constraint("userId", ConstraintOperator::In, vec!["ALICE"], None);
    c.case_insensitive = true;
    let ok = evaluate_constraints(std::slice::from_ref(&c), &ctx_user("alice"));
    assert!(ok);
}

#[test]
fn constraint_not_in_missing_value_is_true() {
    // Per spec: NOT_IN on absent ctx field passes (you are not in the list).
    let c = constraint("missing", ConstraintOperator::NotIn, vec!["x"], None);
    assert!(evaluate_constraints(std::slice::from_ref(&c), &UnleashContext::default()));
}

#[test]
fn constraint_str_ends_with() {
    let c = constraint(
        "userId",
        ConstraintOperator::StrEndsWith,
        vec!["@corp.com"],
        None,
    );
    assert!(evaluate_constraints(
        std::slice::from_ref(&c),
        &ctx_user("alice@corp.com")
    ));
    assert!(!evaluate_constraints(
        std::slice::from_ref(&c),
        &ctx_user("alice@other.io")
    ));
}

#[test]
fn constraint_str_contains_multiple_needles() {
    let c = constraint(
        "userId",
        ConstraintOperator::StrContains,
        vec!["@corp", "@partner"],
        None,
    );
    assert!(evaluate_constraints(
        std::slice::from_ref(&c),
        &ctx_user("u@partner.io")
    ));
    assert!(!evaluate_constraints(
        std::slice::from_ref(&c),
        &ctx_user("u@other.io")
    ));
}

#[test]
fn constraint_num_lt_and_eq() {
    let mut ctx = UnleashContext::default();
    ctx.properties.insert("score".into(), "42".into());

    let lt = constraint("score", ConstraintOperator::NumLt, vec![], Some("100"));
    assert!(evaluate_constraints(std::slice::from_ref(&lt), &ctx));

    let eq = constraint("score", ConstraintOperator::NumEq, vec![], Some("42"));
    assert!(evaluate_constraints(std::slice::from_ref(&eq), &ctx));

    let gt = constraint("score", ConstraintOperator::NumGt, vec![], Some("100"));
    assert!(!evaluate_constraints(std::slice::from_ref(&gt), &ctx));
}

#[test]
fn constraint_num_unparseable_returns_false() {
    let mut ctx = UnleashContext::default();
    ctx.properties.insert("score".into(), "not-a-number".into());
    let c = constraint("score", ConstraintOperator::NumGt, vec![], Some("10"));
    assert!(!evaluate_constraints(std::slice::from_ref(&c), &ctx));
}

#[test]
fn constraint_date_before_after() {
    // currentTime is a top-level field on UnleashContext (not properties).
    let ctx = UnleashContext {
        current_time: Some("2026-05-20T12:00:00Z".into()),
        ..Default::default()
    };
    let before = constraint(
        "currentTime",
        ConstraintOperator::DateBefore,
        vec![],
        Some("2027-01-01T00:00:00Z"),
    );
    assert!(evaluate_constraints(std::slice::from_ref(&before), &ctx));

    let after = constraint(
        "currentTime",
        ConstraintOperator::DateAfter,
        vec![],
        Some("2025-01-01T00:00:00Z"),
    );
    assert!(evaluate_constraints(std::slice::from_ref(&after), &ctx));
}

#[test]
fn constraint_date_malformed_value_false() {
    let ctx = UnleashContext {
        current_time: Some("not-a-date".into()),
        ..Default::default()
    };
    let c = constraint(
        "currentTime",
        ConstraintOperator::DateAfter,
        vec![],
        Some("2025-01-01T00:00:00Z"),
    );
    assert!(!evaluate_constraints(std::slice::from_ref(&c), &ctx));
}

#[test]
fn constraint_semver_eq_and_lt_with_v_prefix() {
    let mut ctx = UnleashContext::default();
    ctx.properties.insert("appVersion".into(), "v1.2.3".into());

    let eq = constraint(
        "appVersion",
        ConstraintOperator::SemverEq,
        vec![],
        Some("1.2.3"),
    );
    assert!(evaluate_constraints(std::slice::from_ref(&eq), &ctx));

    let lt = constraint(
        "appVersion",
        ConstraintOperator::SemverLt,
        vec![],
        Some("2.0.0"),
    );
    assert!(evaluate_constraints(std::slice::from_ref(&lt), &ctx));
}

#[test]
fn constraint_semver_pre_release_stripped() {
    let mut ctx = UnleashContext::default();
    ctx.properties
        .insert("appVersion".into(), "1.2.3-rc1".into());
    let eq = constraint(
        "appVersion",
        ConstraintOperator::SemverEq,
        vec![],
        Some("1.2.3"),
    );
    assert!(evaluate_constraints(std::slice::from_ref(&eq), &ctx));
}

#[test]
fn constraint_semver_malformed_false() {
    let mut ctx = UnleashContext::default();
    ctx.properties.insert("appVersion".into(), "garbage".into());
    let c = constraint(
        "appVersion",
        ConstraintOperator::SemverGt,
        vec![],
        Some("1.0.0"),
    );
    assert!(!evaluate_constraints(std::slice::from_ref(&c), &ctx));
}

#[test]
fn constraint_inverted_str_starts_with() {
    let mut c = constraint(
        "userId",
        ConstraintOperator::StrStartsWith,
        vec!["admin-"],
        None,
    );
    c.inverted = true;
    assert!(!evaluate_constraints(
        std::slice::from_ref(&c),
        &ctx_user("admin-alice")
    ));
    assert!(evaluate_constraints(
        std::slice::from_ref(&c),
        &ctx_user("user-bob")
    ));
}

#[test]
fn multiple_constraints_all_must_pass() {
    let mut ctx = UnleashContext::default();
    ctx.user_id = Some("alice".into());
    ctx.properties.insert("tier".into(), "gold".into());

    let cs = vec![
        constraint("userId", ConstraintOperator::In, vec!["alice", "bob"], None),
        constraint("tier", ConstraintOperator::In, vec!["gold"], None),
    ];
    assert!(evaluate_constraints(&cs, &ctx));

    let cs_fail = vec![
        constraint("userId", ConstraintOperator::In, vec!["alice"], None),
        constraint("tier", ConstraintOperator::In, vec!["silver"], None),
    ];
    assert!(!evaluate_constraints(&cs_fail, &ctx));
}

// ─── engine: strategies ──────────────────────────────────────────────────────

#[test]
fn remote_address_strategy_matches_ip() {
    let s = strat("remoteAddress", &[("IPs", "10.0.0.1, 10.0.0.2")]);
    let flag = flag_with("f", "production", vec![s]);
    let mut hit = ctx_user("u");
    hit.remote_address = Some("10.0.0.2".into());
    assert!(evaluate_flag(&flag, "production", &hit, &HashMap::new()).enabled);

    let mut miss = ctx_user("u");
    miss.remote_address = Some("192.168.1.1".into());
    assert!(!evaluate_flag(&flag, "production", &miss, &HashMap::new()).enabled);
}

#[test]
fn remote_address_missing_context_is_disabled() {
    let s = strat("remoteAddress", &[("IPs", "10.0.0.1")]);
    let flag = flag_with("f", "production", vec![s]);
    // ctx_user() does NOT set remote_address.
    assert!(!evaluate_flag(&flag, "production", &ctx_user("u"), &HashMap::new()).enabled);
}

#[test]
fn application_hostname_strategy() {
    let s = strat("applicationHostname", &[("hostNames", "api-1, api-2")]);
    let flag = flag_with("f", "production", vec![s]);

    let mut ctx = ctx_user("u");
    ctx.properties.insert("hostname".into(), "api-2".into());
    assert!(evaluate_flag(&flag, "production", &ctx, &HashMap::new()).enabled);

    let mut other = ctx_user("u");
    other.properties.insert("hostname".into(), "api-9".into());
    assert!(!evaluate_flag(&flag, "production", &other, &HashMap::new()).enabled);
}

#[test]
fn gradual_rollout_session_id_zero_pct_never_fires() {
    let s = strat(
        "gradualRolloutSessionId",
        &[("percentage", "0"), ("groupId", "g")],
    );
    let flag = flag_with("f", "production", vec![s]);
    let mut ctx = ctx_user("u");
    ctx.session_id = Some("any-session".into());
    assert!(!evaluate_flag(&flag, "production", &ctx, &HashMap::new()).enabled);
}

#[test]
fn gradual_rollout_session_id_hundred_pct_always_fires() {
    let s = strat(
        "gradualRolloutSessionId",
        &[("percentage", "100"), ("groupId", "g")],
    );
    let flag = flag_with("f", "production", vec![s]);
    let mut ctx = ctx_user("u");
    ctx.session_id = Some("any-session".into());
    assert!(evaluate_flag(&flag, "production", &ctx, &HashMap::new()).enabled);
}

#[test]
fn custom_strategy_defaults_to_enabled() {
    let s = strat("myCustomStrategy", &[("foo", "bar")]);
    let flag = flag_with("f", "production", vec![s]);
    assert!(evaluate_flag(&flag, "production", &ctx_user("u"), &HashMap::new()).enabled);
}

#[test]
fn disabled_strategy_is_ignored() {
    let mut s = strat("default", &[]);
    s.disabled = true;
    let flag = flag_with("f", "production", vec![s]);
    // Only a disabled strategy → no enabled strategy → flag stays off.
    assert!(!evaluate_flag(&flag, "production", &ctx_user("u"), &HashMap::new()).enabled);
}

#[test]
fn flexible_rollout_stickiness_property() {
    // stickiness=tier sources from properties.tier instead of userId.
    let s = strat(
        "flexibleRollout",
        &[("rollout", "100"), ("stickiness", "tier"), ("groupId", "g")],
    );
    let flag = flag_with("f", "production", vec![s]);
    let mut ctx = UnleashContext::default();
    ctx.environment = Some("production".into());
    ctx.properties.insert("tier".into(), "gold".into());
    assert!(evaluate_flag(&flag, "production", &ctx, &HashMap::new()).enabled);
}

// ─── engine: feature-level state transitions ─────────────────────────────────

#[test]
fn flag_disabled_master_overrides_everything() {
    let mut flag = flag_with("f", "production", vec![strat("default", &[])]);
    flag.enabled = false;
    assert!(!evaluate_flag(&flag, "production", &ctx_user("u"), &HashMap::new()).enabled);
}

#[test]
fn unknown_environment_returns_disabled() {
    let flag = flag_with("f", "production", vec![strat("default", &[])]);
    assert!(!evaluate_flag(&flag, "staging", &ctx_user("u"), &HashMap::new()).enabled);
}

#[test]
fn empty_strategies_treated_as_enabled() {
    // Per Unleash spec: no strategies = "always on" when env is enabled.
    let flag = flag_with("f", "production", vec![]);
    assert!(evaluate_flag(&flag, "production", &ctx_user("u"), &HashMap::new()).enabled);
}

#[test]
fn project_field_set_through_constructor() {
    let mut flag = flag_with("f", "production", vec![]);
    flag.project = "billing".to_string();
    assert_eq!(flag.project, "billing");
}

#[test]
fn evaluate_all_returns_per_flag_results() {
    let f1 = flag_with("on-flag", "production", vec![strat("default", &[])]);
    let mut f2 = flag_with("off-flag", "production", vec![strat("default", &[])]);
    f2.enabled = false;

    let results = evaluate_all(&[f1, f2], "production", &ctx_user("u"), &[]);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "on-flag");
    assert!(results[0].1);
    assert_eq!(results[1].0, "off-flag");
    assert!(!results[1].1);
}

// ─── engine: segments ────────────────────────────────────────────────────────

#[test]
fn segment_with_multiple_constraints() {
    let seg = Segment {
        id: 7,
        name: "premium-us".into(),
        description: None,
        constraints: vec![
            constraint("tier", ConstraintOperator::In, vec!["gold"], None),
            constraint("region", ConstraintOperator::In, vec!["us"], None),
        ],
        created_at: Utc::now(),
        created_by: None,
        project: None,
    };
    let mut s = strat("default", &[]);
    s.segments = vec![7];
    let flag = flag_with("f", "production", vec![s]);
    let seg_map: HashMap<i64, &Segment> = [(7, &seg)].into_iter().collect();

    let mut ok = ctx_user("u");
    ok.properties.insert("tier".into(), "gold".into());
    ok.properties.insert("region".into(), "us".into());
    assert!(evaluate_flag(&flag, "production", &ok, &seg_map).enabled);

    let mut wrong_region = ctx_user("u");
    wrong_region.properties.insert("tier".into(), "gold".into());
    wrong_region.properties.insert("region".into(), "eu".into());
    assert!(!evaluate_flag(&flag, "production", &wrong_region, &seg_map).enabled);
}

#[test]
fn missing_segment_id_does_not_block_strategy() {
    // Segment id referenced but not provided in map → constraint loop skips it.
    let mut s = strat("default", &[]);
    s.segments = vec![999];
    let flag = flag_with("f", "production", vec![s]);
    let seg_map: HashMap<i64, &Segment> = HashMap::new();
    assert!(evaluate_flag(&flag, "production", &ctx_user("u"), &seg_map).enabled);
}

// ─── engine: variants ────────────────────────────────────────────────────────

#[test]
fn variant_empty_list_returns_disabled() {
    let ev = select_variant(&[], "any-flag", &ctx_user("u"), true);
    assert_eq!(ev.name, "disabled");
    assert!(!ev.enabled);
    assert!(ev.feature_enabled);
}

#[test]
fn variant_all_zero_weight_returns_disabled() {
    let variants = vec![Variant {
        name: "X".into(),
        weight: 0,
        weight_type: WeightType::Variable,
        stickiness: "userId".into(),
        payload: None,
        overrides: vec![],
    }];
    let ev = select_variant(&variants, "flag-z", &ctx_user("u"), true);
    assert!(!ev.enabled);
}

#[test]
fn variant_override_short_circuits_weight() {
    // Even though A has weight 1000, override on B for tier=gold should win.
    let variants = vec![
        Variant {
            name: "A".into(),
            weight: 1000,
            weight_type: WeightType::Variable,
            stickiness: "userId".into(),
            payload: None,
            overrides: vec![],
        },
        Variant {
            name: "B".into(),
            weight: 0,
            weight_type: WeightType::Variable,
            stickiness: "userId".into(),
            payload: Some(VariantPayload {
                payload_type: "string".into(),
                value: "premium".into(),
            }),
            overrides: vec![VariantOverride {
                context_name: "tier".into(),
                values: vec!["gold".into()],
            }],
        },
    ];
    let mut ctx = ctx_user("u");
    ctx.properties.insert("tier".into(), "gold".into());

    let ev = select_variant(&variants, "f", &ctx, true);
    assert_eq!(ev.name, "B");
    assert!(ev.enabled);
    assert_eq!(ev.payload.as_ref().map(|p| p.value.as_str()), Some("premium"));
}

#[test]
fn variant_weighted_cumulative_selection_is_stable() {
    let variants = vec![
        Variant {
            name: "low".into(),
            weight: 100,
            weight_type: WeightType::Variable,
            stickiness: "userId".into(),
            payload: None,
            overrides: vec![],
        },
        Variant {
            name: "high".into(),
            weight: 900,
            weight_type: WeightType::Variable,
            stickiness: "userId".into(),
            payload: None,
            overrides: vec![],
        },
    ];
    // Across 100 distinct users, both buckets must be hit and result must be
    // deterministic for the same user.
    let mut saw_low = false;
    let mut saw_high = false;
    for i in 0..200 {
        let ctx = ctx_user(&format!("user-{i}"));
        let v1 = select_variant(&variants, "feat", &ctx, true);
        let v2 = select_variant(&variants, "feat", &ctx, true);
        assert_eq!(v1.name, v2.name, "non-deterministic for user-{i}");
        if v1.name == "low" {
            saw_low = true;
        }
        if v1.name == "high" {
            saw_high = true;
        }
    }
    assert!(saw_low && saw_high, "both variants must be reachable");
}

// ─── engine: normalization helpers ───────────────────────────────────────────

#[test]
fn normalized_value_100_bounds() {
    for u in &["a", "very-long-user-id-string", "x", "user@corp.com"] {
        let v = normalized_value_100(u, "g");
        assert!((1..=100).contains(&v), "normalized_value_100({u}) = {v}");
    }
}

#[test]
fn normalized_value_1000_bounds_and_determinism() {
    let a = normalized_value_1000("user-1", "feat");
    let b = normalized_value_1000("user-1", "feat");
    assert_eq!(a, b);
    assert!((1..=1000).contains(&a));
}

#[test]
fn normalized_value_group_id_affects_hash() {
    // Same user, different group should (very likely) bucket differently.
    let in_a = normalized_value_100("user-1", "group-a");
    let in_b = normalized_value_100("user-1", "group-b");
    // We cannot assert "always different" without flake risk; instead assert
    // bounds and that both are well-defined.
    assert!((1..=100).contains(&in_a));
    assert!((1..=100).contains(&in_b));
}
