// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Priority task queue.
//!
//! Turns the ranked [`Subsystem`](crate::tracker::Subsystem) list into
//! deduplicated [`Task`]s and tracks each task's lifecycle. The daemon pulls
//! one task at a time, runs it through the escalation ladder, and records the
//! outcome back here so metrics and the daily report can read it.

use crate::tracker::Subsystem;
use std::collections::BTreeMap;

/// Lifecycle of a single dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// Waiting in the queue.
    Pending,
    /// Currently being worked by the daemon.
    InProgress,
    /// Tests passed, committed + merged locally.
    Completed,
    /// Exhausted local retries *and* Claude escalation without a green test.
    Failed,
    /// Bumped to L4 — needs a human (architectural/strategic) decision.
    EscalatedToHuman,
}

/// One unit of autonomous work: bring a single crate closer to parity.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    /// Stable id derived from the subsystem name, e.g. `port-cave-etcd`.
    pub id: String,
    pub subsystem: String,
    pub crate_dir: String,
    pub upstream: Option<String>,
    /// Completion at the time the task was enqueued.
    pub completion: f64,
    /// 0 = highest priority (most incomplete).
    pub priority: usize,
    /// Number of local-LLM attempts spent so far.
    pub attempts: u32,
    pub status: TaskStatus,
}

impl Task {
    fn from_subsystem(s: &Subsystem, priority: usize) -> Self {
        Self {
            id: format!("port-{}", s.name),
            subsystem: s.name.clone(),
            crate_dir: s.crate_dir.clone(),
            upstream: s.upstream.clone(),
            completion: s.completion,
            priority,
            attempts: 0,
            status: TaskStatus::Pending,
        }
    }
}

/// FIFO-by-priority queue keyed on task id for O(log n) dedup + status updates.
#[derive(Debug, Default)]
pub struct TaskQueue {
    tasks: BTreeMap<String, Task>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a queue from ranked subsystems. Priority follows input order
    /// (caller is expected to pass [`TrackerState::ranked_incomplete`] output).
    /// Re-enqueuing an id that already exists is a no-op so a tick that re-reads
    /// the tracker never resets an in-flight task's attempt count.
    ///
    /// [`TrackerState::ranked_incomplete`]: crate::tracker::TrackerState::ranked_incomplete
    pub fn enqueue_ranked(&mut self, ranked: &[Subsystem]) {
        for (i, s) in ranked.iter().enumerate() {
            let id = format!("port-{}", s.name);
            self.tasks
                .entry(id)
                .or_insert_with(|| Task::from_subsystem(s, i));
        }
    }

    /// Highest-priority `Pending` task's id, if any. Does **not** mutate state;
    /// call [`Self::start`] to claim it.
    pub fn peek_next_id(&self) -> Option<String> {
        self.tasks
            .values()
            .filter(|t| t.status == TaskStatus::Pending)
            .min_by_key(|t| t.priority)
            .map(|t| t.id.clone())
    }

    /// Claim the highest-priority pending task, marking it `InProgress`.
    pub fn start_next(&mut self) -> Option<Task> {
        let id = self.peek_next_id()?;
        let t = self.tasks.get_mut(&id)?;
        t.status = TaskStatus::InProgress;
        Some(t.clone())
    }

    /// Record one consumed local attempt against a task.
    pub fn record_attempt(&mut self, id: &str) {
        if let Some(t) = self.tasks.get_mut(id) {
            t.attempts += 1;
        }
    }

    /// Set a task's terminal (or escalated) status.
    pub fn set_status(&mut self, id: &str, status: TaskStatus) {
        if let Some(t) = self.tasks.get_mut(id) {
            t.status = status;
        }
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// Number of tasks still `Pending`.
    pub fn pending_depth(&self) -> usize {
        self.count(TaskStatus::Pending)
    }

    pub fn count(&self, status: TaskStatus) -> usize {
        self.tasks.values().filter(|t| t.status == status).count()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// All tasks, for reporting.
    pub fn all(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(name: &str, completion: f64) -> Subsystem {
        Subsystem {
            name: name.to_string(),
            crate_dir: format!("crates/{name}"),
            completion,
            src_loc: 100,
            tier: "C1".into(),
            upstream: Some("owner/repo".into()),
        }
    }

    #[test]
    fn enqueue_assigns_priority_in_order_and_dedups() {
        let mut q = TaskQueue::new();
        let ranked = vec![sub("cave-etcd", 0.5), sub("cave-policy", 0.65)];
        q.enqueue_ranked(&ranked);
        q.enqueue_ranked(&ranked); // second pass must not duplicate
        assert_eq!(q.len(), 2);
        assert_eq!(q.get("port-cave-etcd").unwrap().priority, 0);
        assert_eq!(q.get("port-cave-policy").unwrap().priority, 1);
    }

    #[test]
    fn start_next_returns_highest_priority_and_marks_in_progress() {
        let mut q = TaskQueue::new();
        q.enqueue_ranked(&[sub("cave-etcd", 0.5), sub("cave-policy", 0.65)]);
        let t = q.start_next().unwrap();
        assert_eq!(t.subsystem, "cave-etcd");
        assert_eq!(q.get("port-cave-etcd").unwrap().status, TaskStatus::InProgress);
        // Now the next pending is policy.
        assert_eq!(q.peek_next_id().as_deref(), Some("port-cave-policy"));
    }

    #[test]
    fn enqueue_does_not_reset_in_flight_attempts() {
        let mut q = TaskQueue::new();
        let ranked = vec![sub("cave-etcd", 0.5)];
        q.enqueue_ranked(&ranked);
        q.start_next();
        q.record_attempt("port-cave-etcd");
        q.record_attempt("port-cave-etcd");
        q.enqueue_ranked(&ranked); // re-read tracker mid-flight
        assert_eq!(q.get("port-cave-etcd").unwrap().attempts, 2);
        assert_eq!(q.get("port-cave-etcd").unwrap().status, TaskStatus::InProgress);
    }

    #[test]
    fn status_counts_track_lifecycle() {
        let mut q = TaskQueue::new();
        q.enqueue_ranked(&[sub("a", 0.1), sub("b", 0.2), sub("c", 0.3)]);
        assert_eq!(q.pending_depth(), 3);
        q.start_next();
        q.set_status("port-a", TaskStatus::Completed);
        q.set_status("port-b", TaskStatus::Failed);
        assert_eq!(q.count(TaskStatus::Completed), 1);
        assert_eq!(q.count(TaskStatus::Failed), 1);
        assert_eq!(q.pending_depth(), 1);
    }
}
