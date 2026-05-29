// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Experiment workflow engine — sequential and parallel fault injection chains.
//!
//! Maps to Chaos Mesh's Workflow CRD, which orchestrates multiple chaos
//! experiments in a DAG of serial and parallel steps.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::executor::ChaosExecutor;
use crate::models::{ChaosExperiment, ExperimentStatus};

/// A workflow template — describes the DAG of chaos nodes.
/// Maps to Chaos Mesh's `Workflow` CR spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowTemplate {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// Ordered root-level nodes (executed sequentially at the top level).
    pub nodes: Vec<WorkflowNode>,
    pub created_at: DateTime<Utc>,
    pub labels: HashMap<String, String>,
}

/// A node within a workflow DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowNode {
    /// Unique identifier within the workflow (not a UUID — e.g. "step-1").
    pub id: String,
    pub node_type: WorkflowNodeType,
    /// Points to a `ChaosExperiment` id if this is a leaf node.
    pub experiment_id: Option<Uuid>,
    /// Child node IDs for `Parallel` groups (empty for leaf `Sequential` nodes).
    pub children: Vec<String>,
    /// Per-node timeout in seconds.
    pub deadline_secs: Option<u64>,
}

/// How child nodes within this node are executed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodeType {
    /// Run this node (and the next sibling) serially.
    Sequential,
    /// Run all children concurrently.
    Parallel,
    /// Suspend for a fixed duration (no experiment).
    Suspend,
}

/// Overall status of a workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Aborted,
}

/// Status of an individual node in a running workflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

/// Result for one workflow node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowNodeResult {
    pub node_id: String,
    pub status: WorkflowNodeStatus,
    pub experiment_id: Option<Uuid>,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// Result for the entire workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowExecutionResult {
    pub status: WorkflowStatus,
    pub node_results: Vec<WorkflowNodeResult>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

/// A running workflow instance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChaosWorkflow {
    pub id: Uuid,
    pub template_id: Uuid,
    pub name: String,
    pub status: WorkflowStatus,
    pub node_results: Vec<WorkflowNodeResult>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl ChaosWorkflow {
    /// Create a new workflow instance in Pending state.
    pub fn new(template_id: Uuid, name: &str) -> Self {
        ChaosWorkflow {
            id: Uuid::new_v4(),
            template_id,
            name: name.to_string(),
            status: WorkflowStatus::Pending,
            node_results: vec![],
            error: None,
            created_at: Utc::now(),
            started_at: None,
            ended_at: None,
        }
    }
}

/// Execute a workflow defined by `nodes` against the provided `experiments`.
///
/// Sequential nodes run one after another in order.
/// Parallel nodes fan-out all children concurrently (simulated synchronously
/// since ChaosExecutor is synchronous).
///
/// Returns a `WorkflowExecutionResult` capturing all node outcomes.
pub fn execute_workflow(
    nodes: &[WorkflowNode],
    experiments: &[ChaosExperiment],
) -> WorkflowExecutionResult {
    let executor = ChaosExecutor::new();
    let started_at = Utc::now();
    let mut node_results: Vec<WorkflowNodeResult> = vec![];

    // Build experiment lookup by UUID
    let exp_map: HashMap<Uuid, ChaosExperiment> =
        experiments.iter().map(|e| (e.id, e.clone())).collect();

    for node in nodes {
        match node.node_type {
            WorkflowNodeType::Sequential => {
                let nr = run_leaf_node(node, &exp_map, &executor);
                let failed = nr.status == WorkflowNodeStatus::Failed;
                node_results.push(nr);
                if failed {
                    return WorkflowExecutionResult {
                        status: WorkflowStatus::Failed,
                        node_results,
                        error: Some(format!(
                            "sequential node '{}' failed — workflow aborted",
                            node.id
                        )),
                        started_at,
                        ended_at: Utc::now(),
                    };
                }
            }
            WorkflowNodeType::Parallel => {
                // Run each child ID as a parallel branch.
                // Children can be experiment UUIDs or node IDs — we attempt UUID parse.
                let mut any_failed = false;
                for child_str in &node.children {
                    let child_uuid = Uuid::parse_str(child_str);
                    let nr = match child_uuid {
                        Ok(eid) => run_experiment_by_id(&node.id, Some(eid), &exp_map, &executor),
                        Err(_) => WorkflowNodeResult {
                            node_id: child_str.clone(),
                            status: WorkflowNodeStatus::Failed,
                            experiment_id: None,
                            error: Some(format!("child '{}' is not a valid experiment UUID", child_str)),
                            started_at: Some(Utc::now()),
                            ended_at: Some(Utc::now()),
                        },
                    };
                    if nr.status == WorkflowNodeStatus::Failed {
                        any_failed = true;
                    }
                    node_results.push(nr);
                }
                if any_failed {
                    return WorkflowExecutionResult {
                        status: WorkflowStatus::Failed,
                        node_results,
                        error: Some(format!(
                            "one or more children of parallel node '{}' failed",
                            node.id
                        )),
                        started_at,
                        ended_at: Utc::now(),
                    };
                }
            }
            WorkflowNodeType::Suspend => {
                // Suspend node is a no-op in simulation
                node_results.push(WorkflowNodeResult {
                    node_id: node.id.clone(),
                    status: WorkflowNodeStatus::Completed,
                    experiment_id: None,
                    error: None,
                    started_at: Some(Utc::now()),
                    ended_at: Some(Utc::now()),
                });
            }
        }
    }

    WorkflowExecutionResult {
        status: WorkflowStatus::Completed,
        node_results,
        error: None,
        started_at,
        ended_at: Utc::now(),
    }
}

fn run_leaf_node(
    node: &WorkflowNode,
    exp_map: &HashMap<Uuid, ChaosExperiment>,
    executor: &ChaosExecutor,
) -> WorkflowNodeResult {
    run_experiment_by_id(&node.id, node.experiment_id, exp_map, executor)
}

fn run_experiment_by_id(
    node_id: &str,
    experiment_id: Option<Uuid>,
    exp_map: &HashMap<Uuid, ChaosExperiment>,
    executor: &ChaosExecutor,
) -> WorkflowNodeResult {
    let eid = match experiment_id {
        Some(id) => id,
        None => {
            return WorkflowNodeResult {
                node_id: node_id.to_string(),
                status: WorkflowNodeStatus::Failed,
                experiment_id: None,
                error: Some("no experiment_id specified for sequential node".to_string()),
                started_at: Some(Utc::now()),
                ended_at: Some(Utc::now()),
            };
        }
    };

    let mut exp = match exp_map.get(&eid) {
        Some(e) => e.clone(),
        None => {
            return WorkflowNodeResult {
                node_id: node_id.to_string(),
                status: WorkflowNodeStatus::Failed,
                experiment_id: Some(eid),
                error: Some(format!("experiment {} not found in workflow context", eid)),
                started_at: Some(Utc::now()),
                ended_at: Some(Utc::now()),
            };
        }
    };

    let started_at = Utc::now();
    let result = executor.execute(&mut exp);
    let ended_at = Utc::now();

    let status = if result.status == ExperimentStatus::Completed {
        WorkflowNodeStatus::Completed
    } else if result.status == ExperimentStatus::Failed {
        WorkflowNodeStatus::Failed
    } else {
        WorkflowNodeStatus::Completed
    };

    WorkflowNodeResult {
        node_id: node_id.to_string(),
        status,
        experiment_id: Some(eid),
        error: result.error,
        started_at: Some(started_at),
        ended_at: Some(ended_at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        BlastRadius, ChaosTarget, ExperimentParams, ExperimentType, SafetyGuard,
    };
    use std::collections::HashMap;

    fn make_exp(exp_type: ExperimentType, ns: &str) -> ChaosExperiment {
        ChaosExperiment {
            id: Uuid::new_v4(),
            name: "wf-test".into(),
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

    #[test]
    fn test_sequential_workflow_completes() {
        let exp = make_exp(ExperimentType::NetworkLatency, "staging");
        let eid = exp.id;
        let nodes = vec![WorkflowNode {
            id: "s1".into(),
            node_type: WorkflowNodeType::Sequential,
            experiment_id: Some(eid),
            children: vec![],
            deadline_secs: Some(60),
        }];
        let result = execute_workflow(&nodes, &[exp]);
        assert_eq!(result.status, WorkflowStatus::Completed);
        assert_eq!(result.node_results[0].status, WorkflowNodeStatus::Completed);
    }

    #[test]
    fn test_sequential_workflow_fails_on_protected_namespace() {
        let exp = make_exp(ExperimentType::PodKill, "kube-system");
        let eid = exp.id;
        let nodes = vec![WorkflowNode {
            id: "s1".into(),
            node_type: WorkflowNodeType::Sequential,
            experiment_id: Some(eid),
            children: vec![],
            deadline_secs: Some(60),
        }];
        let result = execute_workflow(&nodes, &[exp]);
        assert_eq!(result.status, WorkflowStatus::Failed);
    }

    #[test]
    fn test_sequential_fails_on_missing_experiment() {
        let nodes = vec![WorkflowNode {
            id: "s1".into(),
            node_type: WorkflowNodeType::Sequential,
            experiment_id: Some(Uuid::new_v4()),
            children: vec![],
            deadline_secs: None,
        }];
        let result = execute_workflow(&nodes, &[]);
        assert_eq!(result.status, WorkflowStatus::Failed);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_parallel_workflow_runs_both() {
        let exp1 = make_exp(ExperimentType::NetworkLatency, "staging");
        let mut exp2 = make_exp(ExperimentType::CpuStress, "staging");
        exp2.parameters.cpu_load_percent = Some(80);
        let e1id = exp1.id;
        let e2id = exp2.id;
        let nodes = vec![WorkflowNode {
            id: "par".into(),
            node_type: WorkflowNodeType::Parallel,
            experiment_id: None,
            children: vec![e1id.to_string(), e2id.to_string()],
            deadline_secs: None,
        }];
        let result = execute_workflow(&nodes, &[exp1, exp2]);
        assert_eq!(result.status, WorkflowStatus::Completed);
        assert_eq!(result.node_results.len(), 2);
    }

    #[test]
    fn test_suspend_node_completes() {
        let nodes = vec![WorkflowNode {
            id: "wait".into(),
            node_type: WorkflowNodeType::Suspend,
            experiment_id: None,
            children: vec![],
            deadline_secs: Some(5),
        }];
        let result = execute_workflow(&nodes, &[]);
        assert_eq!(result.status, WorkflowStatus::Completed);
    }

    #[test]
    fn test_workflow_new_is_pending() {
        let wf = ChaosWorkflow::new(Uuid::new_v4(), "my-workflow");
        assert_eq!(wf.status, WorkflowStatus::Pending);
        assert!(wf.node_results.is_empty());
    }
}
