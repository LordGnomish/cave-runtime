// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: pulp/pulpcore@0f991c2fa2bf6c8635e8a2de064ef04dacbbcf4f pulpcore/app/tasks/repository.py + pulpcore/app/models/task.py
//! Async task queue — Pulp v3 task system.
//!
//! All long-running operations (sync, publish, repair) return a task href
//! immediately; callers poll or wait for completion.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Waiting,
    Running,
    Completed,
    Failed,
    Canceled,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub state: TaskState,
    pub name: String,
    pub logging_cid: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<TaskError>,
    pub worker: Option<String>,
    pub parent_task: Option<String>,
    pub child_tasks: Vec<String>,
    pub task_group: Option<String>,
    pub progress_reports: Vec<ProgressReport>,
    pub created_resources: Vec<String>,
    pub reserved_resources_record: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskError {
    pub reason: String,
    pub code: String,
    pub description: String,
    pub traceback: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressReport {
    pub message: String,
    pub code: String,
    pub total: Option<u64>,
    pub done: u64,
    pub state: String,
    pub suffix: Option<String>,
}

impl Task {
    pub fn new(name: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/tasks/{}/", id),
            pulp_id: id,
            pulp_created: Utc::now(),
            state: TaskState::Waiting,
            name: name.into(),
            logging_cid: format!("{}", Uuid::new_v4()),
            started_at: None,
            finished_at: None,
            error: None,
            worker: None,
            parent_task: None,
            child_tasks: vec![],
            task_group: None,
            progress_reports: vec![],
            created_resources: vec![],
            reserved_resources_record: vec![],
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            TaskState::Completed | TaskState::Failed | TaskState::Canceled | TaskState::Skipped
        )
    }

    pub fn mark_running(&mut self) {
        self.state = TaskState::Running;
        self.started_at = Some(Utc::now());
    }

    pub fn mark_completed(&mut self, created_resources: Vec<String>) {
        self.state = TaskState::Completed;
        self.finished_at = Some(Utc::now());
        self.created_resources = created_resources;
    }

    pub fn mark_failed(&mut self, reason: impl Into<String>) {
        self.state = TaskState::Failed;
        self.finished_at = Some(Utc::now());
        self.error = Some(TaskError {
            reason: reason.into(),
            code: "ERR001".to_string(),
            description: "Task execution failed".to_string(),
            traceback: None,
        });
    }

    pub fn mark_canceled(&mut self) {
        self.state = TaskState::Canceled;
        self.finished_at = Some(Utc::now());
    }

    pub fn add_progress(&mut self, message: impl Into<String>, done: u64, total: Option<u64>) {
        self.progress_reports.push(ProgressReport {
            message: message.into(),
            code: "sync".to_string(),
            total,
            done,
            state: "running".to_string(),
            suffix: None,
        });
    }

    pub fn elapsed_seconds(&self) -> Option<f64> {
        let started = self.started_at?;
        let finished = self.finished_at.unwrap_or_else(Utc::now);
        Some((finished - started).num_milliseconds() as f64 / 1000.0)
    }
}

// ─── Task queue ──────────────────────────────────────────────────────────────

pub struct TaskQueue {
    tasks: Mutex<HashMap<Uuid, Task>>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// Enqueue a new task and return its initial state.
    pub fn enqueue(&self, name: impl Into<String>) -> Task {
        let task = Task::new(name);
        let mut tasks = self.tasks.lock().unwrap();
        tasks.insert(task.pulp_id, task.clone());
        task
    }

    pub fn get(&self, id: &Uuid) -> Option<Task> {
        let tasks = self.tasks.lock().unwrap();
        tasks.get(id).cloned()
    }

    pub fn update(&self, task: Task) {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.insert(task.pulp_id, task);
    }

    pub fn cancel(&self, id: &Uuid) -> bool {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.get_mut(id) {
            if !task.is_terminal() {
                task.mark_canceled();
                return true;
            }
        }
        false
    }

    pub fn list(&self) -> Vec<Task> {
        let tasks = self.tasks.lock().unwrap();
        tasks.values().cloned().collect()
    }

    pub fn list_by_state(&self, state: &TaskState) -> Vec<Task> {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .values()
            .filter(|t| &t.state == state)
            .cloned()
            .collect()
    }

    pub fn purge_completed(&self) -> usize {
        let mut tasks = self.tasks.lock().unwrap();
        let before = tasks.len();
        tasks.retain(|_, t| !t.is_terminal());
        before - tasks.len()
    }
}

// ─── TaskGroup ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGroup {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub description: String,
    pub all_tasks_dispatched: bool,
    pub waiting: u64,
    pub skipped: u64,
    pub running: u64,
    pub completed: u64,
    pub canceled: u64,
    pub failed: u64,
    pub tasks: Vec<String>,
}

impl TaskGroup {
    pub fn new(description: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/task-groups/{}/", id),
            pulp_id: id,
            description: description.into(),
            all_tasks_dispatched: false,
            waiting: 0,
            skipped: 0,
            running: 0,
            completed: 0,
            canceled: 0,
            failed: 0,
            tasks: vec![],
        }
    }

    pub fn is_complete(&self) -> bool {
        self.all_tasks_dispatched && self.waiting == 0 && self.running == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_lifecycle() {
        let mut task = Task::new("pulp.tasks.synchronize");
        assert_eq!(task.state, TaskState::Waiting);
        assert!(!task.is_terminal());

        task.mark_running();
        assert_eq!(task.state, TaskState::Running);

        task.mark_completed(vec![
            "/pulp/api/v3/repositories/abc/versions/1/".to_string(),
        ]);
        assert_eq!(task.state, TaskState::Completed);
        assert!(task.is_terminal());
        assert!(!task.created_resources.is_empty());
    }

    #[test]
    fn task_failure() {
        let mut task = Task::new("pulp.tasks.synchronize");
        task.mark_running();
        task.mark_failed("Connection refused");
        assert_eq!(task.state, TaskState::Failed);
        assert!(task.error.is_some());
        assert!(task.finished_at.is_some());
    }

    #[test]
    fn task_cancellation() {
        let mut task = Task::new("pulp.tasks.repair");
        task.mark_running();
        task.mark_canceled();
        assert_eq!(task.state, TaskState::Canceled);
    }

    #[test]
    fn task_progress_reports() {
        let mut task = Task::new("pulp.tasks.synchronize");
        task.mark_running();
        task.add_progress("Downloading packages", 50, Some(100));
        task.add_progress("Saving packages", 100, Some(100));
        assert_eq!(task.progress_reports.len(), 2);
    }

    #[test]
    fn task_queue_enqueue_and_get() {
        let queue = TaskQueue::new();
        let task = queue.enqueue("pulp.tasks.synchronize");
        let retrieved = queue.get(&task.pulp_id).unwrap();
        assert_eq!(retrieved.state, TaskState::Waiting);
    }

    #[test]
    fn task_queue_cancel() {
        let queue = TaskQueue::new();
        let task = queue.enqueue("pulp.tasks.repair");
        let mut running = task.clone();
        running.mark_running();
        queue.update(running);
        assert!(queue.cancel(&task.pulp_id));
        let canceled = queue.get(&task.pulp_id).unwrap();
        assert_eq!(canceled.state, TaskState::Canceled);
    }

    #[test]
    fn task_queue_cancel_already_complete() {
        let queue = TaskQueue::new();
        let task = queue.enqueue("task");
        let mut t = task.clone();
        t.mark_running();
        t.mark_completed(vec![]);
        queue.update(t);
        assert!(!queue.cancel(&task.pulp_id));
    }

    #[test]
    fn task_queue_list_by_state() {
        let queue = TaskQueue::new();
        let t1 = queue.enqueue("task-1");
        let t2 = queue.enqueue("task-2");
        let mut running = t1.clone();
        running.mark_running();
        queue.update(running);

        let waiting = queue.list_by_state(&TaskState::Waiting);
        let running_tasks = queue.list_by_state(&TaskState::Running);
        assert_eq!(waiting.len(), 1);
        assert_eq!(running_tasks.len(), 1);
    }

    #[test]
    fn task_queue_purge_completed() {
        let queue = TaskQueue::new();
        let t1 = queue.enqueue("t1");
        let t2 = queue.enqueue("t2");
        let mut done = t1.clone();
        done.mark_running();
        done.mark_completed(vec![]);
        queue.update(done);
        let purged = queue.purge_completed();
        assert_eq!(purged, 1);
    }

    #[test]
    fn task_group_complete_when_all_dispatched_and_done() {
        let mut group = TaskGroup::new("sync all repos");
        group.all_tasks_dispatched = true;
        group.waiting = 0;
        group.running = 0;
        group.completed = 3;
        assert!(group.is_complete());
    }

    #[test]
    fn task_group_not_complete_while_running() {
        let mut group = TaskGroup::new("sync all repos");
        group.all_tasks_dispatched = true;
        group.running = 1;
        assert!(!group.is_complete());
    }
}
