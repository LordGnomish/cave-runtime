// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: additional Chaos Mesh fault types beyond the original 17 —
//! NetworkDuplicate, DNSChaos, KernelChaos, BlockChaos, PhysicalMachineChaos.
//! Covers serde naming, type-specific validation, and executor simulation.

use cave_chaos::engine::validate_experiment;
use cave_chaos::executor::ChaosExecutor;
use cave_chaos::models::{
    BlastRadius, ChaosExperiment, ChaosTarget, ExperimentParams, ExperimentStatus, ExperimentType,
    SafetyGuard,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn exp(t: ExperimentType, params: ExperimentParams) -> ChaosExperiment {
    ChaosExperiment {
        id: Uuid::new_v4(),
        name: "ef".to_string(),
        experiment_type: t,
        target: ChaosTarget { namespace: "staging".to_string(), selector: HashMap::new(), pod_count: None },
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

fn empty() -> ExperimentParams {
    ExperimentParams { latency_ms: None, packet_loss_percent: None, cpu_load_percent: None, memory_mb: None }
}

#[test]
fn test_new_variants_serde_names() {
    let cases = [
        (ExperimentType::NetworkDuplicate, "\"network_duplicate\""),
        (ExperimentType::DnsChaos, "\"dns_chaos\""),
        (ExperimentType::KernelChaos, "\"kernel_chaos\""),
        (ExperimentType::BlockChaos, "\"block_chaos\""),
        (ExperimentType::PhysicalMachineChaos, "\"physical_machine_chaos\""),
    ];
    for (variant, name) in cases {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, name);
        let back: ExperimentType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn test_network_duplicate_requires_percent() {
    let errors = validate_experiment(&exp(ExperimentType::NetworkDuplicate, empty()));
    assert!(errors.iter().any(|e| e.contains("packet_loss_percent")), "got: {errors:?}");
}

#[test]
fn test_network_duplicate_percent_over_100() {
    let p = ExperimentParams { packet_loss_percent: Some(150.0), ..empty() };
    let errors = validate_experiment(&exp(ExperimentType::NetworkDuplicate, p));
    assert!(errors.iter().any(|e| e.contains("packet_loss_percent")));
}

#[test]
fn test_network_duplicate_valid() {
    let p = ExperimentParams { packet_loss_percent: Some(5.0), ..empty() };
    let errors = validate_experiment(&exp(ExperimentType::NetworkDuplicate, p));
    assert!(errors.is_empty(), "got: {errors:?}");
}

#[test]
fn test_dns_kernel_block_pm_have_no_required_params() {
    for t in [
        ExperimentType::DnsChaos,
        ExperimentType::KernelChaos,
        ExperimentType::BlockChaos,
        ExperimentType::PhysicalMachineChaos,
    ] {
        let errors = validate_experiment(&exp(t.clone(), empty()));
        assert!(errors.is_empty(), "{t:?} should need no params, got: {errors:?}");
    }
}

#[test]
fn test_executor_completes_new_types() {
    let executor = ChaosExecutor::new();
    for t in [
        ExperimentType::DnsChaos,
        ExperimentType::KernelChaos,
        ExperimentType::BlockChaos,
        ExperimentType::PhysicalMachineChaos,
    ] {
        let mut e = exp(t.clone(), empty());
        let result = executor.execute(&mut e);
        assert_eq!(result.status, ExperimentStatus::Completed, "type {t:?} should complete");
        assert!(!result.events.is_empty());
        assert!(!result.metrics_after.is_empty());
    }
}

#[test]
fn test_executor_network_duplicate_completes() {
    let executor = ChaosExecutor::new();
    let p = ExperimentParams { packet_loss_percent: Some(5.0), ..empty() };
    let mut e = exp(ExperimentType::NetworkDuplicate, p);
    let result = executor.execute(&mut e);
    assert_eq!(result.status, ExperimentStatus::Completed);
}
