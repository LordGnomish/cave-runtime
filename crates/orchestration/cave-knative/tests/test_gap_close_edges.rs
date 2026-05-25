// SPDX-License-Identifier: AGPL-3.0-or-later
//! Edge-case + boundary coverage for cave-knative.
//!
//! Areas exercised:
//!   * ObjectMeta / annotation constants — creator + last-modifier + autoscaling.knative.dev/*
//!   * validate_traffic / validate_template — boundary error paths
//!   * Configuration / Revision / Route / Ksvc — state transitions + idempotence
//!   * EventingSource / Channel / Trigger — fanout, unsubscribe-of-missing, multi-attr filter
//!   * AutoscalerMetric / Autoscaler — windowing math, panic/stable mode, min/max clamping
//!
//! These complement the inline `mod tests` in src/lib.rs (40 cases) without
//! duplicating coverage. No src/ files were modified.

use cave_knative::{
    Autoscaler, AutoscalerConfig, AutoscalerMetric, AutoscalerMode, Channel, Configuration,
    Container, EnvVar, EventingSink, EventingSource, Ksvc, ObjectMeta, PodSpec, Revision,
    RevisionTemplateSpec, Route, Subscription, TrafficTarget, Trigger, ANNOTATION_AUTOSCALER_CLASS,
    ANNOTATION_CREATOR, ANNOTATION_LAST_MODIFIER, ANNOTATION_MAX_SCALE, ANNOTATION_METRIC,
    ANNOTATION_MIN_SCALE, ANNOTATION_TARGET, validate_template, validate_traffic,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

// ─── tiny helpers ────────────────────────────────────────────────────────────

fn container(name: &str, image: &str) -> Container {
    Container { name: name.to_string(), image: image.to_string(), env: vec![] }
}

fn template(image: &str) -> RevisionTemplateSpec {
    RevisionTemplateSpec {
        metadata: ObjectMeta::default(),
        spec: PodSpec { containers: vec![container("app", image)] },
    }
}

fn rev_target(name: &str, pct: i32) -> TrafficTarget {
    TrafficTarget {
        revision_name: Some(name.to_string()),
        percent: Some(pct),
        ..Default::default()
    }
}

// ─── 1. Annotation-constant stability ────────────────────────────────────────

#[test]
fn annotation_constants_match_upstream_keys() {
    // Pin the public string values — these are the upstream Knative keys,
    // changing them silently would break operator round-trips.
    assert_eq!(ANNOTATION_CREATOR, "knative.dev/creator");
    assert_eq!(ANNOTATION_LAST_MODIFIER, "knative.dev/lastModifier");
    assert_eq!(ANNOTATION_AUTOSCALER_CLASS, "autoscaling.knative.dev/class");
    assert_eq!(ANNOTATION_MIN_SCALE, "autoscaling.knative.dev/minScale");
    assert_eq!(ANNOTATION_MAX_SCALE, "autoscaling.knative.dev/maxScale");
    assert_eq!(ANNOTATION_TARGET, "autoscaling.knative.dev/target");
    assert_eq!(ANNOTATION_METRIC, "autoscaling.knative.dev/metric");
}

#[test]
fn objectmeta_with_creator_records_both_creator_and_modifier() {
    let m = ObjectMeta::with_creator("tenant-X");
    assert_eq!(m.annotations.get(ANNOTATION_CREATOR), Some(&"tenant-X".to_string()));
    assert_eq!(m.annotations.get(ANNOTATION_LAST_MODIFIER), Some(&"tenant-X".to_string()));
    // Default fields stay empty.
    assert!(m.name.is_empty());
    assert!(m.namespace.is_empty());
    assert_eq!(m.generation, 0);
    assert!(m.labels.is_empty());
}

#[test]
fn objectmeta_default_has_no_creator() {
    let m = ObjectMeta::default();
    assert!(m.creator().is_none());
}

// ─── 2. validate_traffic — boundary cases ────────────────────────────────────

#[test]
fn validate_traffic_rejects_undersum() {
    let t = vec![rev_target("a", 50), rev_target("b", 40)];
    let err = validate_traffic(&t).unwrap_err();
    assert!(err.contains("100"));
    assert!(err.contains("90"));
}

#[test]
fn validate_traffic_rejects_oversum() {
    let t = vec![rev_target("a", 60), rev_target("b", 60)];
    let err = validate_traffic(&t).unwrap_err();
    assert!(err.contains("120"));
}

#[test]
fn validate_traffic_rejects_target_without_any_ref() {
    // sums to 100 but no revision/configuration/latest_revision set on the second.
    let t = vec![
        rev_target("a", 50),
        TrafficTarget { percent: Some(50), ..Default::default() },
    ];
    assert!(validate_traffic(&t).is_err());
}

#[test]
fn validate_traffic_accepts_split_with_configuration_ref() {
    let t = vec![
        TrafficTarget {
            configuration_name: Some("cfg-1".to_string()),
            percent: Some(70),
            ..Default::default()
        },
        rev_target("rev-x", 30),
    ];
    assert!(validate_traffic(&t).is_ok());
}

#[test]
fn validate_traffic_treats_missing_percent_as_zero() {
    // `revision_name=Some, percent=None` — sum becomes 0, must reject.
    let t = vec![TrafficTarget {
        revision_name: Some("a".to_string()),
        percent: None,
        ..Default::default()
    }];
    assert!(validate_traffic(&t).is_err());
}

// ─── 3. validate_template — boundary cases ───────────────────────────────────

#[test]
fn validate_template_rejects_empty_containers() {
    let tpl = RevisionTemplateSpec {
        metadata: ObjectMeta::default(),
        spec: PodSpec { containers: vec![] },
    };
    let err = validate_template(&tpl).unwrap_err();
    assert!(err.contains("at least one container"));
}

#[test]
fn validate_template_rejects_when_any_container_image_empty() {
    let tpl = RevisionTemplateSpec {
        metadata: ObjectMeta::default(),
        spec: PodSpec {
            containers: vec![container("sidecar", "envoy:1.30"), container("app", "")],
        },
    };
    assert!(validate_template(&tpl).is_err());
}

#[test]
fn validate_template_accepts_multi_container_pod() {
    let tpl = RevisionTemplateSpec {
        metadata: ObjectMeta::default(),
        spec: PodSpec {
            containers: vec![container("main", "img:v1"), container("sidecar", "envoy:1.30")],
        },
    };
    assert!(validate_template(&tpl).is_ok());
}

// ─── 4. Configuration — state transitions ────────────────────────────────────

#[test]
fn configuration_record_created_overwrites_previous() {
    let mut cfg = Configuration::new("t");
    cfg.record_created_revision("rev-1");
    cfg.record_created_revision("rev-2");
    assert_eq!(cfg.status.latestCreatedRevisionName, Some("rev-2".to_string()));
}

#[test]
fn configuration_record_ready_does_not_overwrite_created() {
    let mut cfg = Configuration::new("t");
    cfg.record_created_revision("rev-new");
    cfg.record_ready_revision("rev-old");
    // Created and ready can diverge during a rollout.
    assert_eq!(cfg.status.latestCreatedRevisionName, Some("rev-new".to_string()));
    assert_eq!(cfg.status.latestReadyRevisionName, Some("rev-old".to_string()));
}

#[test]
fn configuration_scale_to_zero_mirrors_generation_each_call() {
    let mut cfg = Configuration::new("t");
    cfg.metadata.generation = 3;
    cfg.scale_to_zero();
    assert_eq!(cfg.status.observed_generation, 3);
    cfg.metadata.generation = 4;
    cfg.scale_to_zero();
    assert_eq!(cfg.status.observed_generation, 4);
}

// ─── 5. Revision — clamping + validation interaction ─────────────────────────

#[test]
fn revision_set_actual_replicas_clamps_negative_to_zero() {
    let mut rev = Revision::new("t");
    rev.set_actual_replicas(-7);
    assert_eq!(rev.status.actualReplicas, Some(0));
}

#[test]
fn revision_validate_accepts_zero_concurrency_meaning_unbounded() {
    // containerConcurrency=0 is upstream-valid (unbounded).
    let mut rev = Revision::new("t");
    rev.spec.containerConcurrency = Some(0);
    rev.spec.timeoutSeconds = Some(60);
    rev.spec.template = template("img");
    assert!(rev.validate().is_ok());
}

#[test]
fn revision_validate_rejects_negative_timeout() {
    let mut rev = Revision::new("t");
    rev.spec.timeoutSeconds = Some(-1);
    rev.spec.template = template("img");
    assert!(rev.validate().is_err());
}

#[test]
fn revision_validate_bubbles_template_error_when_image_empty() {
    let mut rev = Revision::new("t");
    rev.spec.template = RevisionTemplateSpec {
        metadata: ObjectMeta::default(),
        spec: PodSpec { containers: vec![container("c", "")] },
    };
    let err = rev.validate().unwrap_err();
    assert!(err.contains("image"));
}

// ─── 6. Route — resolve_revision boundary & promote/tag interaction ─────────

#[test]
fn route_resolve_revision_returns_none_when_status_empty() {
    let r = Route::new("t");
    assert!(r.resolve_revision(0).is_none());
}

#[test]
fn route_resolve_revision_handles_zero_percentile() {
    let mut r = Route::new("t");
    r.status.traffic = vec![rev_target("rev-a", 50), rev_target("rev-b", 50)];
    assert_eq!(r.resolve_revision(0), Some("rev-a"));
}

#[test]
fn route_resolve_revision_boundary_between_buckets() {
    let mut r = Route::new("t");
    r.status.traffic = vec![rev_target("rev-a", 50), rev_target("rev-b", 50)];
    // percentile==50 falls into bucket [50,100) — second revision.
    assert_eq!(r.resolve_revision(50), Some("rev-b"));
    // percentile==49 still belongs to bucket [0,50) — first revision.
    assert_eq!(r.resolve_revision(49), Some("rev-a"));
}

#[test]
fn route_resolve_revision_clamps_negative_to_first_bucket() {
    let mut r = Route::new("t");
    r.status.traffic = vec![rev_target("rev-a", 30), rev_target("rev-b", 70)];
    // clamp(0, 99) keeps a negative input inside the first bucket.
    assert_eq!(r.resolve_revision(-5), Some("rev-a"));
}

#[test]
fn route_promote_replaces_previous_traffic_entirely() {
    let mut r = Route::new("t");
    r.spec.traffic = vec![rev_target("rev-old", 100)];
    r.promote("rev-new");
    assert_eq!(r.spec.traffic.len(), 1);
    assert_eq!(r.spec.traffic[0].revision_name, Some("rev-new".to_string()));
}

#[test]
fn route_scale_to_zero_zeroes_every_status_target() {
    let mut r = Route::new("t");
    r.status.traffic = vec![rev_target("a", 60), rev_target("b", 40)];
    r.scale_to_zero();
    assert!(r.status.traffic.iter().all(|t| t.percent == Some(0)));
}

#[test]
fn route_validate_accepts_empty_spec_traffic() {
    // Empty `spec.traffic` means "implicit single-revision route".
    let r = Route::new("t");
    assert!(r.validate().is_ok());
}

// ─── 7. Ksvc — composite validation paths ────────────────────────────────────

#[test]
fn ksvc_validate_accepts_traffic_with_latest_revision_only() {
    let mut svc = Ksvc::new("t");
    svc.spec.template = template("img");
    svc.spec.traffic = vec![TrafficTarget {
        latest_revision: Some(true),
        percent: Some(100),
        ..Default::default()
    }];
    assert!(svc.validate().is_ok());
}

#[test]
fn ksvc_scale_to_zero_is_idempotent() {
    let mut svc = Ksvc::new("t");
    svc.status.traffic = vec![rev_target("rev", 100)];
    svc.scale_to_zero();
    svc.scale_to_zero();
    assert_eq!(svc.status.traffic[0].percent, Some(0));
}

#[test]
fn ksvc_name_defaults_empty_when_metadata_name_unset() {
    let svc = Ksvc::new("tenant");
    assert_eq!(svc.name(), "");
}

// ─── 8. Eventing — Source / Channel / Sink / Trigger ─────────────────────────

#[test]
fn eventing_source_resolve_sink_returns_none_when_spec_empty() {
    let mut s = EventingSource::new("t");
    assert!(s.resolve_sink().is_none());
    assert!(s.status.sinkURI.is_none());
}

#[test]
fn eventing_source_ce_overrides_persist() {
    let mut s = EventingSource::new("t");
    s.add_ce_override("source", "/devices/1");
    s.add_ce_override("type", "com.example.foo");
    assert_eq!(s.spec.ce_overrides.get("source"), Some(&"/devices/1".to_string()));
    assert_eq!(s.spec.ce_overrides.get("type"), Some(&"com.example.foo".to_string()));
    assert_eq!(s.spec.ce_overrides.len(), 2);
}

#[test]
fn eventing_source_add_ce_override_overwrites_existing_key() {
    let mut s = EventingSource::new("t");
    s.add_ce_override("type", "com.example.foo");
    s.add_ce_override("type", "com.example.bar");
    assert_eq!(s.spec.ce_overrides.get("type"), Some(&"com.example.bar".to_string()));
    assert_eq!(s.spec.ce_overrides.len(), 1);
}

#[test]
fn eventing_sink_resolve_address_mirrors_destination() {
    let mut sink = EventingSink::new("t");
    sink.spec.destination = Some("http://broker.svc/in".to_string());
    sink.resolve_address();
    assert_eq!(sink.status.address_url, Some("http://broker.svc/in".to_string()));
}

#[test]
fn eventing_sink_resolve_address_with_no_destination_stays_none() {
    let mut sink = EventingSink::new("t");
    sink.resolve_address();
    assert!(sink.status.address_url.is_none());
}

#[test]
fn channel_unsubscribe_unknown_uid_is_noop() {
    let mut c = Channel::new("t");
    c.subscribe(Subscription {
        uid: "u1".to_string(),
        subscriber_uri: "http://a".to_string(),
        reply_uri: None,
    });
    c.unsubscribe("does-not-exist");
    assert_eq!(c.subscribers.len(), 1);
}

#[test]
fn channel_fanout_preserves_subscription_order_and_duplicates() {
    let mut c = Channel::new("t");
    for uid in ["u1", "u2", "u3"] {
        c.subscribe(Subscription {
            uid: uid.to_string(),
            subscriber_uri: format!("http://{uid}"),
            reply_uri: None,
        });
    }
    // Same URI under a different uid — fanout returns both.
    c.subscribe(Subscription {
        uid: "u4".to_string(),
        subscriber_uri: "http://u1".to_string(),
        reply_uri: None,
    });
    let out = c.fanout();
    assert_eq!(out, vec!["http://u1", "http://u2", "http://u3", "http://u1"]);
}

#[test]
fn trigger_filter_multi_attribute_requires_all_to_match() {
    let mut trig = Trigger::new("t", "default");
    trig.filter.attributes.insert("type".to_string(), "com.example.foo".to_string());
    trig.filter.attributes.insert("source".to_string(), "/devices/1".to_string());

    let matching = HashMap::from([
        ("type".to_string(), "com.example.foo".to_string()),
        ("source".to_string(), "/devices/1".to_string()),
        ("extra".to_string(), "ignored".to_string()),
    ]);
    let partial = HashMap::from([
        ("type".to_string(), "com.example.foo".to_string()),
        ("source".to_string(), "/devices/2".to_string()),
    ]);
    assert!(trig.matches(&matching));
    assert!(!trig.matches(&partial));
}

#[test]
fn trigger_carries_broker_name() {
    let trig = Trigger::new("tenant", "default");
    assert_eq!(trig.broker, "default");
}

// ─── 9. AutoscalerMetric windowing ───────────────────────────────────────────

#[test]
fn autoscaler_metric_average_empty_returns_zero() {
    let m = AutoscalerMetric::new();
    assert_eq!(m.average(Instant::now(), Duration::from_secs(60)), 0.0);
}

#[test]
fn autoscaler_metric_last_activity_unset_when_only_zero_samples() {
    let mut m = AutoscalerMetric::new();
    let now = Instant::now();
    m.record_at(now, 0.0);
    m.record_at(now + Duration::from_secs(1), 0.0);
    assert!(m.last_activity().is_none());
}

#[test]
fn autoscaler_metric_last_activity_updates_to_most_recent_nonzero() {
    let mut m = AutoscalerMetric::new();
    let t0 = Instant::now();
    m.record_at(t0, 5.0);
    let t1 = t0 + Duration::from_secs(10);
    m.record_at(t1, 7.0);
    assert_eq!(m.last_activity(), Some(t1));
}

#[test]
fn autoscaler_metric_average_includes_samples_at_cutoff_boundary() {
    let mut m = AutoscalerMetric::new();
    let now = Instant::now();
    // Sample at exactly `now - window` should be included (>= cutoff).
    m.record_at(now - Duration::from_secs(60), 30.0);
    m.record_at(now - Duration::from_secs(30), 60.0);
    let avg = m.average(now, Duration::from_secs(60));
    assert_eq!(avg, 45.0);
}

// ─── 10. Autoscaler.decide() — branches ──────────────────────────────────────

#[test]
fn autoscaler_with_min_scale_nonzero_never_returns_zero_for_idle() {
    let cfg = AutoscalerConfig {
        min_scale: 1,
        target_concurrency: 100.0,
        scale_to_zero_grace_period: Duration::from_secs(0),
        ..AutoscalerConfig::default()
    };
    let scaler = Autoscaler::new("t", cfg);
    let metric = AutoscalerMetric::new();
    let decision = scaler.decide(&metric, Instant::now());
    assert!(decision.desired_replicas >= 1);
}

#[test]
fn autoscaler_zero_concurrency_with_min_zero_and_no_activity_yields_zero() {
    let cfg = AutoscalerConfig {
        min_scale: 0,
        scale_to_zero_grace_period: Duration::from_secs(0),
        ..AutoscalerConfig::default()
    };
    let scaler = Autoscaler::new("t", cfg);
    let metric = AutoscalerMetric::new();
    let d = scaler.decide(&metric, Instant::now());
    assert_eq!(d.desired_replicas, 0);
    assert_eq!(d.mode, AutoscalerMode::Stable);
}

#[test]
fn autoscaler_grace_period_keeps_replicas_alive_until_expiry() {
    let cfg = AutoscalerConfig {
        min_scale: 0,
        target_concurrency: 100.0,
        scale_to_zero_grace_period: Duration::from_secs(30),
        stable_window: Duration::from_secs(60),
        panic_window: Duration::from_secs(6),
        ..AutoscalerConfig::default()
    };
    let scaler = Autoscaler::new("t", cfg);
    let mut metric = AutoscalerMetric::new();
    let now = Instant::now();
    // Last activity 10s ago — still inside the 30s grace period.
    metric.record_at(now - Duration::from_secs(10), 100.0);
    let d = scaler.decide(&metric, now);
    assert!(
        d.desired_replicas >= 1,
        "grace period not yet expired, expected >=1 replicas, got {}",
        d.desired_replicas
    );
}

#[test]
fn autoscaler_panic_decision_carries_averages_in_result() {
    let cfg = AutoscalerConfig {
        min_scale: 1,
        target_concurrency: 100.0,
        stable_window: Duration::from_secs(60),
        panic_window: Duration::from_secs(6),
        panic_threshold: 2.0,
        ..AutoscalerConfig::default()
    };
    let scaler = Autoscaler::new("t", cfg);
    let mut metric = AutoscalerMetric::new();
    let now = Instant::now();
    for offset in [55u64, 45, 35, 25] {
        metric.record_at(now - Duration::from_secs(offset), 10.0);
    }
    metric.record_at(now - Duration::from_secs(2), 600.0);
    metric.record_at(now - Duration::from_secs(1), 600.0);
    let d = scaler.decide(&metric, now);
    assert_eq!(d.mode, AutoscalerMode::Panic);
    assert!(d.panic_average > d.stable_average);
}

#[test]
fn autoscaler_stable_mode_uses_stable_average_for_replica_count() {
    let cfg = AutoscalerConfig {
        min_scale: 1,
        max_scale: 1000,
        target_concurrency: 100.0,
        scale_to_zero_grace_period: Duration::from_secs(0),
        ..AutoscalerConfig::default()
    };
    let scaler = Autoscaler::new("t", cfg);
    let mut metric = AutoscalerMetric::new();
    let now = Instant::now();
    // Flat 400 concurrency → 400/100 = 4 replicas.
    for offset in [50u64, 40, 30, 20, 10] {
        metric.record_at(now - Duration::from_secs(offset), 400.0);
    }
    let d = scaler.decide(&metric, now);
    assert_eq!(d.mode, AutoscalerMode::Stable);
    assert_eq!(d.desired_replicas, 4);
}

#[test]
fn autoscaler_default_config_has_kpa_canonical_values() {
    let cfg = AutoscalerConfig::default();
    assert_eq!(cfg.target_concurrency, 100.0);
    assert_eq!(cfg.min_scale, 0);
    assert_eq!(cfg.max_scale, 1000);
    assert_eq!(cfg.stable_window, Duration::from_secs(60));
    assert_eq!(cfg.panic_window, Duration::from_secs(6));
    assert_eq!(cfg.panic_threshold, 2.0);
    assert_eq!(cfg.scale_to_zero_grace_period, Duration::from_secs(30));
}

// ─── 11. Container env round-trip ────────────────────────────────────────────

#[test]
fn container_env_preserves_optional_value() {
    let c = Container {
        name: "app".to_string(),
        image: "img".to_string(),
        env: vec![
            EnvVar { name: "DATABASE_URL".to_string(), value: Some("postgres://x".to_string()) },
            EnvVar { name: "DEBUG".to_string(), value: None },
        ],
    };
    assert_eq!(c.env.len(), 2);
    assert_eq!(c.env[0].value.as_deref(), Some("postgres://x"));
    assert!(c.env[1].value.is_none());
}
