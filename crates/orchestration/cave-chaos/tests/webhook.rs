// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: Chaos Mesh validating admission webhook (api/v1alpha1/*_webhook.go).
//! Rejects experiments/schedules at admission time — before they are stored —
//! on safety-guard, blast-radius, required-param, duration and cron violations.

use cave_chaos::models::{
    BlastRadius, ChaosExperiment, ChaosTarget, ExperimentParams, ExperimentStatus, ExperimentType,
    SafetyGuard,
};
use cave_chaos::webhook::{validate_experiment_admission, validate_schedule_admission};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn base(t: ExperimentType, ns: &str, params: ExperimentParams, duration: u32) -> ChaosExperiment {
    ChaosExperiment {
        id: Uuid::new_v4(),
        name: "wh".to_string(),
        experiment_type: t,
        target: ChaosTarget { namespace: ns.to_string(), selector: HashMap::new(), pod_count: None },
        parameters: params,
        status: ExperimentStatus::Draft,
        created_at: Utc::now(),
        started_at: None,
        ended_at: None,
        duration_secs: duration,
        blast_radius: BlastRadius::default(),
        safety_guard: SafetyGuard::default(),
        result: None,
        annotations: HashMap::new(),
    }
}

fn empty() -> ExperimentParams {
    ExperimentParams { latency_ms: None, packet_loss_percent: None, cpu_load_percent: None, memory_mb: None }
}

#[test]
fn test_admission_allows_valid_experiment() {
    let exp = base(
        ExperimentType::NetworkLatency,
        "staging",
        ExperimentParams { latency_ms: Some(100), ..empty() },
        60,
    );
    let r = validate_experiment_admission(&exp);
    assert!(r.allowed, "expected allowed, got: {}", r.message);
}

#[test]
fn test_admission_denies_empty_name() {
    let mut exp = base(ExperimentType::PodKill, "staging", empty(), 30);
    exp.name = String::new();
    let r = validate_experiment_admission(&exp);
    assert!(!r.allowed);
    assert!(r.message.to_lowercase().contains("name"), "got: {}", r.message);
}

#[test]
fn test_admission_denies_protected_namespace() {
    let exp = base(ExperimentType::PodKill, "kube-system", empty(), 30);
    let r = validate_experiment_admission(&exp);
    assert!(!r.allowed);
    assert!(r.message.contains("protected"), "got: {}", r.message);
}

#[test]
fn test_admission_denies_missing_required_param() {
    let exp = base(ExperimentType::NetworkLatency, "staging", empty(), 60);
    let r = validate_experiment_admission(&exp);
    assert!(!r.allowed);
    assert!(r.message.contains("latency_ms"), "got: {}", r.message);
}

#[test]
fn test_admission_denies_zero_duration() {
    let exp = base(
        ExperimentType::NetworkLatency,
        "staging",
        ExperimentParams { latency_ms: Some(100), ..empty() },
        0,
    );
    let r = validate_experiment_admission(&exp);
    assert!(!r.allowed);
    assert!(r.message.contains("duration"), "got: {}", r.message);
}

#[test]
fn test_admission_denies_packet_loss_over_100() {
    let exp = base(
        ExperimentType::NetworkPacketLoss,
        "staging",
        ExperimentParams { packet_loss_percent: Some(150.0), ..empty() },
        60,
    );
    let r = validate_experiment_admission(&exp);
    assert!(!r.allowed);
}

#[test]
fn test_admission_denies_blast_radius_fraction_over_one() {
    let mut exp = base(
        ExperimentType::NetworkLatency,
        "staging",
        ExperimentParams { latency_ms: Some(100), ..empty() },
        60,
    );
    exp.blast_radius.max_pod_fraction = 1.5;
    let r = validate_experiment_admission(&exp);
    assert!(!r.allowed);
    assert!(r.message.contains("blast"), "got: {}", r.message);
}

#[test]
fn test_admission_denies_blast_radius_fraction_zero_or_negative() {
    let mut exp = base(
        ExperimentType::NetworkLatency,
        "staging",
        ExperimentParams { latency_ms: Some(100), ..empty() },
        60,
    );
    exp.blast_radius.max_pod_fraction = 0.0;
    let r = validate_experiment_admission(&exp);
    assert!(!r.allowed, "fraction must be > 0");
}

#[test]
fn test_admission_denies_max_pods_zero() {
    let mut exp = base(
        ExperimentType::NetworkLatency,
        "staging",
        ExperimentParams { latency_ms: Some(100), ..empty() },
        60,
    );
    exp.blast_radius.max_pods = Some(0);
    let r = validate_experiment_admission(&exp);
    assert!(!r.allowed);
}

#[test]
fn test_admission_allows_disabled_safety_guard_protected_ns() {
    // With the guard disabled, a protected namespace is no longer auto-denied.
    let mut exp = base(ExperimentType::PodKill, "kube-system", empty(), 30);
    exp.safety_guard.enabled = false;
    let r = validate_experiment_admission(&exp);
    assert!(r.allowed, "got: {}", r.message);
}

// ── schedule (cron) admission ───────────────────────────────────────────────

#[test]
fn test_schedule_admission_allows_valid_cron() {
    let r = validate_schedule_admission("0 2 * * 1");
    assert!(r.allowed, "got: {}", r.message);
}

#[test]
fn test_schedule_admission_denies_four_field_cron() {
    let r = validate_schedule_admission("* * * *");
    assert!(!r.allowed);
}

#[test]
fn test_schedule_admission_denies_minute_out_of_range() {
    let r = validate_schedule_admission("60 * * * *");
    assert!(!r.allowed);
}

#[test]
fn test_schedule_admission_denies_empty_cron() {
    let r = validate_schedule_admission("");
    assert!(!r.allowed);
}
