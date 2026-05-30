// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-keda: KEDA event-driven autoscaler reimplementation.
//!
//! upstream: kedacore/keda v2.x
//!
//! Modules:
//!   scaler                  — Scaler trait + ScalingModifiers + replica math
//!   scaledobject            — ScaledObject CRD + cooldown reconcile
//!   scaledjob               — ScaledJob CRD with Default/Custom/Accurate strategy
//!   trigger_authentication  — TriggerAuthentication CRD with secret/env resolution
//!   cron_scaler             — Schedule-based scaling + cron expression validator
//!   http_scaler             — KEDA HTTP add-on style pending-request scaler
//!   kafka_scaler            — Consumer-group lag scaler (per-partition)
//!   prometheus_scaler       — PromQL value scaler
//!   redis_scaler            — Redis list/stream length scaler
//!   cpu_memory_scaler       — Resource metric (Utilization / AverageValue) scaler

pub mod aws_sqs_scaler;
pub mod azure_eventhub_scaler;
pub mod azure_servicebus_scaler;
pub mod cpu_memory_scaler;
pub mod cron_scaler;
pub mod datadog_scaler;
pub mod etcd_scaler;
pub mod gcp_pubsub_scaler;
pub mod hibernation;
pub mod http_scaler;
pub mod kafka_scaler;
pub mod nats_jetstream_scaler;
pub mod prometheus_scaler;
pub mod redis_scaler;
pub mod scaledjob;
pub mod scaledobject;
pub mod scaler;
pub mod scaling_modifiers;
pub mod splunk_scaler;
pub mod trigger_authentication;

pub use aws_sqs_scaler::AwsSqsScaler;
pub use azure_eventhub_scaler::AzureEventHubScaler;
pub use azure_servicebus_scaler::{AzureServiceBusEntity, AzureServiceBusScaler};
pub use cpu_memory_scaler::{CpuScaler, MemoryScaler, ResourceMetricType};
pub use cron_scaler::{CronScaler, validate_cron};
pub use datadog_scaler::DatadogScaler;
pub use etcd_scaler::EtcdScaler;
pub use gcp_pubsub_scaler::GcpPubSubScaler;
pub use hibernation::{Hibernation, HibernationDecision, HibernationSchedule};
pub use http_scaler::HttpScaler;
pub use kafka_scaler::KafkaScaler;
pub use nats_jetstream_scaler::NatsJetStreamScaler;
pub use prometheus_scaler::PrometheusScaler;
pub use redis_scaler::{RedisDataType, RedisScaler};
pub use scaledjob::{ScaledJob, ScalingStrategy};
pub use scaledobject::ScaledObject;
pub use scaler::{Scaler, ScalerTrait, ScalingModifiers, replicas_from_metric};
pub use scaling_modifiers::{ScalingModifiersEvaluator, Trigger};
pub use splunk_scaler::{SearchResponse, SplunkScaler, SplunkValidationError};
pub use trigger_authentication::{EnvTargetRef, SecretTargetRef, TriggerAuthentication};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    // ─── Scaler / replica math ───────────────────────────────

    #[test]
    fn replicas_from_metric_basic_ceil() {
        assert_eq!(replicas_from_metric(250.0, 100.0), 3);
        assert_eq!(replicas_from_metric(100.0, 100.0), 1);
        assert_eq!(replicas_from_metric(0.0, 100.0), 0);
    }

    #[test]
    fn replicas_from_metric_zero_target_safe() {
        assert_eq!(replicas_from_metric(50.0, 0.0), 0);
    }

    #[test]
    fn scaler_new_records_tenant_id_and_defaults() {
        let s = Scaler::new("tenant-x");
        assert_eq!(s.tenant_id, "tenant-x");
        assert_eq!(s.polling_interval, Some(Duration::from_secs(30)));
        assert_eq!(s.cooldown_period, Some(Duration::from_secs(300)));
    }

    #[test]
    fn scaler_fallback_returns_configured_replicas() {
        let mut s = Scaler::new("t");
        s.fallback_replicas = Some(2);
        assert_eq!(s.fallback(), Some(2));
    }

    // ─── ScaledObject reconciliation ─────────────────────────

    #[test]
    fn scaledobject_reconcile_active_recommends_max_of_min_and_recommended() {
        let mut so = ScaledObject::new("t");
        so.min_replica_count = Some(2);
        so.max_replica_count = Some(10);
        let r = so.reconcile(5, true, Instant::now());
        assert_eq!(r, 5);
    }

    #[test]
    fn scaledobject_reconcile_active_clamps_to_max() {
        let mut so = ScaledObject::new("t");
        so.max_replica_count = Some(3);
        let r = so.reconcile(99, true, Instant::now());
        assert_eq!(r, 3);
    }

    #[test]
    fn scaledobject_reconcile_active_lifts_to_min_floor() {
        let mut so = ScaledObject::new("t");
        so.min_replica_count = Some(4);
        let r = so.reconcile(1, true, Instant::now());
        assert_eq!(r, 4);
    }

    #[test]
    fn scaledobject_inactive_within_cooldown_holds_replicas() {
        let mut so = ScaledObject::new("t");
        so.cooldown_period = Some(Duration::from_secs(60));
        let now = Instant::now();
        so.reconcile(3, true, now);
        // 30s later, still inactive — still in cooldown
        let later = now + Duration::from_secs(30);
        let r = so.reconcile(0, false, later);
        assert_eq!(r, 3);
    }

    #[test]
    fn scaledobject_inactive_past_cooldown_scales_to_min() {
        let mut so = ScaledObject::new("t");
        so.cooldown_period = Some(Duration::from_secs(60));
        so.min_replica_count = Some(0);
        let now = Instant::now();
        so.reconcile(3, true, now);
        let later = now + Duration::from_secs(120);
        let r = so.reconcile(0, false, later);
        assert_eq!(r, 0);
    }

    #[test]
    fn scaledobject_paused_freezes_replicas() {
        let mut so = ScaledObject::new("t");
        so.current_replicas = 7;
        so.pause();
        let r = so.reconcile(0, false, Instant::now());
        assert_eq!(r, 7);
    }

    #[test]
    fn scaledobject_resume_restores_reconcile() {
        let mut so = ScaledObject::new("t");
        so.pause();
        so.resume();
        assert!(!so.paused);
    }

    #[test]
    fn scaledobject_scale_to_zero_uses_idle_count() {
        let mut so = ScaledObject::new("t");
        so.idle_replica_count = Some(2);
        so.current_replicas = 10;
        so.scale_to_zero();
        assert_eq!(so.current_replicas, 2);
    }

    #[test]
    fn scaledobject_scale_to_zero_falls_back_to_min() {
        let mut so = ScaledObject::new("t");
        so.idle_replica_count = None;
        so.min_replica_count = Some(1);
        so.current_replicas = 5;
        so.scale_to_zero();
        assert_eq!(so.current_replicas, 1);
    }

    // ─── ScaledJob ───────────────────────────────────────────

    #[test]
    fn scaledjob_default_strategy_caps_at_max_replicas() {
        let mut sj = ScaledJob::new("t");
        sj.max_replica_count = Some(5);
        assert_eq!(sj.jobs_to_spawn(20), 5);
    }

    #[test]
    fn scaledjob_custom_strategy_subtracts_running_jobs() {
        let mut sj = ScaledJob::new("t");
        sj.scaling_strategy = ScalingStrategy::Custom;
        sj.running_jobs = 3;
        sj.max_replica_count = Some(100);
        assert_eq!(sj.jobs_to_spawn(10), 7);
    }

    #[test]
    fn scaledjob_accurate_strategy_floor_zero() {
        let mut sj = ScaledJob::new("t");
        sj.scaling_strategy = ScalingStrategy::Accurate;
        sj.running_jobs = 100;
        assert_eq!(sj.jobs_to_spawn(10), 0);
    }

    #[test]
    fn scaledjob_history_trimmed_to_limit() {
        let mut sj = ScaledJob::new("t");
        sj.successful_jobs_history_limit = Some(2);
        for i in 0..5 {
            sj.record_outcome(&format!("job-{i}"), true);
        }
        assert_eq!(sj.successful_jobs.len(), 2);
        assert_eq!(sj.successful_jobs[0], "job-3");
        assert_eq!(sj.successful_jobs[1], "job-4");
    }

    #[test]
    fn scaledjob_failed_history_separate_from_successful() {
        let mut sj = ScaledJob::new("t");
        sj.record_outcome("ok", true);
        sj.record_outcome("nope", false);
        assert_eq!(sj.successful_jobs.len(), 1);
        assert_eq!(sj.failed_jobs.len(), 1);
    }

    // ─── TriggerAuthentication ───────────────────────────────

    #[test]
    fn trigger_auth_resolves_from_secret_store() {
        let mut ta = TriggerAuthentication::new("t");
        ta.add_secret_ref("password", "redis-creds", "REDIS_PASS");
        let mut secrets = HashMap::new();
        let mut redis_creds = HashMap::new();
        redis_creds.insert("REDIS_PASS".to_string(), "s3cret".to_string());
        secrets.insert("redis-creds".to_string(), redis_creds);
        ta.resolve(&secrets, &HashMap::new());
        assert_eq!(ta.parameter("password"), Some("s3cret"));
    }

    #[test]
    fn trigger_auth_resolves_from_env_store() {
        let mut ta = TriggerAuthentication::new("t");
        ta.env_target_ref.push(EnvTargetRef {
            parameter: "endpoint".to_string(),
            name: "API_ENDPOINT".to_string(),
            container_name: None,
        });
        let env: HashMap<String, String> =
            [("API_ENDPOINT".to_string(), "https://api/".to_string())]
                .into_iter()
                .collect();
        ta.resolve(&HashMap::new(), &env);
        assert_eq!(ta.parameter("endpoint"), Some("https://api/"));
    }

    #[test]
    fn trigger_auth_inline_fallback_only_when_unresolved() {
        let mut ta = TriggerAuthentication::new("t");
        ta.add_secret_ref("p1", "s", "k"); // no value present
        ta.inline.insert("p2".to_string(), "inline".to_string());
        ta.inline.insert("p1".to_string(), "fallback".to_string()); // should NOT win if secret resolves
        let mut secrets = HashMap::new();
        let mut s = HashMap::new();
        s.insert("k".to_string(), "from-secret".to_string());
        secrets.insert("s".to_string(), s);
        ta.resolve(&secrets, &HashMap::new());
        assert_eq!(ta.parameter("p1"), Some("from-secret"));
        assert_eq!(ta.parameter("p2"), Some("inline"));
    }

    #[test]
    fn trigger_auth_missing_secret_does_not_resolve() {
        let mut ta = TriggerAuthentication::new("t");
        ta.add_secret_ref("p", "missing-secret", "k");
        ta.resolve(&HashMap::new(), &HashMap::new());
        assert!(ta.parameter("p").is_none());
    }

    // ─── CronScaler + cron validation ────────────────────────

    #[test]
    fn validate_cron_accepts_standard_5_field() {
        assert!(validate_cron("0 9 * * *").is_ok());
        assert!(validate_cron("*/5 * * * *").is_ok());
        assert!(validate_cron("0 0-12 1,15 * 1-5").is_ok());
    }

    #[test]
    fn validate_cron_rejects_wrong_field_count() {
        assert!(validate_cron("0 9 * *").is_err());
        assert!(validate_cron("0 9 * * * *").is_err());
    }

    #[test]
    fn validate_cron_rejects_out_of_range_value() {
        assert!(validate_cron("60 0 * * *").is_err()); // minute > 59
        assert!(validate_cron("0 24 * * *").is_err()); // hour > 23
    }

    #[test]
    fn cron_scaler_active_returns_desired_replicas() {
        let mut cs = CronScaler::new("t");
        cs.desired_replicas = Some(3);
        cs.set_active(true);
        assert_eq!(cs.metric_value(), Some(3.0));
        assert!(cs.is_active());
    }

    #[test]
    fn cron_scaler_inactive_metric_zero() {
        let cs = CronScaler::new("t");
        assert_eq!(cs.metric_value(), Some(0.0));
        assert!(!cs.is_active());
    }

    // ─── HttpScaler ──────────────────────────────────────────

    #[test]
    fn http_scaler_active_when_pending_above_zero() {
        let mut s = HttpScaler::new("t");
        s.observe(50);
        assert!(s.is_active());
        assert_eq!(s.metric_value(), Some(50.0));
    }

    #[test]
    fn http_scaler_observe_clamps_negative() {
        let mut s = HttpScaler::new("t");
        s.observe(-10);
        assert_eq!(s.current_pending_requests, 0);
    }

    // ─── KafkaScaler ─────────────────────────────────────────

    #[test]
    fn kafka_scaler_total_lag_sums_partitions() {
        let mut k = KafkaScaler::new("t");
        k.record_lag(0, 100);
        k.record_lag(1, 200);
        k.record_lag(2, 50);
        assert_eq!(k.total_lag(), 350);
    }

    #[test]
    fn kafka_scaler_recommended_replicas_capped_by_partitions() {
        let mut k = KafkaScaler::new("t");
        k.lag_threshold = Some(10);
        // 2 partitions, lag totals 1000 — math says 100 replicas, partitions cap to 2
        k.record_lag(0, 500);
        k.record_lag(1, 500);
        assert_eq!(k.recommended_replicas(), 2);
    }

    #[test]
    fn kafka_scaler_active_above_activation_threshold() {
        let mut k = KafkaScaler::new("t");
        k.activation_lag_threshold = Some(10);
        k.record_lag(0, 5);
        assert!(!k.is_active());
        k.record_lag(0, 50);
        assert!(k.is_active());
    }

    // ─── PrometheusScaler ────────────────────────────────────

    #[test]
    fn prom_scaler_observe_handles_nan() {
        let mut p = PrometheusScaler::new("t");
        p.observe(f64::NAN);
        assert_eq!(p.current_value, 0.0);
    }

    #[test]
    fn prom_scaler_active_when_above_activation_threshold() {
        let mut p = PrometheusScaler::new("t");
        p.activation_threshold = 10.0;
        p.observe(5.0);
        assert!(!p.is_active());
        p.observe(15.0);
        assert!(p.is_active());
    }

    // ─── RedisScaler ─────────────────────────────────────────

    #[test]
    fn redis_scaler_default_data_type_list() {
        let r = RedisScaler::new("t");
        assert_eq!(r.data_type, RedisDataType::List);
    }

    #[test]
    fn redis_scaler_active_when_length_above_activation() {
        let mut r = RedisScaler::new("t");
        r.activation_list_length = 3;
        r.observe(2);
        assert!(!r.is_active());
        r.observe(4);
        assert!(r.is_active());
    }

    // ─── CPU + Memory scalers ────────────────────────────────

    #[test]
    fn cpu_scaler_active_when_nonzero() {
        let mut c = CpuScaler::new("t");
        assert!(!c.is_active());
        c.observe(40);
        assert!(c.is_active());
        assert_eq!(c.metric_value(), Some(40.0));
    }

    #[test]
    fn memory_scaler_observe_clamps_negative() {
        let mut m = MemoryScaler::new("t");
        m.observe(-100);
        assert_eq!(m.current, 0);
    }

    #[test]
    fn cpu_scaler_default_target_80_percent() {
        let c = CpuScaler::new("t");
        assert_eq!(c.target, 80);
        assert_eq!(c.metric_type, ResourceMetricType::Utilization);
    }
}
