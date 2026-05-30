// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-knative: Knative Serverless reimplementation.
//!
//! upstream: knative/serving v1.18.x, knative/eventing v1.18.x
//!
//! Modules:
//!   meta              — shared CRD primitives (ObjectMeta, RevisionTemplateSpec, TrafficTarget, validators)
//!   ksvc              — top-level Service (wraps Configuration + Route)
//!   configuration     — desired-state spec; spawns Revisions
//!   revision          — immutable code+config snapshot, scaling target
//!   route             — traffic split between revisions
//!   eventing          — Source/Sink/Channel/Subscription/Trigger
//!   autoscaler        — Knative Pod Autoscaler (KPA) with stable/panic modes
//!   sources_ping      — PingSource cron event emitter (Phase 2)
//!   sources_apiserver — ApiServerSource K8s watch → CloudEvent (Phase 2)
//!   sources_container — ContainerSource deployment projection (Phase 2)
//!   eventing_transports — Kafka / RabbitMQ / Pulsar / NATS / GitHub adapters (Phase 2)
//!   broker_controller — Broker reconciler state machine (Phase 2)
//!   webhook           — Admission validation + defaulting (Phase 2)
//!   cert_bridge       — cert-manager Certificate CR projection (Phase 2)

#![allow(non_snake_case)]

pub mod autoscaler;
pub mod broker_controller;
pub mod cert_bridge;
pub mod configuration;
pub mod eventing;
pub mod eventing_transports;
pub mod hpa_bridge;
pub mod in_memory_channel;
pub mod ksvc;
pub mod meta;
pub mod queue_proxy;
pub mod revision;
pub mod route;
pub mod sources_apiserver;
pub mod sources_container;
pub mod sources_ping;
pub mod webhook;

pub use autoscaler::{
    Autoscaler, AutoscalerConfig, AutoscalerMetric, AutoscalerMode, ScaleDecision,
};
pub use broker_controller::{
    Broker, BrokerSpec, BrokerStatus, ConditionState, DeliverySpec, ReconcileAction,
};
pub use cert_bridge::{
    CertManagerCertificate, CertManagerStatus, IssuerRef, KnativeCertificate,
    KnativeCertificateSpec, KnativeCertificateStatus, project_status_back, to_cert_manager,
};
pub use configuration::{Configuration, ConfigurationSpec, ConfigurationStatus};
pub use eventing::{Channel, EventingSink, EventingSource, Subscription, Trigger, TriggerFilter};
pub use eventing_transports::{
    DeliveryReceipt, GitHubSource, KafkaTransport, NatsTransport, PulsarTransport,
    RabbitMqTransport, Transport, hmac_sha256_hex, sha256,
};
pub use ksvc::{Ksvc, ServiceSpec, ServiceStatus};
pub use meta::{
    ANNOTATION_AUTOSCALER_CLASS, ANNOTATION_CREATOR, ANNOTATION_LAST_MODIFIER,
    ANNOTATION_MAX_SCALE, ANNOTATION_METRIC, ANNOTATION_MIN_SCALE, ANNOTATION_TARGET, Container,
    EnvVar, ObjectMeta, PodSpec, RevisionTemplateSpec, TrafficTarget, validate_template,
    validate_traffic,
};
pub use revision::{Revision, RevisionSpec, RevisionStatus};
pub use route::{Route, RouteSpec, RouteStatus};
pub use sources_apiserver::{
    ApiServerSource, ApiServerSourceSpec, ApiServerSourceStatus, EventMode, ResourceEvent,
    ResourceEventType,
};
pub use sources_container::{ContainerSource, ContainerSourceSpec, ContainerSourceStatus};
pub use sources_ping::{CloudEvent, PingSource, PingSourceSpec, PingSourceStatus};
pub use webhook::{
    AdmissionObject, AdmissionOp, PatchOp, WebhookRequest, WebhookResponse, admit,
    validate_ksvc_template,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    fn container(name: &str, image: &str) -> Container {
        Container {
            name: name.to_string(),
            image: image.to_string(),
            env: vec![],
        }
    }

    fn template(image: &str) -> RevisionTemplateSpec {
        RevisionTemplateSpec {
            metadata: ObjectMeta::default(),
            spec: PodSpec {
                containers: vec![container("app", image)],
            },
        }
    }

    // ─── Ksvc ────────────────────────────────────────────────

    #[test]
    fn ksvc_new_records_creator_annotation() {
        let svc = Ksvc::new("tenant-1");
        assert_eq!(svc.metadata.creator(), Some(&"tenant-1".to_string()));
    }

    #[test]
    fn ksvc_scale_to_zero_zeroes_traffic_percent() {
        let mut svc = Ksvc::new("t");
        svc.status.traffic = vec![TrafficTarget {
            revision_name: Some("rev-0".to_string()),
            percent: Some(100),
            ..Default::default()
        }];
        svc.scale_to_zero();
        assert_eq!(svc.status.traffic[0].percent, Some(0));
    }

    #[test]
    fn ksvc_scale_to_zero_preserves_creator() {
        let mut svc = Ksvc::new("alpha");
        svc.status.traffic = vec![TrafficTarget {
            revision_name: Some("r".to_string()),
            percent: Some(100),
            ..Default::default()
        }];
        svc.scale_to_zero();
        assert_eq!(svc.metadata.creator(), Some(&"alpha".to_string()));
    }

    #[test]
    fn ksvc_name_returns_metadata_name() {
        let mut svc = Ksvc::new("t");
        svc.metadata.name = "my-service".to_string();
        assert_eq!(svc.name(), "my-service");
    }

    #[test]
    fn ksvc_validate_rejects_template_without_containers() {
        let svc = Ksvc::new("t");
        assert!(svc.validate().is_err());
    }

    #[test]
    fn ksvc_validate_accepts_minimal_template() {
        let mut svc = Ksvc::new("t");
        svc.spec.template = template("nginx:1");
        assert!(svc.validate().is_ok());
    }

    #[test]
    fn ksvc_validate_rejects_traffic_not_summing_to_100() {
        let mut svc = Ksvc::new("t");
        svc.spec.template = template("img");
        svc.spec.traffic = vec![
            TrafficTarget {
                revision_name: Some("a".to_string()),
                percent: Some(40),
                ..Default::default()
            },
            TrafficTarget {
                revision_name: Some("b".to_string()),
                percent: Some(40),
                ..Default::default()
            },
        ];
        let err = svc.validate().unwrap_err();
        assert!(err.contains("100"));
    }

    // ─── Configuration ────────────────────────────────────────

    #[test]
    fn configuration_records_created_revision() {
        let mut cfg = Configuration::new("t");
        cfg.record_created_revision("rev-0001");
        assert_eq!(
            cfg.status.latestCreatedRevisionName,
            Some("rev-0001".to_string())
        );
    }

    #[test]
    fn configuration_records_ready_revision_separately_from_created() {
        let mut cfg = Configuration::new("t");
        cfg.record_created_revision("rev-1");
        cfg.record_ready_revision("rev-0"); // older one becomes ready
        assert_eq!(
            cfg.status.latestCreatedRevisionName,
            Some("rev-1".to_string())
        );
        assert_eq!(
            cfg.status.latestReadyRevisionName,
            Some("rev-0".to_string())
        );
    }

    #[test]
    fn configuration_validate_requires_template_image() {
        let mut cfg = Configuration::new("t");
        cfg.spec.template = RevisionTemplateSpec {
            metadata: ObjectMeta::default(),
            spec: PodSpec {
                containers: vec![container("c", "")],
            },
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn configuration_scale_to_zero_records_observed_generation() {
        let mut cfg = Configuration::new("t");
        cfg.metadata.generation = 7;
        cfg.scale_to_zero();
        assert_eq!(cfg.status.observed_generation, 7);
    }

    // ─── Revision ─────────────────────────────────────────────

    #[test]
    fn revision_scale_to_zero_sets_desired_replicas_zero() {
        let mut rev = Revision::new("t");
        rev.set_desired_replicas(5);
        rev.scale_to_zero();
        assert_eq!(rev.status.desiredReplicas, Some(0));
    }

    #[test]
    fn revision_is_active_tracks_desired_replicas() {
        let mut rev = Revision::new("t");
        assert!(!rev.is_active());
        rev.set_desired_replicas(2);
        assert!(rev.is_active());
        rev.scale_to_zero();
        assert!(!rev.is_active());
    }

    #[test]
    fn revision_set_desired_replicas_clamps_negative_to_zero() {
        let mut rev = Revision::new("t");
        rev.set_desired_replicas(-3);
        assert_eq!(rev.status.desiredReplicas, Some(0));
    }

    #[test]
    fn revision_validate_rejects_negative_concurrency() {
        let mut rev = Revision::new("t");
        rev.spec.containerConcurrency = Some(-1);
        rev.spec.template = template("img");
        assert!(rev.validate().is_err());
    }

    #[test]
    fn revision_validate_rejects_zero_timeout() {
        let mut rev = Revision::new("t");
        rev.spec.timeoutSeconds = Some(0);
        rev.spec.template = template("img");
        assert!(rev.validate().is_err());
    }

    #[test]
    fn revision_name_returns_metadata_name() {
        let mut rev = Revision::new("t");
        rev.metadata.name = "rev-123".to_string();
        assert_eq!(rev.name(), "rev-123");
    }

    // ─── Route ────────────────────────────────────────────────

    #[test]
    fn route_promote_sets_100_percent_target() {
        let mut r = Route::new("t");
        r.promote("rev-blue");
        assert_eq!(r.spec.traffic.len(), 1);
        assert_eq!(r.spec.traffic[0].percent, Some(100));
        assert_eq!(
            r.spec.traffic[0].revision_name,
            Some("rev-blue".to_string())
        );
    }

    #[test]
    fn route_resolve_revision_returns_correct_target_for_percentile() {
        let mut r = Route::new("t");
        r.status.traffic = vec![
            TrafficTarget {
                revision_name: Some("rev-a".to_string()),
                percent: Some(80),
                ..Default::default()
            },
            TrafficTarget {
                revision_name: Some("rev-b".to_string()),
                percent: Some(20),
                ..Default::default()
            },
        ];
        assert_eq!(r.resolve_revision(50), Some("rev-a"));
        assert_eq!(r.resolve_revision(85), Some("rev-b"));
    }

    #[test]
    fn route_resolve_revision_clamps_high_percentile() {
        let mut r = Route::new("t");
        r.status.traffic = vec![TrafficTarget {
            revision_name: Some("rev-x".to_string()),
            percent: Some(100),
            ..Default::default()
        }];
        assert_eq!(r.resolve_revision(999), Some("rev-x"));
    }

    #[test]
    fn route_validate_rejects_traffic_not_summing_to_100() {
        let mut r = Route::new("t");
        r.spec.traffic = vec![TrafficTarget {
            revision_name: Some("a".to_string()),
            percent: Some(50),
            ..Default::default()
        }];
        assert!(r.validate().is_err());
    }

    #[test]
    fn route_tag_creates_zero_percent_subroute() {
        let mut r = Route::new("t");
        r.promote("rev-stable");
        r.tag("rev-canary", "canary");
        assert_eq!(r.spec.traffic.len(), 2);
        let canary = r
            .spec
            .traffic
            .iter()
            .find(|t| t.tag.as_deref() == Some("canary"))
            .unwrap();
        assert_eq!(canary.percent, Some(0));
    }

    // ─── Eventing ─────────────────────────────────────────────

    #[test]
    fn eventing_source_resolve_sink_populates_uri() {
        let mut s = EventingSource::new("t");
        s.spec.sink = Some("https://my-sink.svc/in".to_string());
        let resolved = s.resolve_sink().map(str::to_string);
        assert_eq!(resolved, Some("https://my-sink.svc/in".to_string()));
        assert_eq!(s.status.sinkURI, Some("https://my-sink.svc/in".to_string()));
    }

    #[test]
    fn eventing_source_scale_to_zero_clears_sink_uri() {
        let mut s = EventingSource::new("t");
        s.spec.sink = Some("https://x".to_string());
        s.resolve_sink();
        s.scale_to_zero();
        assert!(s.status.sinkURI.is_none());
    }

    #[test]
    fn channel_subscribe_and_fanout() {
        let mut c = Channel::new("t");
        c.subscribe(Subscription {
            uid: "u1".to_string(),
            subscriber_uri: "http://a".to_string(),
            reply_uri: None,
        });
        c.subscribe(Subscription {
            uid: "u2".to_string(),
            subscriber_uri: "http://b".to_string(),
            reply_uri: None,
        });
        let mut targets = c.fanout();
        targets.sort();
        assert_eq!(
            targets,
            vec!["http://a".to_string(), "http://b".to_string()]
        );
    }

    #[test]
    fn channel_unsubscribe_removes_target() {
        let mut c = Channel::new("t");
        c.subscribe(Subscription {
            uid: "u1".to_string(),
            subscriber_uri: "http://a".to_string(),
            reply_uri: None,
        });
        c.unsubscribe("u1");
        assert!(c.fanout().is_empty());
    }

    #[test]
    fn trigger_filter_empty_matches_anything() {
        let trig = Trigger::new("t", "default");
        let attrs = HashMap::from([("type".to_string(), "x".to_string())]);
        assert!(trig.matches(&attrs));
    }

    #[test]
    fn trigger_filter_exact_attributes_match_required() {
        let mut trig = Trigger::new("t", "default");
        trig.filter
            .attributes
            .insert("type".to_string(), "com.example.foo".to_string());
        let good = HashMap::from([("type".to_string(), "com.example.foo".to_string())]);
        let bad = HashMap::from([("type".to_string(), "com.example.bar".to_string())]);
        assert!(trig.matches(&good));
        assert!(!trig.matches(&bad));
    }

    #[test]
    fn trigger_filter_missing_attribute_is_no_match() {
        let mut trig = Trigger::new("t", "default");
        trig.filter
            .attributes
            .insert("source".to_string(), "/devices/1".to_string());
        let attrs = HashMap::new();
        assert!(!trig.matches(&attrs));
    }

    // ─── Validation helpers ───────────────────────────────────

    #[test]
    fn validate_traffic_rejects_empty() {
        assert!(validate_traffic(&[]).is_err());
    }

    #[test]
    fn validate_traffic_accepts_latest_revision_target() {
        let t = vec![TrafficTarget {
            latest_revision: Some(true),
            percent: Some(100),
            ..Default::default()
        }];
        assert!(validate_traffic(&t).is_ok());
    }

    #[test]
    fn validate_template_requires_image() {
        let tpl = RevisionTemplateSpec {
            metadata: ObjectMeta::default(),
            spec: PodSpec {
                containers: vec![Container::default()],
            },
        };
        assert!(validate_template(&tpl).is_err());
    }

    // ─── Autoscaler (KPA) ─────────────────────────────────────

    #[test]
    fn autoscaler_scales_to_zero_when_idle_past_grace_period() {
        let cfg = AutoscalerConfig {
            min_scale: 0,
            scale_to_zero_grace_period: Duration::from_secs(10),
            ..AutoscalerConfig::default()
        };
        let scaler = Autoscaler::new("t", cfg);
        let metric = AutoscalerMetric::new();
        let decision = scaler.decide(&metric, Instant::now());
        assert_eq!(decision.desired_replicas, 0);
    }

    #[test]
    fn autoscaler_respects_min_scale_floor() {
        let cfg = AutoscalerConfig {
            min_scale: 2,
            target_concurrency: 100.0,
            ..AutoscalerConfig::default()
        };
        let scaler = Autoscaler::new("t", cfg);
        let mut metric = AutoscalerMetric::new();
        let now = Instant::now();
        // No real activity, but min_scale floor is 2
        metric.record_at(now, 0.0);
        let d = scaler.decide(&metric, now);
        assert!(d.desired_replicas >= 2);
    }

    #[test]
    fn autoscaler_respects_max_scale_ceiling() {
        let cfg = AutoscalerConfig {
            min_scale: 1,
            max_scale: 5,
            target_concurrency: 10.0,
            scale_to_zero_grace_period: Duration::from_secs(0),
            ..AutoscalerConfig::default()
        };
        let scaler = Autoscaler::new("t", cfg);
        let mut metric = AutoscalerMetric::new();
        let now = Instant::now();
        metric.record_at(now, 1000.0); // 1000/10 = 100 raw, capped
        let d = scaler.decide(&metric, now);
        assert_eq!(d.desired_replicas, 5);
    }

    #[test]
    fn autoscaler_panic_mode_engages_above_threshold() {
        let cfg = AutoscalerConfig {
            min_scale: 1,
            target_concurrency: 50.0,
            stable_window: Duration::from_secs(60),
            panic_window: Duration::from_secs(6),
            panic_threshold: 2.0,
            ..AutoscalerConfig::default()
        };
        let scaler = Autoscaler::new("t", cfg);
        let mut metric = AutoscalerMetric::new();
        let now = Instant::now();
        // Many low samples in the stable-but-not-panic horizon
        for offset in [55u64, 50, 45, 40, 35, 30, 25, 20, 15, 10] {
            metric.record_at(now - Duration::from_secs(offset), 10.0);
        }
        // Recent burst (within panic window) — much higher than stable
        metric.record_at(now - Duration::from_secs(2), 500.0);
        metric.record_at(now - Duration::from_secs(1), 500.0);
        let d = scaler.decide(&metric, now);
        assert_eq!(d.mode, AutoscalerMode::Panic);
    }

    #[test]
    fn autoscaler_stable_mode_when_load_smooth() {
        let cfg = AutoscalerConfig {
            min_scale: 1,
            target_concurrency: 50.0,
            scale_to_zero_grace_period: Duration::from_secs(0),
            ..AutoscalerConfig::default()
        };
        let scaler = Autoscaler::new("t", cfg);
        let mut metric = AutoscalerMetric::new();
        let now = Instant::now();
        for offset in [10u64, 20, 30, 40] {
            metric.record_at(now - Duration::from_secs(offset), 50.0);
        }
        let d = scaler.decide(&metric, now);
        assert_eq!(d.mode, AutoscalerMode::Stable);
    }

    #[test]
    fn autoscaler_concurrency_to_replicas_ceil() {
        let cfg = AutoscalerConfig {
            min_scale: 1,
            max_scale: 100,
            target_concurrency: 100.0,
            scale_to_zero_grace_period: Duration::from_secs(0),
            ..AutoscalerConfig::default()
        };
        let scaler = Autoscaler::new("t", cfg);
        let mut metric = AutoscalerMetric::new();
        let now = Instant::now();
        // 250 concurrent → ceil(250/100) = 3
        metric.record_at(now, 250.0);
        let d = scaler.decide(&metric, now);
        assert_eq!(d.desired_replicas, 3);
    }

    #[test]
    fn autoscaler_metric_average_excludes_old_samples() {
        let mut m = AutoscalerMetric::new();
        let now = Instant::now();
        m.record_at(now - Duration::from_secs(120), 100.0); // outside 60s window
        m.record_at(now - Duration::from_secs(10), 50.0);
        let avg = m.average(now, Duration::from_secs(60));
        assert_eq!(avg, 50.0);
    }

    #[test]
    fn autoscaler_metric_last_activity_only_for_nonzero() {
        let mut m = AutoscalerMetric::new();
        let t1 = Instant::now() - Duration::from_secs(30);
        m.record_at(t1, 5.0);
        m.record_at(t1 + Duration::from_secs(10), 0.0);
        // last_activity is the last *non-zero* sample
        assert_eq!(m.last_activity(), Some(t1));
    }
}
