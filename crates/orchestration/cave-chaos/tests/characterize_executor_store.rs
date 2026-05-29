// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Characterization tests for the pre-existing orphan modules:
//! executor.rs and store.rs (existed on origin/main but not compiled via lib.rs).
//! These are now wired in as pub mod. Tests verify real existing behaviour.

use cave_chaos::executor::ChaosExecutor;
use cave_chaos::models::{
    BlastRadius, ChaosExperiment, ChaosTarget, ExperimentParams, ExperimentSchedule,
    ExperimentStatus, ExperimentType, SafetyGuard,
};
use cave_chaos::store::ChaosStore;
use std::collections::HashMap;
use uuid::Uuid;
use chrono::Utc;

fn make_run_experiment(exp_type: ExperimentType, namespace: &str, duration_secs: u32) -> ChaosExperiment {
    ChaosExperiment {
        id: Uuid::new_v4(),
        name: "char-executor".into(),
        experiment_type: exp_type,
        target: ChaosTarget {
            namespace: namespace.into(),
            selector: HashMap::new(),
            pod_count: Some(1),
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
        duration_secs,
        blast_radius: BlastRadius::default(),
        safety_guard: SafetyGuard::default(),
        result: None,
        annotations: HashMap::new(),
    }
}

fn make_store_experiment(status: ExperimentStatus) -> ChaosExperiment {
    ChaosExperiment {
        id: Uuid::new_v4(),
        name: "store-char".into(),
        experiment_type: ExperimentType::PodKill,
        target: ChaosTarget {
            namespace: "staging".into(),
            selector: HashMap::new(),
            pod_count: None,
        },
        parameters: ExperimentParams {
            latency_ms: None,
            packet_loss_percent: None,
            cpu_load_percent: None,
            memory_mb: None,
        },
        status,
        created_at: Utc::now(),
        started_at: None,
        ended_at: None,
        duration_secs: 30,
        blast_radius: BlastRadius::default(),
        safety_guard: SafetyGuard::default(),
        result: None,
        annotations: HashMap::new(),
    }
}

// ─── ChaosExecutor characterization ──────────────────────────────────────────

#[test]
fn char_executor_execute_completes_experiment() {
    let exec = ChaosExecutor::new();
    let mut exp = make_run_experiment(ExperimentType::NetworkLatency, "staging", 60);
    let result = exec.execute(&mut exp);
    assert_eq!(result.status, ExperimentStatus::Completed);
    assert!(!result.events.is_empty());
}

#[test]
fn char_executor_execute_fails_protected_namespace() {
    let exec = ChaosExecutor::new();
    let mut exp = make_run_experiment(ExperimentType::PodKill, "kube-system", 30);
    let result = exec.execute(&mut exp);
    assert_eq!(result.status, ExperimentStatus::Failed);
    assert!(result.error.is_some());
}

#[test]
fn char_executor_execute_sets_timestamps() {
    let exec = ChaosExecutor::new();
    let mut exp = make_run_experiment(ExperimentType::NetworkLatency, "staging", 60);
    exec.execute(&mut exp);
    assert!(exp.started_at.is_some());
    assert!(exp.ended_at.is_some());
}

#[test]
fn char_executor_rollback_sets_aborted() {
    let exec = ChaosExecutor::new();
    let mut exp = make_run_experiment(ExperimentType::PodKill, "staging", 60);
    exp.status = ExperimentStatus::Running;
    exp.started_at = Some(Utc::now());
    let result = exec.rollback(&mut exp);
    assert_eq!(result.status, ExperimentStatus::Aborted);
    assert!(result.rollback_triggered);
}

#[test]
fn char_executor_check_safety_halts_below_threshold() {
    let exec = ChaosExecutor::new();
    let exp = make_run_experiment(ExperimentType::PodKill, "staging", 30);
    // default min_healthy = 0.5; 0.3 < 0.5 → halt
    assert!(exec.check_safety(&exp, 0.3));
    // 0.9 >= 0.5 → do not halt
    assert!(!exec.check_safety(&exp, 0.9));
}

#[test]
fn char_executor_validate_rejects_protected_namespace() {
    let exec = ChaosExecutor::new();
    let exp = make_run_experiment(ExperimentType::PodKill, "kube-system", 60);
    assert!(exec.validate(&exp).is_err());
}

#[test]
fn char_executor_validate_accepts_staging() {
    let exec = ChaosExecutor::new();
    let exp = make_run_experiment(ExperimentType::NetworkLatency, "staging", 60);
    assert!(exec.validate(&exp).is_ok());
}

#[test]
fn char_executor_metrics_degrade_for_network_latency() {
    let exec = ChaosExecutor::new();
    let mut exp = make_run_experiment(ExperimentType::NetworkLatency, "staging", 60);
    let result = exec.execute(&mut exp);
    let before = result.metrics_before["p99_latency_ms"];
    let after = result.metrics_after["p99_latency_ms"];
    assert!(after > before);
}

// ─── ChaosStore characterization ─────────────────────────────────────────────

#[test]
fn char_store_insert_and_get() {
    let store = ChaosStore::new();
    let exp = make_store_experiment(ExperimentStatus::Draft);
    let id = exp.id;
    store.insert(exp);
    let retrieved = store.get(id).unwrap();
    assert_eq!(retrieved.id, id);
}

#[test]
fn char_store_update_existing() {
    let store = ChaosStore::new();
    let mut exp = make_store_experiment(ExperimentStatus::Draft);
    store.insert(exp.clone());
    exp.status = ExperimentStatus::Running;
    assert!(store.update(exp.clone()));
    assert_eq!(store.get(exp.id).unwrap().status, ExperimentStatus::Running);
}

#[test]
fn char_store_remove() {
    let store = ChaosStore::new();
    let exp = make_store_experiment(ExperimentStatus::Draft);
    let id = exp.id;
    store.insert(exp);
    assert!(store.remove(id).is_some());
    assert!(store.get(id).is_none());
}

#[test]
fn char_store_list_by_status() {
    let store = ChaosStore::new();
    store.insert(make_store_experiment(ExperimentStatus::Draft));
    store.insert(make_store_experiment(ExperimentStatus::Draft));
    store.insert(make_store_experiment(ExperimentStatus::Running));
    let drafts = store.list_by_status(&ExperimentStatus::Draft);
    assert_eq!(drafts.len(), 2);
}

#[test]
fn char_store_schedules() {
    let store = ChaosStore::new();
    let sched = ExperimentSchedule {
        id: Uuid::new_v4(),
        experiment_id: Uuid::new_v4(),
        cron_expression: "0 2 * * 1".into(),
        enabled: true,
        last_run: None,
        next_run: None,
        max_runs: Some(5),
        run_count: 0,
    };
    let sid = sched.id;
    store.add_schedule(sched);
    assert!(store.get_schedule(sid).is_some());
    assert_eq!(store.list_schedules().len(), 1);
    store.remove_schedule(sid);
    assert!(store.list_schedules().is_empty());
}
