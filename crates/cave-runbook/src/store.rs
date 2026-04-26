//! In-memory store for cave-runbook.

use crate::models::{Execution, ExecutionStatus, Runbook, StepExecution};
use chrono::Utc;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub struct RunbookStore {
    runbooks: Arc<Mutex<Vec<Runbook>>>,
    executions: Arc<Mutex<Vec<Execution>>>,
}

impl RunbookStore {
    pub fn new() -> Self {
        Self {
            runbooks: Arc::new(Mutex::new(vec![])),
            executions: Arc::new(Mutex::new(vec![])),
        }
    }

    // ─── Runbook CRUD ─────────────────────────────────────────────────────────

    pub fn list_runbooks(&self) -> Vec<Runbook> {
        self.runbooks.lock().unwrap().clone()
    }

    pub fn get_runbook(&self, id: Uuid) -> Option<Runbook> {
        self.runbooks
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    pub fn create_runbook(&self, runbook: Runbook) -> Runbook {
        let mut store = self.runbooks.lock().unwrap();
        store.push(runbook.clone());
        runbook
    }

    pub fn update_runbook(&self, id: Uuid, updated: Runbook) -> Option<Runbook> {
        let mut store = self.runbooks.lock().unwrap();
        if let Some(r) = store.iter_mut().find(|r| r.id == id) {
            *r = updated.clone();
            Some(updated)
        } else {
            None
        }
    }

    pub fn delete_runbook(&self, id: Uuid) -> bool {
        let mut store = self.runbooks.lock().unwrap();
        let len_before = store.len();
        store.retain(|r| r.id != id);
        store.len() < len_before
    }

    // ─── Execution CRUD ───────────────────────────────────────────────────────

    pub fn list_executions(&self, runbook_id: Option<Uuid>) -> Vec<Execution> {
        let store = self.executions.lock().unwrap();
        match runbook_id {
            Some(id) => store.iter().filter(|e| e.runbook_id == id).cloned().collect(),
            None => store.clone(),
        }
    }

    pub fn get_execution(&self, id: Uuid) -> Option<Execution> {
        self.executions
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id)
            .cloned()
    }

    pub fn add_execution(&self, execution: Execution) -> Execution {
        let mut store = self.executions.lock().unwrap();
        store.push(execution.clone());
        execution
    }

    pub fn update_execution_step(
        &self,
        exec_id: Uuid,
        step_id: Uuid,
        step_exec: StepExecution,
    ) -> bool {
        let mut store = self.executions.lock().unwrap();
        if let Some(exec) = store.iter_mut().find(|e| e.id == exec_id) {
            if let Some(se) = exec.step_executions.iter_mut().find(|s| s.step_id == step_id) {
                *se = step_exec;
            } else {
                exec.step_executions.push(step_exec);
            }
            true
        } else {
            false
        }
    }

    /// Concatenate all stdout/stderr from step executions.
    pub fn get_execution_output(&self, id: Uuid) -> Option<String> {
        let store = self.executions.lock().unwrap();
        store.iter().find(|e| e.id == id).map(|exec| {
            exec.step_executions
                .iter()
                .map(|se| {
                    format!(
                        "=== Step: {} ===\nSTDOUT:\n{}\nSTDERR:\n{}\n",
                        se.step_name, se.stdout, se.stderr
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub fn cancel_execution(&self, id: Uuid) -> bool {
        let mut store = self.executions.lock().unwrap();
        if let Some(exec) = store.iter_mut().find(|e| e.id == id) {
            if exec.status == ExecutionStatus::Running
                || exec.status == ExecutionStatus::Pending
                || exec.status == ExecutionStatus::WaitingApproval
            {
                exec.status = ExecutionStatus::Cancelled;
                exec.completed_at = Some(Utc::now());
                return true;
            }
        }
        false
    }
}

impl Default for RunbookStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ExecutionStatus, RunbookNotifications};
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_runbook(name: &str) -> Runbook {
        Runbook {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: "desc".to_string(),
            steps: vec![],
            parameters: vec![],
            schedule: None,
            access_control: vec![],
            notifications: RunbookNotifications::default(),
            timeout_seconds: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: Uuid::new_v4(),
            enabled: true,
        }
    }

    fn make_execution(runbook_id: Uuid, runbook_name: &str) -> Execution {
        Execution {
            id: Uuid::new_v4(),
            runbook_id,
            runbook_name: runbook_name.to_string(),
            status: ExecutionStatus::Running,
            triggered_by: Uuid::new_v4(),
            parameters: HashMap::new(),
            step_executions: vec![],
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        }
    }

    #[test]
    fn test_create_and_get_runbook() {
        let store = RunbookStore::new();
        let rb = make_runbook("deploy-prod");
        let id = rb.id;
        store.create_runbook(rb.clone());
        let found = store.get_runbook(id).unwrap();
        assert_eq!(found.name, "deploy-prod");
    }

    #[test]
    fn test_list_runbooks() {
        let store = RunbookStore::new();
        store.create_runbook(make_runbook("rb1"));
        store.create_runbook(make_runbook("rb2"));
        assert_eq!(store.list_runbooks().len(), 2);
    }

    #[test]
    fn test_update_runbook() {
        let store = RunbookStore::new();
        let rb = make_runbook("original");
        let id = rb.id;
        store.create_runbook(rb.clone());
        let mut updated = rb.clone();
        updated.name = "updated".to_string();
        store.update_runbook(id, updated);
        assert_eq!(store.get_runbook(id).unwrap().name, "updated");
    }

    #[test]
    fn test_delete_runbook() {
        let store = RunbookStore::new();
        let rb = make_runbook("to-delete");
        let id = rb.id;
        store.create_runbook(rb);
        assert!(store.delete_runbook(id));
        assert!(store.get_runbook(id).is_none());
    }

    #[test]
    fn test_add_and_get_execution() {
        let store = RunbookStore::new();
        let rb = make_runbook("rb");
        let exec = make_execution(rb.id, &rb.name);
        let exec_id = exec.id;
        store.add_execution(exec);
        assert!(store.get_execution(exec_id).is_some());
    }

    #[test]
    fn test_list_executions_filtered_by_runbook() {
        let store = RunbookStore::new();
        let rb1 = make_runbook("rb1");
        let rb2 = make_runbook("rb2");
        store.add_execution(make_execution(rb1.id, "rb1"));
        store.add_execution(make_execution(rb1.id, "rb1"));
        store.add_execution(make_execution(rb2.id, "rb2"));
        let rb1_execs = store.list_executions(Some(rb1.id));
        assert_eq!(rb1_execs.len(), 2);
        let rb2_execs = store.list_executions(Some(rb2.id));
        assert_eq!(rb2_execs.len(), 1);
    }

    #[test]
    fn test_cancel_execution() {
        let store = RunbookStore::new();
        let exec = make_execution(Uuid::new_v4(), "rb");
        let exec_id = exec.id;
        store.add_execution(exec);
        assert!(store.cancel_execution(exec_id));
        let updated = store.get_execution(exec_id).unwrap();
        assert_eq!(updated.status, ExecutionStatus::Cancelled);
    }

    #[test]
    fn test_get_execution_output() {
        let store = RunbookStore::new();
        let mut exec = make_execution(Uuid::new_v4(), "rb");
        exec.step_executions.push(StepExecution {
            step_id: Uuid::new_v4(),
            step_name: "step1".to_string(),
            status: ExecutionStatus::Completed,
            started_at: Some(Utc::now()),
            completed_at: Some(Utc::now()),
            exit_code: Some(0),
            stdout: "hello world".to_string(),
            stderr: String::new(),
            error: None,
            retries: 0,
        });
        let exec_id = exec.id;
        store.add_execution(exec);
        let output = store.get_execution_output(exec_id).unwrap();
        assert!(output.contains("hello world"));
        assert!(output.contains("step1"));
    }

    #[test]
    fn test_multiple_executions_per_runbook() {
        let store = RunbookStore::new();
        let rb = make_runbook("multi-rb");
        for _ in 0..5 {
            store.add_execution(make_execution(rb.id, &rb.name));
        }
        let execs = store.list_executions(Some(rb.id));
        assert_eq!(execs.len(), 5);
    }
}
