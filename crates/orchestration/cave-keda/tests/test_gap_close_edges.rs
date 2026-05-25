// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-keda gap-close edge tests.
//!
//! Targets the 10 modules without `#[cfg(test)]` blocks in `src/`:
//!   scaler / scaledobject / scaledjob / trigger_authentication /
//!   cron_scaler / http_scaler / kafka_scaler / prometheus_scaler /
//!   redis_scaler / cpu_memory_scaler
//!
//! Focuses on failure modes, boundary conditions, and state transitions
//! that the in-module #[test]s in `lib.rs` and the Phase-2 integration
//! suite do not exercise.

use cave_keda::cpu_memory_scaler::{CpuScaler, MemoryScaler, ResourceMetricType};
use cave_keda::cron_scaler::{CronScaler, validate_cron};
use cave_keda::http_scaler::HttpScaler;
use cave_keda::kafka_scaler::KafkaScaler;
use cave_keda::prometheus_scaler::PrometheusScaler;
use cave_keda::redis_scaler::{RedisDataType, RedisScaler};
use cave_keda::scaledjob::{ScaledJob, ScalingStrategy};
use cave_keda::scaledobject::ScaledObject;
use cave_keda::scaler::{Scaler, ScalerTrait, ScalingModifiers, replicas_from_metric};
use cave_keda::trigger_authentication::{
    EnvTargetRef, SecretTargetRef, TriggerAuthentication,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

// ─── scaler.rs: replicas_from_metric boundary cases ─────────────────────────

#[test]
fn replicas_from_metric_negative_metric_floored_to_zero() {
    // metric / target = negative → ceil → still negative → .max(0.0) → 0
    assert_eq!(replicas_from_metric(-50.0, 100.0), 0);
}

#[test]
fn replicas_from_metric_negative_target_returns_zero() {
    // negative target hits the guard path; mirror of zero-target.
    assert_eq!(replicas_from_metric(100.0, -1.0), 0);
}

#[test]
fn replicas_from_metric_fractional_ceil_up() {
    // 0.1 -> ceil(0.001) = 1
    assert_eq!(replicas_from_metric(0.1, 100.0), 1);
    // 99.9 -> ceil(0.999) = 1
    assert_eq!(replicas_from_metric(99.9, 100.0), 1);
    // 100.1 -> ceil(1.001) = 2
    assert_eq!(replicas_from_metric(100.1, 100.0), 2);
}

#[test]
fn scaler_scale_to_zero_noop_preserves_fields() {
    let mut s = Scaler::new("alpha");
    s.fallback_replicas = Some(4);
    s.scale_to_zero();
    // scale_to_zero is documented as no-op at this layer.
    assert_eq!(s.tenant_id, "alpha");
    assert_eq!(s.fallback_replicas, Some(4));
}

#[test]
fn scaling_modifiers_default_is_empty() {
    let m = ScalingModifiers::default();
    assert!(m.formula.is_none());
    assert!(m.target.is_none());
    assert!(m.activation_target.is_none());
}

#[test]
fn scaler_default_polling_interval_is_thirty() {
    // Default trait method on a struct that doesn't override it.
    // PrometheusScaler is a convenient witness.
    let p = PrometheusScaler::new("t");
    assert_eq!(p.polling_interval(), Duration::from_secs(30));
}

// ─── scaledobject.rs: state transitions ────────────────────────────────────

#[test]
fn scaledobject_active_with_zero_recommended_lifts_to_one() {
    // Active triggers force at least 1 replica even when recommended is 0
    // (min.max(1) clause in reconcile()).
    let mut so = ScaledObject::new("t");
    so.min_replica_count = Some(0);
    let r = so.reconcile(0, true, Instant::now());
    assert_eq!(r, 1);
}

#[test]
fn scaledobject_active_records_last_active_at() {
    let mut so = ScaledObject::new("t");
    let now = Instant::now();
    so.reconcile(2, true, now);
    assert!(so.last_active_at.is_some());
    assert_eq!(so.last_active_at.unwrap(), now);
}

#[test]
fn scaledobject_inactive_never_active_scales_immediately_to_min() {
    // No prior active state → last_active_at = None → skip cooldown.
    let mut so = ScaledObject::new("t");
    so.min_replica_count = Some(1);
    so.current_replicas = 5;
    let r = so.reconcile(0, false, Instant::now());
    assert_eq!(r, 1);
}

#[test]
fn scaledobject_idle_replica_overrides_min_on_scale_down() {
    let mut so = ScaledObject::new("t");
    so.min_replica_count = Some(0);
    so.idle_replica_count = Some(3);
    so.cooldown_period = Some(Duration::from_secs(60));
    let now = Instant::now();
    so.reconcile(5, true, now);
    let later = now + Duration::from_secs(120);
    let r = so.reconcile(0, false, later);
    assert_eq!(r, 3); // idle wins over min
}

#[test]
fn scaledobject_paused_ignores_active_recommendation() {
    let mut so = ScaledObject::new("t");
    so.current_replicas = 4;
    so.pause();
    let r = so.reconcile(99, true, Instant::now());
    assert_eq!(r, 4);
}

#[test]
fn scaledobject_scale_to_zero_with_neither_set_returns_zero() {
    let mut so = ScaledObject::new("t");
    so.idle_replica_count = None;
    so.min_replica_count = None;
    so.current_replicas = 7;
    so.scale_to_zero();
    assert_eq!(so.current_replicas, 0);
}

// ─── scaledjob.rs: strategy edges ──────────────────────────────────────────

#[test]
fn scalingstrategy_default_variant_is_default() {
    let s: ScalingStrategy = Default::default();
    assert_eq!(s, ScalingStrategy::Default);
}

#[test]
fn scaledjob_default_strategy_negative_queue_clamps_to_zero() {
    // queue_length negative → .clamp(0, max) → 0
    let sj = ScaledJob::new("t");
    assert_eq!(sj.jobs_to_spawn(-100), 0);
}

#[test]
fn scaledjob_default_strategy_zero_queue_returns_zero() {
    let sj = ScaledJob::new("t");
    assert_eq!(sj.jobs_to_spawn(0), 0);
}

#[test]
fn scaledjob_custom_strategy_running_exceeds_queue_clamps_zero() {
    let mut sj = ScaledJob::new("t");
    sj.scaling_strategy = ScalingStrategy::Custom;
    sj.running_jobs = 50;
    assert_eq!(sj.jobs_to_spawn(10), 0);
}

#[test]
fn scaledjob_failed_history_trimmed_to_limit() {
    let mut sj = ScaledJob::new("t");
    sj.failed_jobs_history_limit = Some(3);
    for i in 0..7 {
        sj.record_outcome(&format!("f{i}"), false);
    }
    assert_eq!(sj.failed_jobs.len(), 3);
    // Most recent 3 retained (oldest evicted).
    assert_eq!(sj.failed_jobs[0], "f4");
    assert_eq!(sj.failed_jobs[2], "f6");
}

#[test]
fn scaledjob_history_limit_zero_drops_everything() {
    let mut sj = ScaledJob::new("t");
    sj.successful_jobs_history_limit = Some(0);
    sj.record_outcome("only", true);
    assert!(sj.successful_jobs.is_empty());
}

// ─── trigger_authentication.rs: resolution precedence ──────────────────────

#[test]
fn trigger_auth_resolve_clears_previous_resolution() {
    // Calling resolve() twice must rebuild from scratch — stale params drop.
    let mut ta = TriggerAuthentication::new("t");
    ta.add_secret_ref("p", "s", "k");
    let mut secrets = HashMap::new();
    let mut inner = HashMap::new();
    inner.insert("k".to_string(), "v1".to_string());
    secrets.insert("s".to_string(), inner);
    ta.resolve(&secrets, &HashMap::new());
    assert_eq!(ta.parameter("p"), Some("v1"));
    // Second call with empty stores must drop the resolved value.
    ta.resolve(&HashMap::new(), &HashMap::new());
    assert!(ta.parameter("p").is_none());
}

#[test]
fn trigger_auth_env_overrides_secret_when_both_target_same_param() {
    // When both a secret_ref and env_ref bind the same parameter, env wins
    // (resolve iterates env after secrets and does an unconditional insert).
    let mut ta = TriggerAuthentication::new("t");
    ta.add_secret_ref("token", "s", "k");
    ta.env_target_ref.push(EnvTargetRef {
        parameter: "token".to_string(),
        name: "TOKEN".to_string(),
        container_name: None,
    });
    let mut secrets = HashMap::new();
    let mut inner = HashMap::new();
    inner.insert("k".to_string(), "from-secret".to_string());
    secrets.insert("s".to_string(), inner);
    let env: HashMap<String, String> =
        [("TOKEN".to_string(), "from-env".to_string())]
            .into_iter()
            .collect();
    ta.resolve(&secrets, &env);
    assert_eq!(ta.parameter("token"), Some("from-env"));
}

#[test]
fn trigger_auth_secret_target_ref_default_fields_empty() {
    let r = SecretTargetRef::default();
    assert!(r.parameter.is_empty());
    assert!(r.name.is_empty());
    assert!(r.key.is_empty());
}

#[test]
fn trigger_auth_unknown_parameter_returns_none() {
    let ta = TriggerAuthentication::new("t");
    assert!(ta.parameter("nope").is_none());
}

// ─── cron_scaler.rs: validate_cron edges ───────────────────────────────────

#[test]
fn validate_cron_empty_string_is_zero_fields() {
    let err = validate_cron("").unwrap_err();
    assert!(err.contains("expected 5 cron fields"));
}

#[test]
fn validate_cron_whitespace_only_collapses_to_zero_fields() {
    assert!(validate_cron("   \t  ").is_err());
}

#[test]
fn validate_cron_step_value_accepted() {
    assert!(validate_cron("*/15 */2 */1 */1 */1").is_ok());
}

#[test]
fn validate_cron_step_value_non_numeric_rejected() {
    let err = validate_cron("*/foo * * * *").unwrap_err();
    assert!(err.contains("invalid step value"));
}

#[test]
fn validate_cron_range_inverted_rejected() {
    // a > b → out of bounds error.
    assert!(validate_cron("0 17-9 * * *").is_err());
}

#[test]
fn validate_cron_dow_boundary_six_accepted_seven_rejected() {
    // dow range is 0..=6 (Sunday=0..Saturday=6).
    assert!(validate_cron("0 0 * * 6").is_ok());
    assert!(validate_cron("0 0 * * 7").is_err());
}

#[test]
fn validate_cron_dom_one_indexed_zero_rejected() {
    // day-of-month range starts at 1.
    assert!(validate_cron("0 0 0 * *").is_err());
    assert!(validate_cron("0 0 1 * *").is_ok());
}

#[test]
fn cron_scaler_default_active_window_nine_to_five() {
    let c = CronScaler::new("t");
    assert_eq!(c.start_schedule, "0 9 * * *");
    assert_eq!(c.end_schedule, "0 17 * * *");
    assert_eq!(c.timezone, "UTC");
}

#[test]
fn cron_scaler_polling_interval_overridden_to_sixty() {
    let c = CronScaler::new("t");
    assert_eq!(c.polling_interval(), Duration::from_secs(60));
}

#[test]
fn cron_scaler_no_desired_replicas_active_returns_none() {
    let mut c = CronScaler::new("t");
    c.desired_replicas = None;
    c.set_active(true);
    // Active but no target → metric_value is None.
    assert_eq!(c.metric_value(), None);
    assert!(c.is_active());
}

// ─── http_scaler.rs: pending request semantics ─────────────────────────────

#[test]
fn http_scaler_zero_pending_is_inactive() {
    let s = HttpScaler::new("t");
    assert!(!s.is_active());
    assert_eq!(s.metric_value(), Some(0.0));
}

#[test]
fn http_scaler_polling_interval_overridden_to_fifteen() {
    let s = HttpScaler::new("t");
    assert_eq!(s.polling_interval(), Duration::from_secs(15));
}

#[test]
fn http_scaler_observe_replaces_not_accumulates() {
    let mut s = HttpScaler::new("t");
    s.observe(100);
    s.observe(40);
    assert_eq!(s.current_pending_requests, 40);
}

// ─── kafka_scaler.rs: lag math edges ───────────────────────────────────────

#[test]
fn kafka_scaler_record_lag_clamps_negative() {
    let mut k = KafkaScaler::new("t");
    k.record_lag(0, -42);
    assert_eq!(k.partition_lag.get(&0), Some(&0));
}

#[test]
fn kafka_scaler_record_lag_overwrites_partition() {
    let mut k = KafkaScaler::new("t");
    k.record_lag(0, 100);
    k.record_lag(0, 25); // overwrite, not accumulate
    assert_eq!(k.total_lag(), 25);
}

#[test]
fn kafka_scaler_empty_partition_map_zero_recommended() {
    let k = KafkaScaler::new("t");
    assert_eq!(k.recommended_replicas(), 0);
    assert_eq!(k.total_lag(), 0);
}

#[test]
fn kafka_scaler_threshold_zero_falls_back_to_partition_count() {
    // Guard branch: threshold <= 0 → recommended = partition count.
    let mut k = KafkaScaler::new("t");
    k.lag_threshold = Some(0);
    k.record_lag(0, 100);
    k.record_lag(1, 100);
    k.record_lag(2, 100);
    assert_eq!(k.recommended_replicas(), 3);
}

#[test]
fn kafka_scaler_activation_threshold_via_trait() {
    let mut k = KafkaScaler::new("t");
    k.activation_lag_threshold = Some(42);
    assert_eq!(k.activation_threshold(), 42.0);
}

// ─── prometheus_scaler.rs: NaN + activation ────────────────────────────────

#[test]
fn prom_scaler_observe_infinity_passes_through() {
    // Only NaN is sanitised — infinity is not (documented behaviour).
    let mut p = PrometheusScaler::new("t");
    p.observe(f64::INFINITY);
    assert_eq!(p.current_value, f64::INFINITY);
}

#[test]
fn prom_scaler_observe_negative_passes_through() {
    let mut p = PrometheusScaler::new("t");
    p.observe(-10.0);
    assert_eq!(p.current_value, -10.0);
}

#[test]
fn prom_scaler_activation_threshold_trait_method() {
    let mut p = PrometheusScaler::new("t");
    p.activation_threshold = 7.5;
    assert_eq!(ScalerTrait::activation_threshold(&p), 7.5);
}

#[test]
fn prom_scaler_exact_activation_threshold_is_inactive() {
    // Strict > comparison — equal is inactive.
    let mut p = PrometheusScaler::new("t");
    p.activation_threshold = 10.0;
    p.observe(10.0);
    assert!(!p.is_active());
}

// ─── redis_scaler.rs: data type + activation ───────────────────────────────

#[test]
fn redis_scaler_stream_data_type_persists() {
    let mut r = RedisScaler::new("t");
    r.data_type = RedisDataType::Stream;
    assert_eq!(r.data_type, RedisDataType::Stream);
}

#[test]
fn redis_scaler_negative_observation_clamps_zero() {
    let mut r = RedisScaler::new("t");
    r.observe(-25);
    assert_eq!(r.current_length, 0);
    assert!(!r.is_active());
}

#[test]
fn redis_scaler_activation_threshold_trait_matches_field() {
    let mut r = RedisScaler::new("t");
    r.activation_list_length = 9;
    assert_eq!(r.activation_threshold(), 9.0);
}

// ─── cpu_memory_scaler.rs: defaults + state ────────────────────────────────

#[test]
fn memory_scaler_default_target_and_metric_type() {
    let m = MemoryScaler::new("t");
    assert_eq!(m.target, 80);
    assert_eq!(m.metric_type, ResourceMetricType::Utilization);
}

#[test]
fn memory_scaler_average_value_metric_type_persists() {
    let mut m = MemoryScaler::new("t");
    m.metric_type = ResourceMetricType::AverageValue;
    assert_eq!(m.metric_type, ResourceMetricType::AverageValue);
}

#[test]
fn cpu_scaler_polling_interval_is_fifteen() {
    let c = CpuScaler::new("t");
    assert_eq!(c.polling_interval(), Duration::from_secs(15));
}

#[test]
fn memory_scaler_active_only_when_positive() {
    let mut m = MemoryScaler::new("t");
    assert!(!m.is_active());
    m.observe(0);
    assert!(!m.is_active()); // strict > 0
    m.observe(1);
    assert!(m.is_active());
}

#[test]
fn resource_metric_type_default_is_utilization() {
    let t: ResourceMetricType = Default::default();
    assert_eq!(t, ResourceMetricType::Utilization);
}
