// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Failing tests for chaos experiment workflows.
//! Workflows are a core Chaos Mesh feature: sequential/parallel experiment chains.
//! These types are NEW (not in origin/main).

use cave_chaos::models::{
    BlastRadius, ChaosExperiment, ChaosTarget, ExperimentParams, ExperimentStatus, ExperimentType,
    SafetyGuard,
};
use cave_chaos::workflow::{
    execute_workflow, WorkflowExecutionResult, WorkflowNode, WorkflowNodeResult,
    WorkflowNodeStatus, WorkflowNodeType, WorkflowStatus,
};
use cave_chaos::workflow::{ChaosWorkflow, WorkflowTemplate};
use std::collections::HashMap;
use uuid::Uuid;
use chrono::Utc;

fn make_exp(ns: &str, exp_type: ExperimentType) -> ChaosExperiment {
    ChaosExperiment {
        id: Uuid::new_v4(),
        name: "wf-exp".into(),
        experiment_type: exp_type,
        target: ChaosTarget {
            namespace: ns.into(),
            selector: HashMap::new(),
            pod_count: Some(1),
        },
        parameters: ExperimentParams {
            latency_ms: Some(50),
            packet_loss_percent: None,
            cpu_load_percent: None,
            memory_mb: None,
        },
        status: ExperimentStatus::Draft,
        created_at: Utc::now(),
        started_at: None,
        ended_at: None,
        duration_secs: 10,
        blast_radius: BlastRadius::default(),
        safety_guard: SafetyGuard::default(),
        result: None,
        annotations: HashMap::new(),
    }
}

// ─── WorkflowTemplate model ───────────────────────────────────────────────────

#[test]
fn workflow_template_serde_roundtrip() {
    let tmpl = WorkflowTemplate {
        id: Uuid::new_v4(),
        name: "resilience-test".into(),
        description: Some("Network + pod kill chain".into()),
        nodes: vec![
            WorkflowNode {
                id: "step-1".into(),
                node_type: WorkflowNodeType::Sequential,
                experiment_id: Some(Uuid::new_v4()),
                children: vec![],
                deadline_secs: Some(120),
            },
        ],
        created_at: Utc::now(),
        labels: HashMap::new(),
    };
    let json = serde_json::to_string(&tmpl).unwrap();
    let back: WorkflowTemplate = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "resilience-test");
    assert_eq!(back.nodes.len(), 1);
}

#[test]
fn workflow_node_parallel_type_serde() {
    let node = WorkflowNode {
        id: "par-1".into(),
        node_type: WorkflowNodeType::Parallel,
        experiment_id: None,
        children: vec!["child-a".into(), "child-b".into()],
        deadline_secs: None,
    };
    let json = serde_json::to_string(&node).unwrap();
    let back: WorkflowNode = serde_json::from_str(&json).unwrap();
    assert_eq!(back.node_type, WorkflowNodeType::Parallel);
    assert_eq!(back.children.len(), 2);
}

// ─── ChaosWorkflow (running instance) ────────────────────────────────────────

#[test]
fn chaos_workflow_initial_state_pending() {
    let wf = ChaosWorkflow::new(Uuid::new_v4(), "test-workflow");
    assert_eq!(wf.status, WorkflowStatus::Pending);
    assert!(wf.node_results.is_empty());
}

#[test]
fn chaos_workflow_serde_roundtrip() {
    let wf = ChaosWorkflow::new(Uuid::new_v4(), "resilience-test");
    let json = serde_json::to_string(&wf).unwrap();
    let back: ChaosWorkflow = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "resilience-test");
    assert_eq!(back.status, WorkflowStatus::Pending);
}

// ─── Sequential workflow execution ───────────────────────────────────────────

#[test]
fn sequential_workflow_runs_all_steps() {
    let exp1 = make_exp("staging", ExperimentType::NetworkLatency);
    let exp2 = make_exp("staging", ExperimentType::CpuStress);

    let exp1_id = exp1.id;
    let exp2_id = exp2.id;

    let experiments = vec![exp1, exp2];

    let nodes = vec![
        WorkflowNode {
            id: "step-1".into(),
            node_type: WorkflowNodeType::Sequential,
            experiment_id: Some(exp1_id),
            children: vec![],
            deadline_secs: Some(60),
        },
        WorkflowNode {
            id: "step-2".into(),
            node_type: WorkflowNodeType::Sequential,
            experiment_id: Some(exp2_id),
            children: vec![],
            deadline_secs: Some(60),
        },
    ];

    let result = execute_workflow(&nodes, &experiments);
    assert_eq!(result.status, WorkflowStatus::Completed);
    assert_eq!(result.node_results.len(), 2);
    for nr in &result.node_results {
        assert_eq!(nr.status, WorkflowNodeStatus::Completed);
    }
}

#[test]
fn sequential_workflow_unknown_experiment_fails() {
    let nodes = vec![WorkflowNode {
        id: "bad-step".into(),
        node_type: WorkflowNodeType::Sequential,
        experiment_id: Some(Uuid::new_v4()), // not in experiments list
        children: vec![],
        deadline_secs: Some(30),
    }];
    let result = execute_workflow(&nodes, &[]);
    assert_eq!(result.status, WorkflowStatus::Failed);
    assert!(result.error.is_some());
}

// ─── Parallel workflow execution ──────────────────────────────────────────────

#[test]
fn parallel_workflow_runs_concurrent_steps() {
    let exp1 = make_exp("staging", ExperimentType::NetworkLatency);
    let exp2 = make_exp("staging", ExperimentType::CpuStress);

    let exp1_id = exp1.id;
    let exp2_id = exp2.id;

    let experiments = vec![exp1, exp2];

    // A parallel node groups the children
    let nodes = vec![WorkflowNode {
        id: "par-root".into(),
        node_type: WorkflowNodeType::Parallel,
        experiment_id: None,
        children: vec![exp1_id.to_string(), exp2_id.to_string()],
        deadline_secs: Some(60),
    }];

    let result = execute_workflow(&nodes, &experiments);
    assert_eq!(result.status, WorkflowStatus::Completed);
    // parallel groups produce one result per child
    assert!(!result.node_results.is_empty());
}

// ─── WorkflowNodeResult ───────────────────────────────────────────────────────

#[test]
fn workflow_node_result_serde_roundtrip() {
    let nr = WorkflowNodeResult {
        node_id: "step-1".into(),
        status: WorkflowNodeStatus::Completed,
        experiment_id: Some(Uuid::new_v4()),
        error: None,
        started_at: Some(Utc::now()),
        ended_at: Some(Utc::now()),
    };
    let json = serde_json::to_string(&nr).unwrap();
    let back: WorkflowNodeResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.node_id, "step-1");
    assert_eq!(back.status, WorkflowNodeStatus::Completed);
}

// ─── WorkflowStatus transitions ──────────────────────────────────────────────

#[test]
fn workflow_status_variants_serde() {
    for s in [
        WorkflowStatus::Pending,
        WorkflowStatus::Running,
        WorkflowStatus::Completed,
        WorkflowStatus::Failed,
        WorkflowStatus::Aborted,
    ] {
        let json = serde_json::to_string(&s).unwrap();
        let back: WorkflowStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
