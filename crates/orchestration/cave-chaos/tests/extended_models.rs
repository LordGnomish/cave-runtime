// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Failing tests for extended model types:
//! BlastRadius, SafetyGuard, ExperimentResult, ExperimentEvent,
//! ExperimentSchedule, extended ExperimentType enum.
//!
//! These types are NEW (not in origin/main models.rs).

use cave_chaos::models::{
    BlastRadius, ChaosExperiment, ChaosTarget, ExperimentEvent, ExperimentParams, ExperimentResult,
    ExperimentSchedule, ExperimentStatus, ExperimentType, SafetyGuard,
};
use std::collections::HashMap;
use uuid::Uuid;
use chrono::Utc;

fn make_full_experiment(exp_type: ExperimentType) -> ChaosExperiment {
    ChaosExperiment {
        id: Uuid::new_v4(),
        name: "extended-test".into(),
        experiment_type: exp_type,
        target: ChaosTarget {
            namespace: "staging".into(),
            selector: HashMap::new(),
            pod_count: Some(2),
        },
        parameters: ExperimentParams {
            latency_ms: Some(100),
            packet_loss_percent: None,
            cpu_load_percent: None,
            memory_mb: None,
        },
        status: ExperimentStatus::Draft,
        created_at: Utc::now(),
        started_at: None,
        ended_at: None,
        duration_secs: 60,
        blast_radius: BlastRadius::default(),
        safety_guard: SafetyGuard::default(),
        result: None,
        annotations: HashMap::new(),
    }
}

// ─── BlastRadius ──────────────────────────────────────────────────────────────

#[test]
fn blast_radius_default_max_pod_fraction() {
    let br = BlastRadius::default();
    // Default: 50% max affected
    assert!(br.max_pod_fraction > 0.0 && br.max_pod_fraction <= 1.0);
}

#[test]
fn blast_radius_serde_roundtrip() {
    let br = BlastRadius {
        max_pod_fraction: 0.3,
        max_pods: Some(5),
        namespaces: vec!["staging".into(), "dev".into()],
    };
    let json = serde_json::to_string(&br).unwrap();
    let back: BlastRadius = serde_json::from_str(&json).unwrap();
    assert_eq!(back.max_pod_fraction, br.max_pod_fraction);
    assert_eq!(back.max_pods, br.max_pods);
    assert_eq!(back.namespaces.len(), 2);
}

// ─── SafetyGuard ─────────────────────────────────────────────────────────────

#[test]
fn safety_guard_default_enabled_with_protected_namespaces() {
    let sg = SafetyGuard::default();
    assert!(sg.enabled);
    // kube-system must be protected by default
    assert!(sg.protected_namespaces.contains(&"kube-system".to_string()));
}

#[test]
fn safety_guard_serde_roundtrip() {
    let sg = SafetyGuard {
        enabled: true,
        protected_namespaces: vec!["kube-system".into(), "monitoring".into()],
        min_healthy_pod_percentage: 0.6,
    };
    let json = serde_json::to_string(&sg).unwrap();
    let back: SafetyGuard = serde_json::from_str(&json).unwrap();
    assert!(back.enabled);
    assert_eq!(back.protected_namespaces.len(), 2);
    assert!((back.min_healthy_pod_percentage - 0.6).abs() < 1e-6);
}

// ─── ExperimentEvent ─────────────────────────────────────────────────────────

#[test]
fn experiment_event_serde_roundtrip() {
    let ev = ExperimentEvent {
        timestamp: Utc::now(),
        event_type: "injection_started".into(),
        message: "Fault injection started".into(),
        target: Some("staging-pod-abc123".into()),
    };
    let json = serde_json::to_string(&ev).unwrap();
    let back: ExperimentEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back.event_type, "injection_started");
    assert_eq!(back.target, Some("staging-pod-abc123".into()));
}

#[test]
fn experiment_event_no_target_ok() {
    let ev = ExperimentEvent {
        timestamp: Utc::now(),
        event_type: "rollback".into(),
        message: "Rolled back by operator".into(),
        target: None,
    };
    let json = serde_json::to_string(&ev).unwrap();
    let back: ExperimentEvent = serde_json::from_str(&json).unwrap();
    assert!(back.target.is_none());
}

// ─── ExperimentResult ────────────────────────────────────────────────────────

#[test]
fn experiment_result_completed_serde_roundtrip() {
    let exp_id = Uuid::new_v4();
    let started = Utc::now();
    let ended = started + chrono::Duration::seconds(60);
    let mut metrics_before = HashMap::new();
    metrics_before.insert("p99_latency_ms".into(), 48.0_f64);
    let mut metrics_after = metrics_before.clone();
    metrics_after.insert("p99_latency_ms".into(), 548.0_f64);

    let result = ExperimentResult {
        experiment_id: exp_id,
        status: ExperimentStatus::Completed,
        started_at: started,
        ended_at: Some(ended),
        affected_targets: vec!["staging-pod-abc".into()],
        metrics_before,
        metrics_after,
        rollback_triggered: false,
        error: None,
        events: vec![],
    };
    let json = serde_json::to_string(&result).unwrap();
    let back: ExperimentResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.experiment_id, exp_id);
    assert_eq!(back.status, ExperimentStatus::Completed);
    assert!(!back.rollback_triggered);
}

#[test]
fn experiment_result_failed_with_error() {
    let exp_id = Uuid::new_v4();
    let result = ExperimentResult {
        experiment_id: exp_id,
        status: ExperimentStatus::Failed,
        started_at: Utc::now(),
        ended_at: Some(Utc::now()),
        affected_targets: vec![],
        metrics_before: HashMap::new(),
        metrics_after: HashMap::new(),
        rollback_triggered: false,
        error: Some("namespace 'kube-system' is protected".into()),
        events: vec![],
    };
    assert!(result.error.is_some());
}

// ─── ExperimentSchedule ──────────────────────────────────────────────────────

#[test]
fn experiment_schedule_serde_roundtrip() {
    let sched = ExperimentSchedule {
        id: Uuid::new_v4(),
        experiment_id: Uuid::new_v4(),
        cron_expression: "0 2 * * 1".into(),
        enabled: true,
        last_run: None,
        next_run: None,
        max_runs: Some(10),
        run_count: 0,
    };
    let json = serde_json::to_string(&sched).unwrap();
    let back: ExperimentSchedule = serde_json::from_str(&json).unwrap();
    assert_eq!(back.cron_expression, "0 2 * * 1");
    assert_eq!(back.max_runs, Some(10));
    assert!(back.enabled);
}

#[test]
fn experiment_schedule_run_count_increments() {
    let mut sched = ExperimentSchedule {
        id: Uuid::new_v4(),
        experiment_id: Uuid::new_v4(),
        cron_expression: "0 * * * *".into(),
        enabled: true,
        last_run: None,
        next_run: None,
        max_runs: None,
        run_count: 0,
    };
    sched.run_count += 1;
    assert_eq!(sched.run_count, 1);
}

// ─── Extended ExperimentType ──────────────────────────────────────────────────

#[test]
fn extended_experiment_types_serde() {
    // Types added beyond the original 6
    let new_types = vec![
        ExperimentType::NetworkCorruption,
        ExperimentType::NetworkBandwidth,
        ExperimentType::NetworkPartition,
        ExperimentType::IoLatency,
        ExperimentType::IoChaos,
        ExperimentType::ProcessKill,
        ExperimentType::NodeDrain,
        ExperimentType::ClockSkew,
        ExperimentType::HttpFault,
        ExperimentType::GrpcFault,
        ExperimentType::JvmException,
    ];
    for t in new_types {
        let json = serde_json::to_string(&t).unwrap();
        let back: ExperimentType = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back, "roundtrip failed for {:?}", t);
    }
}

// ─── Full experiment with extended fields ────────────────────────────────────

#[test]
fn full_experiment_with_blast_radius_and_safety_guard_roundtrip() {
    let exp = make_full_experiment(ExperimentType::NetworkLatency);
    let json = serde_json::to_string(&exp).unwrap();
    let back: ChaosExperiment = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, exp.id);
    assert!(back.safety_guard.enabled);
}
