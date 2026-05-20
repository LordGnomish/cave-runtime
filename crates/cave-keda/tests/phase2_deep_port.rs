// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-keda Phase 2 deep-port — 7 new scalers (AWS SQS, Azure
//! ServiceBus, Azure EventHub, GCP PubSub, NATS JetStream, Etcd,
//! Datadog), ScalingModifiers evaluator, hibernation strategy, deep
//! pause semantics.

use cave_keda::aws_sqs_scaler::AwsSqsScaler;
use cave_keda::azure_eventhub_scaler::AzureEventHubScaler;
use cave_keda::azure_servicebus_scaler::{AzureServiceBusEntity, AzureServiceBusScaler};
use cave_keda::datadog_scaler::DatadogScaler;
use cave_keda::etcd_scaler::EtcdScaler;
use cave_keda::gcp_pubsub_scaler::GcpPubSubScaler;
use cave_keda::hibernation::{Hibernation, HibernationDecision, HibernationSchedule};
use cave_keda::nats_jetstream_scaler::NatsJetStreamScaler;
use cave_keda::scaler::ScalerTrait;
use cave_keda::scaling_modifiers::{ScalingModifiersEvaluator, Trigger};
use chrono::{TimeZone, Utc};

// ─── AWS SQS scaler ─────────────────────────────────────────────────────────

#[test]
fn aws_sqs_active_above_activation_threshold() {
    let mut s = AwsSqsScaler::new("t");
    s.activation_queue_length = 5;
    s.queue_length_target = 50;
    s.observe(2);
    assert!(!s.is_active());
    s.observe(20);
    assert!(s.is_active());
    assert_eq!(s.metric_value(), Some(20.0));
}

#[test]
fn aws_sqs_clamps_negative_observation() {
    let mut s = AwsSqsScaler::new("t");
    s.observe(-100);
    assert_eq!(s.current_queue_length, 0);
}

// ─── Azure Service Bus scaler ───────────────────────────────────────────────

#[test]
fn azure_servicebus_queue_entity_default() {
    let s = AzureServiceBusScaler::new("t");
    assert_eq!(s.entity, AzureServiceBusEntity::Queue);
    assert_eq!(s.target_message_count, 5);
}

#[test]
fn azure_servicebus_topic_subscription_uses_separate_metric() {
    let mut s = AzureServiceBusScaler::new("t");
    s.entity = AzureServiceBusEntity::Subscription;
    s.observe(10);
    assert!(s.is_active());
}

// ─── Azure Event Hub scaler ─────────────────────────────────────────────────

#[test]
fn azure_eventhub_per_partition_unprocessed_event_count() {
    let mut s = AzureEventHubScaler::new("t");
    s.record_unprocessed(0, 100);
    s.record_unprocessed(1, 200);
    assert_eq!(s.total_unprocessed(), 300);
}

#[test]
fn azure_eventhub_inactive_when_no_partitions_observed() {
    let s = AzureEventHubScaler::new("t");
    assert!(!s.is_active());
}

// ─── GCP Pub/Sub scaler ─────────────────────────────────────────────────────

#[test]
fn gcp_pubsub_active_above_threshold() {
    let mut s = GcpPubSubScaler::new("t");
    s.subscription_size_target = 100;
    s.activation_threshold = 10;
    s.observe(5);
    assert!(!s.is_active());
    s.observe(500);
    assert!(s.is_active());
    assert_eq!(s.metric_value(), Some(500.0));
}

// ─── NATS JetStream scaler ──────────────────────────────────────────────────

#[test]
fn nats_jetstream_consumer_lag_metric() {
    let mut s = NatsJetStreamScaler::new("t");
    s.consumer_lag_target = 10;
    s.observe(50);
    assert_eq!(s.metric_value(), Some(50.0));
    assert!(s.is_active());
}

#[test]
fn nats_jetstream_pending_messages_above_activation() {
    let mut s = NatsJetStreamScaler::new("t");
    s.activation_lag_threshold = 5;
    s.observe(3);
    assert!(!s.is_active());
}

// ─── Etcd scaler ────────────────────────────────────────────────────────────

#[test]
fn etcd_key_value_threshold_drives_metric() {
    let mut s = EtcdScaler::new("t");
    s.target_value = 100;
    s.activation_threshold = 5;
    s.observe(150);
    assert_eq!(s.metric_value(), Some(150.0));
    assert!(s.is_active());
}

// ─── Datadog scaler ─────────────────────────────────────────────────────────

#[test]
fn datadog_active_when_query_returns_above_activation() {
    let mut s = DatadogScaler::new("t");
    s.activation_query_value = 2.0;
    s.query_value = 5.0;
    assert!(s.is_active());
}

#[test]
fn datadog_metric_value_passes_through() {
    let mut s = DatadogScaler::new("t");
    s.query_value = 42.5;
    assert_eq!(s.metric_value(), Some(42.5));
}

// ─── ScalingModifiers (formula-based replica recommendation) ────────────────

#[test]
fn scaling_modifiers_picks_max_when_formula_max() {
    let mut ev = ScalingModifiersEvaluator::new();
    ev.target = 1.0;
    ev.formula = "max(a,b)".into();
    ev.add_trigger(Trigger::new("a", 5.0, true));
    ev.add_trigger(Trigger::new("b", 12.0, true));
    let replicas = ev.evaluate();
    assert_eq!(replicas, 12);
}

#[test]
fn scaling_modifiers_picks_sum_when_formula_sum() {
    let mut ev = ScalingModifiersEvaluator::new();
    ev.target = 1.0;
    ev.formula = "sum(a,b,c)".into();
    ev.add_trigger(Trigger::new("a", 3.0, true));
    ev.add_trigger(Trigger::new("b", 4.0, true));
    ev.add_trigger(Trigger::new("c", 5.0, true));
    let replicas = ev.evaluate();
    assert_eq!(replicas, 12);
}

#[test]
fn scaling_modifiers_picks_min_when_formula_min() {
    let mut ev = ScalingModifiersEvaluator::new();
    ev.target = 1.0;
    ev.formula = "min(a,b)".into();
    ev.add_trigger(Trigger::new("a", 5.0, true));
    ev.add_trigger(Trigger::new("b", 12.0, true));
    assert_eq!(ev.evaluate(), 5);
}

#[test]
fn scaling_modifiers_activation_target_gates_below_zero() {
    let mut ev = ScalingModifiersEvaluator::new();
    ev.target = 1.0;
    ev.activation_target = Some(10);
    ev.formula = "max(a)".into();
    ev.add_trigger(Trigger::new("a", 5.0, true));
    // metric 5 < activation_target 10 → scaler reports inactive
    assert!(!ev.is_active());
}

#[test]
fn scaling_modifiers_unknown_formula_falls_back_to_sum() {
    let mut ev = ScalingModifiersEvaluator::new();
    ev.target = 1.0;
    ev.formula = "unsupported(a,b)".into();
    ev.add_trigger(Trigger::new("a", 5.0, true));
    ev.add_trigger(Trigger::new("b", 7.0, true));
    assert_eq!(ev.evaluate(), 12);
}

// ─── Hibernation schedule ───────────────────────────────────────────────────

#[test]
fn hibernation_active_inside_window_returns_zero_replicas() {
    let h = Hibernation {
        schedules: vec![HibernationSchedule {
            cron_start: "0 18 * * 1-5".into(), // 18:00 Mon-Fri
            cron_end: "0 8 * * 1-5".into(),    // 08:00 Mon-Fri
            replicas_during_hibernation: 0,
            timezone: "UTC".into(),
        }],
    };
    // Saturday at any time — not Mon-Fri, so not hibernating per the
    // weekday-locked schedule.
    let sat_noon = Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap();
    assert_eq!(h.decide_at(sat_noon), HibernationDecision::Awake);

    // Wed at 22:00 UTC — inside the 18→08 window.
    let wed_night = Utc.with_ymd_and_hms(2026, 5, 20, 22, 0, 0).unwrap();
    match h.decide_at(wed_night) {
        HibernationDecision::Hibernating { replicas } => assert_eq!(replicas, 0),
        other => panic!("expected hibernating, got {other:?}"),
    }
}

#[test]
fn hibernation_no_schedules_is_awake() {
    let h = Hibernation { schedules: vec![] };
    let now = Utc::now();
    assert_eq!(h.decide_at(now), HibernationDecision::Awake);
}

#[test]
fn hibernation_replicas_during_hibernation_can_be_nonzero() {
    let h = Hibernation {
        schedules: vec![HibernationSchedule {
            cron_start: "0 0 * * *".into(),
            cron_end: "59 23 * * *".into(),
            replicas_during_hibernation: 2,
            timezone: "UTC".into(),
        }],
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 20, 12, 0, 0).unwrap();
    match h.decide_at(now) {
        HibernationDecision::Hibernating { replicas } => assert_eq!(replicas, 2),
        other => panic!("expected hibernating, got {other:?}"),
    }
}
