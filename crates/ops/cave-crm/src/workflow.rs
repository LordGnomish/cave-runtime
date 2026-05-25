// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Workflow engine — mirrors `packages/twenty-server/src/modules/workflow/`.
//!
//! Twenty's workflow engine is a DAG of `WorkflowStep`s scoped to a single
//! `Workspace`. Each step matches the upstream `WorkflowStepType`:
//! `TRIGGER` (entity-event fired from `cave-cdc`), `CODE` (sandboxed user
//! script — a placeholder until the cave-runtime JS isolate lands),
//! `RECORD` (CRUD operation against a CRM entity), `HTTP_REQUEST`
//! (outbound webhook). The engine here covers the control-plane state
//! machine: DAG validation, run state, edge transitions, and execution
//! traces. The user-script sandbox is intentionally out of scope — it
//! belongs to cave-runtime's evaluator wave.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Type discriminator from upstream `workflow.workspace-entity.ts::WorkflowStepType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowStepType {
    Trigger,
    Code,
    Record,
    HttpRequest,
}

/// A single node in the workflow DAG. `id` is unique per workflow,
/// `next` is the (possibly empty) list of downstream step ids.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub name: String,
    pub kind: WorkflowStepType,
    /// JSON-serialised step settings. Upstream stores this as `JsonValue`.
    pub settings: serde_json::Value,
    /// Downstream-edges.
    pub next: Vec<String>,
}

/// A versioned workflow definition. `is_active = true` is what the
/// trigger-dispatcher consults at fire time. Mirrors upstream
/// `WorkflowVersion`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub version: u32,
    pub is_active: bool,
    pub steps: Vec<WorkflowStep>,
    pub root_step_id: String,
}

/// Run status — `RUNNING -> COMPLETED | FAILED`. Upstream tracks this on
/// `WorkflowRun.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowRunStatus {
    Running,
    Completed,
    Failed,
}

/// Single instance of a workflow execution. Stores the trace of step ids
/// visited in the order they fired, plus the final status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub status: WorkflowRunStatus,
    pub trace: Vec<String>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct WorkflowStore {
    workflows: HashMap<String, Workflow>,
    runs: HashMap<String, WorkflowRun>,
}

impl WorkflowStore {
    /// Register or replace a workflow. Returns an error if the DAG is
    /// malformed (cycle / dangling next / missing root).
    pub fn put_workflow(&mut self, w: Workflow) -> Result<(), String> {
        validate_dag(&w)?;
        self.workflows.insert(w.id.clone(), w);
        Ok(())
    }

    pub fn get_workflow(&self, id: &str) -> Option<&Workflow> {
        self.workflows.get(id)
    }

    pub fn list_workflows(&self, workspace_id: &str) -> Vec<&Workflow> {
        let mut out: Vec<_> = self
            .workflows
            .values()
            .filter(|w| w.workspace_id == workspace_id)
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn delete_workflow(&mut self, id: &str) -> bool {
        self.workflows.remove(id).is_some()
    }

    /// Execute the workflow synchronously starting from `root_step_id`,
    /// following the first `next` edge of each visited step until a
    /// terminal node. The execution model deliberately matches
    /// upstream's "first-edge" walk used by the visualizer — a more
    /// elaborate edge-condition engine arrives with the JS isolate.
    pub fn execute(&mut self, run_id: &str, workflow_id: &str) -> Result<WorkflowRunStatus, String> {
        let workflow = self
            .workflows
            .get(workflow_id)
            .ok_or_else(|| format!("workflow '{}' not found", workflow_id))?;
        if !workflow.is_active {
            return Err(format!("workflow '{}' is inactive", workflow_id));
        }
        let mut trace = Vec::new();
        let mut cursor = workflow.root_step_id.clone();
        let by_id: HashMap<&str, &WorkflowStep> = workflow
            .steps
            .iter()
            .map(|s| (s.id.as_str(), s))
            .collect();
        loop {
            let step = by_id
                .get(cursor.as_str())
                .ok_or_else(|| format!("step '{}' missing in workflow", cursor))?;
            trace.push(step.id.clone());
            match step.next.first() {
                Some(next) => cursor = next.clone(),
                None => break,
            }
        }
        let run = WorkflowRun {
            id: run_id.to_string(),
            workflow_id: workflow_id.to_string(),
            status: WorkflowRunStatus::Completed,
            trace,
            error: None,
        };
        self.runs.insert(run_id.to_string(), run);
        Ok(WorkflowRunStatus::Completed)
    }

    pub fn run(&self, run_id: &str) -> Option<&WorkflowRun> {
        self.runs.get(run_id)
    }
}

/// Validate the DAG: every `next` id must resolve, the root must exist,
/// no cycles. Mirrors upstream `WorkflowGraphValidator`.
fn validate_dag(w: &Workflow) -> Result<(), String> {
    let by_id: HashMap<&str, &WorkflowStep> = w.steps.iter().map(|s| (s.id.as_str(), s)).collect();
    if !by_id.contains_key(w.root_step_id.as_str()) {
        return Err(format!("root_step_id '{}' not in steps", w.root_step_id));
    }
    for s in &w.steps {
        for n in &s.next {
            if !by_id.contains_key(n.as_str()) {
                return Err(format!(
                    "step '{}' has dangling next '{}'",
                    s.id, n
                ));
            }
        }
    }
    let mut visiting: HashSet<&str> = HashSet::new();
    let mut visited: HashSet<&str> = HashSet::new();
    fn dfs<'a>(
        node: &'a str,
        by_id: &HashMap<&'a str, &'a WorkflowStep>,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
    ) -> Result<(), String> {
        if visited.contains(node) {
            return Ok(());
        }
        if !visiting.insert(node) {
            return Err(format!("cycle detected at step '{}'", node));
        }
        let step = by_id.get(node).copied().unwrap();
        for n in &step.next {
            dfs(n.as_str(), by_id, visiting, visited)?;
        }
        visiting.remove(node);
        visited.insert(node);
        Ok(())
    }
    dfs(w.root_step_id.as_str(), &by_id, &mut visiting, &mut visited)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(id: &str, kind: WorkflowStepType, next: &[&str]) -> WorkflowStep {
        WorkflowStep {
            id: id.into(),
            name: id.into(),
            kind,
            settings: json!({}),
            next: next.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn workflow(id: &str, steps: Vec<WorkflowStep>, root: &str) -> Workflow {
        Workflow {
            id: id.into(),
            workspace_id: "ws-1".into(),
            name: "demo".into(),
            version: 1,
            is_active: true,
            root_step_id: root.into(),
            steps,
        }
    }

    #[test]
    fn linear_workflow_executes_in_order() {
        let mut store = WorkflowStore::default();
        let w = workflow(
            "wf-1",
            vec![
                step("a", WorkflowStepType::Trigger, &["b"]),
                step("b", WorkflowStepType::Record, &["c"]),
                step("c", WorkflowStepType::HttpRequest, &[]),
            ],
            "a",
        );
        store.put_workflow(w).unwrap();
        let status = store.execute("run-1", "wf-1").unwrap();
        assert_eq!(status, WorkflowRunStatus::Completed);
        let run = store.run("run-1").unwrap();
        assert_eq!(run.trace, vec!["a".to_string(), "b".into(), "c".into()]);
    }

    #[test]
    fn workflow_with_cycle_is_rejected() {
        let mut store = WorkflowStore::default();
        let w = workflow(
            "wf-bad",
            vec![
                step("a", WorkflowStepType::Trigger, &["b"]),
                step("b", WorkflowStepType::Record, &["a"]),
            ],
            "a",
        );
        let err = store.put_workflow(w).unwrap_err();
        assert!(err.contains("cycle"));
    }

    #[test]
    fn workflow_with_dangling_next_is_rejected() {
        let mut store = WorkflowStore::default();
        let w = workflow(
            "wf-bad",
            vec![step("a", WorkflowStepType::Trigger, &["nowhere"])],
            "a",
        );
        let err = store.put_workflow(w).unwrap_err();
        assert!(err.contains("dangling"));
    }

    #[test]
    fn workflow_with_missing_root_is_rejected() {
        let mut store = WorkflowStore::default();
        let w = workflow(
            "wf-bad",
            vec![step("a", WorkflowStepType::Trigger, &[])],
            "missing-root",
        );
        let err = store.put_workflow(w).unwrap_err();
        assert!(err.contains("root_step_id"));
    }

    #[test]
    fn inactive_workflow_does_not_execute() {
        let mut store = WorkflowStore::default();
        let mut w = workflow(
            "wf-1",
            vec![step("a", WorkflowStepType::Trigger, &[])],
            "a",
        );
        w.is_active = false;
        store.put_workflow(w).unwrap();
        let err = store.execute("run-1", "wf-1").unwrap_err();
        assert!(err.contains("inactive"));
    }

    #[test]
    fn list_workflows_returns_only_workspace_scoped() {
        let mut store = WorkflowStore::default();
        store
            .put_workflow(workflow(
                "wf-1",
                vec![step("a", WorkflowStepType::Trigger, &[])],
                "a",
            ))
            .unwrap();
        let mut w2 = workflow(
            "wf-2",
            vec![step("a", WorkflowStepType::Trigger, &[])],
            "a",
        );
        w2.workspace_id = "ws-2".into();
        store.put_workflow(w2).unwrap();
        let in_ws1 = store.list_workflows("ws-1");
        assert_eq!(in_ws1.len(), 1);
        assert_eq!(in_ws1[0].id, "wf-1");
    }

    #[test]
    fn delete_workflow_returns_whether_existed() {
        let mut store = WorkflowStore::default();
        store
            .put_workflow(workflow(
                "wf-1",
                vec![step("a", WorkflowStepType::Trigger, &[])],
                "a",
            ))
            .unwrap();
        assert!(store.delete_workflow("wf-1"));
        assert!(!store.delete_workflow("wf-1"));
    }

    #[test]
    fn execute_missing_workflow_errors() {
        let mut store = WorkflowStore::default();
        let err = store.execute("run", "nope").unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn workflow_step_serde_round_trip() {
        let s = step("a", WorkflowStepType::HttpRequest, &["b"]);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("HTTP_REQUEST"));
        let back: WorkflowStep = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, WorkflowStepType::HttpRequest);
    }
}
