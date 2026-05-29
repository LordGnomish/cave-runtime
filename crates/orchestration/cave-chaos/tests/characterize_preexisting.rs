// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Characterization tests for pre-existing cave-chaos modules:
//! models, engine, routes.
//! These modules exist on origin/main and are absorbed (not TDD-authored).
//! Tests assert real existing behaviour — they must pass immediately.

use cave_chaos::engine::{actual_duration_secs, is_active, is_high_risk, validate_experiment};
use cave_chaos::models::{
    BlastRadius, ChaosExperiment, ChaosTarget, ExperimentParams, ExperimentStatus, ExperimentType,
    SafetyGuard,
};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

fn base_exp(exp_type: ExperimentType, ns: &str, params: ExperimentParams) -> ChaosExperiment {
    ChaosExperiment {
        id: Uuid::new_v4(),
        name: "char-test".to_string(),
        experiment_type: exp_type,
        target: ChaosTarget {
            namespace: ns.to_string(),
            selector: HashMap::new(),
            pod_count: None,
        },
        parameters: params,
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

fn no_params() -> ExperimentParams {
    ExperimentParams {
        latency_ms: None,
        packet_loss_percent: None,
        cpu_load_percent: None,
        memory_mb: None,
    }
}

// ─── engine characterization ──────────────────────────────────────────────────

#[test]
fn char_engine_validate_latency_requires_latency_ms() {
    let exp = base_exp(ExperimentType::NetworkLatency, "staging", no_params());
    let errs = validate_experiment(&exp);
    assert!(
        errs.iter().any(|e| e.contains("latency_ms")),
        "missing latency_ms should produce error"
    );
}

#[test]
fn char_engine_validate_latency_valid() {
    let params = ExperimentParams {
        latency_ms: Some(50),
        ..no_params()
    };
    let exp = base_exp(ExperimentType::NetworkLatency, "staging", params);
    assert!(validate_experiment(&exp).is_empty());
}

#[test]
fn char_engine_validate_packet_loss_out_of_range() {
    let params = ExperimentParams {
        packet_loss_percent: Some(200.0),
        ..no_params()
    };
    let exp = base_exp(ExperimentType::NetworkPacketLoss, "staging", params);
    let errs = validate_experiment(&exp);
    assert!(!errs.is_empty());
}

#[test]
fn char_engine_is_active_only_running() {
    let running = ChaosExperiment {
        status: ExperimentStatus::Running,
        ..base_exp(ExperimentType::PodKill, "staging", no_params())
    };
    let draft = ChaosExperiment {
        status: ExperimentStatus::Draft,
        ..base_exp(ExperimentType::PodKill, "staging", no_params())
    };
    assert!(is_active(&running));
    assert!(!is_active(&draft));
}

#[test]
fn char_engine_is_high_risk_production_namespaces() {
    let prod = base_exp(ExperimentType::PodKill, "production", no_params());
    let prod2 = base_exp(ExperimentType::PodKill, "prod", no_params());
    let staging = base_exp(ExperimentType::PodKill, "staging", no_params());
    assert!(is_high_risk(&prod));
    assert!(is_high_risk(&prod2));
    assert!(!is_high_risk(&staging));
}

#[test]
fn char_engine_actual_duration_secs_calculates_diff() {
    let now = Utc::now();
    let mut exp = base_exp(
        ExperimentType::CpuStress,
        "staging",
        ExperimentParams {
            cpu_load_percent: Some(80),
            ..no_params()
        },
    );
    exp.started_at = Some(now);
    exp.ended_at = Some(now + Duration::seconds(30));
    assert_eq!(actual_duration_secs(&exp), Some(30));
}

#[test]
fn char_engine_actual_duration_secs_none_without_both() {
    let exp = base_exp(ExperimentType::CpuStress, "staging", no_params());
    assert_eq!(actual_duration_secs(&exp), None);
}

// ─── models characterization ──────────────────────────────────────────────────

#[test]
fn char_models_experiment_type_serde_roundtrip() {
    let types = vec![
        ExperimentType::NetworkLatency,
        ExperimentType::NetworkPacketLoss,
        ExperimentType::CpuStress,
        ExperimentType::MemoryStress,
        ExperimentType::PodKill,
        ExperimentType::DiskFill,
    ];
    for t in types {
        let json = serde_json::to_string(&t).unwrap();
        let back: ExperimentType = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }
}

#[test]
fn char_models_status_serde_roundtrip() {
    let statuses = vec![
        ExperimentStatus::Draft,
        ExperimentStatus::Running,
        ExperimentStatus::Completed,
        ExperimentStatus::Aborted,
        ExperimentStatus::Failed,
    ];
    for s in statuses {
        let json = serde_json::to_string(&s).unwrap();
        let back: ExperimentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}

#[test]
fn char_models_experiment_serde_roundtrip() {
    let exp = base_exp(
        ExperimentType::NetworkLatency,
        "staging",
        ExperimentParams {
            latency_ms: Some(100),
            ..no_params()
        },
    );
    let json = serde_json::to_string(&exp).unwrap();
    let back: ChaosExperiment = serde_json::from_str(&json).unwrap();
    assert_eq!(exp.id, back.id);
    assert_eq!(exp.name, back.name);
}

// ─── routes characterization (module-level struct is accessible) ──────────────

#[test]
fn char_routes_module_name_is_chaos() {
    // Just verify the module constant compiles and has expected value
    assert_eq!(cave_chaos::MODULE_NAME, "chaos");
}
