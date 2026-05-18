// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{
    ExecutionStatus, Workflow, WorkflowExecution, WorkflowStats, WorkflowStatus,
};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct WorkflowStore {
    workflows: RwLock<HashMap<Uuid, Workflow>>,
    executions: RwLock<HashMap<Uuid, WorkflowExecution>>,
}

impl WorkflowStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Workflows ─────────────────────────────────────────────────────────────

    pub fn create_workflow(&self, wf: Workflow) -> Workflow {
        let mut workflows = self.workflows.write().unwrap();
        let w = wf.clone();
        workflows.insert(wf.id, wf);
        w
    }

    pub fn get_workflow(&self, id: &Uuid) -> Option<Workflow> {
        self.workflows.read().unwrap().get(id).cloned()
    }

    pub fn update_workflow(
        &self,
        id: &Uuid,
        name: Option<String>,
        description: Option<String>,
        trigger: Option<crate::models::TriggerType>,
        trigger_config: Option<serde_json::Value>,
        nodes: Option<Vec<crate::models::WorkflowNode>>,
    ) -> Option<Workflow> {
        let mut workflows = self.workflows.write().unwrap();
        if let Some(wf) = workflows.get_mut(id) {
            if let Some(n) = name {
                wf.name = n;
            }
            if let Some(d) = description {
                wf.description = d;
            }
            if let Some(t) = trigger {
                wf.trigger = t;
            }
            if let Some(tc) = trigger_config {
                wf.trigger_config = tc;
            }
            if let Some(ns) = nodes {
                wf.nodes = ns;
            }
            wf.updated_at = Utc::now();
            return Some(wf.clone());
        }
        None
    }

    pub fn delete_workflow(&self, id: &Uuid) -> Option<Workflow> {
        self.workflows.write().unwrap().remove(id)
    }

    pub fn list_workflows(&self) -> Vec<Workflow> {
        let mut wfs: Vec<Workflow> =
            self.workflows.read().unwrap().values().cloned().collect();
        wfs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        wfs
    }

    pub fn set_status(&self, id: &Uuid, status: WorkflowStatus) -> Option<Workflow> {
        let mut workflows = self.workflows.write().unwrap();
        if let Some(wf) = workflows.get_mut(id) {
            wf.status = status;
            wf.updated_at = Utc::now();
            return Some(wf.clone());
        }
        None
    }

    pub fn record_execution(&self, exec: WorkflowExecution) -> WorkflowExecution {
        // Update workflow stats
        {
            let mut workflows = self.workflows.write().unwrap();
            if let Some(wf) = workflows.get_mut(&exec.workflow_id) {
                wf.execution_count += 1;
                if matches!(exec.status, ExecutionStatus::Failed) {
                    wf.error_count += 1;
                }
                wf.last_executed_at = Some(exec.started_at);
            }
        }
        let mut executions = self.executions.write().unwrap();
        let e = exec.clone();
        executions.insert(exec.id, exec);
        e
    }

    // ── Executions ────────────────────────────────────────────────────────────

    pub fn get_execution(&self, id: &Uuid) -> Option<WorkflowExecution> {
        self.executions.read().unwrap().get(id).cloned()
    }

    pub fn list_executions(&self, workflow_id: &Uuid, limit: usize) -> Vec<WorkflowExecution> {
        let mut execs: Vec<WorkflowExecution> = self
            .executions
            .read()
            .unwrap()
            .values()
            .filter(|e| e.workflow_id == *workflow_id)
            .cloned()
            .collect();
        execs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        execs.truncate(limit);
        execs
    }

    // ── Stats ─────────────────────────────────────────────────────────────────

    pub fn compute_stats(&self) -> WorkflowStats {
        let workflows = self.workflows.read().unwrap();
        let executions = self.executions.read().unwrap();

        let total = workflows.len() as u64;
        let active = workflows
            .values()
            .filter(|w| matches!(w.status, WorkflowStatus::Active))
            .count() as u64;
        let total_executions = executions.len() as u64;

        let success_count = executions
            .values()
            .filter(|e| matches!(e.status, ExecutionStatus::Completed))
            .count() as f64;
        let success_rate = if total_executions == 0 {
            0.0
        } else {
            success_count / total_executions as f64
        };

        let durations: Vec<u64> = executions
            .values()
            .filter_map(|e| e.duration_ms)
            .collect();
        let avg_duration_ms = if durations.is_empty() {
            0.0
        } else {
            durations.iter().sum::<u64>() as f64 / durations.len() as f64
        };

        WorkflowStats {
            total,
            active,
            total_executions,
            success_rate,
            avg_duration_ms,
        }
    }
}
