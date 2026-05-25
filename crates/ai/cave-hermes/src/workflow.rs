// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Workflow checkpoint / resume.
//!
//! Ports the recovery half of Hermes' `agent/retry_utils.py` and the
//! checkpoint pattern that `agent/run_agent.py` uses to survive provider
//! errors. The MVP layout: a [`Workflow`] is an ordered list of named
//! steps; advancing the workflow drops a [`Checkpoint`] every time a step
//! transitions to `Done`. If the process dies, [`Workflow::resume_from`]
//! replays the journal and seeks to the last incomplete step.
//!
//! No tasks run in this module — the planner produces step lists and the
//! tool registry runs them. This module only owns the state machine.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::HermesError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowStatus {
    Pending,
    Running,
    Done,
    Failed(String),
    Stuck(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub name: String,
    pub status: WorkflowStatus,
    /// Optional structured args used by the tool registry. Opaque here.
    #[serde(default)]
    pub args: BTreeMap<String, String>,
    /// Optional output from a completed step. Free-form text.
    #[serde(default)]
    pub output: Option<String>,
    /// Retries already attempted. Capped by `Workflow::max_retries`.
    #[serde(default)]
    pub attempts: u32,
}

impl Step {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: WorkflowStatus::Pending,
            args: BTreeMap::new(),
            output: None,
            attempts: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub steps: Vec<Step>,
    /// Monotonically incremented per checkpoint commit.
    pub revision: u64,
    pub max_retries: u32,
}

impl Workflow {
    pub fn new(id: impl Into<String>, steps: Vec<Step>) -> Self {
        Self {
            id: id.into(),
            steps,
            revision: 0,
            max_retries: 3,
        }
    }

    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// Index of the first non-`Done` step (or `None` if everything is done).
    pub fn next_pending(&self) -> Option<usize> {
        self.steps
            .iter()
            .position(|s| !matches!(s.status, WorkflowStatus::Done))
    }

    pub fn is_complete(&self) -> bool {
        self.next_pending().is_none()
            && self
                .steps
                .iter()
                .all(|s| matches!(s.status, WorkflowStatus::Done))
    }

    /// Mark the current step as running. Returns the step name advanced to.
    pub fn start_step(&mut self) -> crate::error::Result<&str> {
        let Some(idx) = self.next_pending() else {
            return Err(HermesError::CheckpointMissing(format!(
                "workflow '{}' has no pending step",
                self.id
            )));
        };
        self.steps[idx].status = WorkflowStatus::Running;
        self.steps[idx].attempts += 1;
        Ok(&self.steps[idx].name)
    }

    /// Mark the current running step as `Done` with optional output.
    pub fn finish_step(&mut self, output: Option<String>) -> crate::error::Result<()> {
        let idx = self
            .steps
            .iter()
            .position(|s| matches!(s.status, WorkflowStatus::Running))
            .ok_or_else(|| HermesError::CheckpointMissing("no running step".into()))?;
        self.steps[idx].status = WorkflowStatus::Done;
        self.steps[idx].output = output;
        self.revision += 1;
        Ok(())
    }

    /// Mark the current running step as `Failed`. Promotes to `Stuck`
    /// once `attempts > max_retries`, matching Hermes' retry semantics
    /// where exhausted retries flip the step into a recoverable-but-
    /// not-auto-retryable state.
    pub fn fail_step(&mut self, reason: impl Into<String>) -> crate::error::Result<()> {
        let reason = reason.into();
        let idx = self
            .steps
            .iter()
            .position(|s| matches!(s.status, WorkflowStatus::Running))
            .ok_or_else(|| HermesError::CheckpointMissing("no running step".into()))?;
        let attempts = self.steps[idx].attempts;
        if attempts > self.max_retries {
            self.steps[idx].status = WorkflowStatus::Stuck(reason);
        } else {
            // Reset to Pending so a follow-up `start_step` retries it.
            self.steps[idx].status = WorkflowStatus::Pending;
            self.steps[idx].output = Some(format!("attempt {attempts} failed: {reason}"));
        }
        self.revision += 1;
        Ok(())
    }

    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            workflow_id: self.id.clone(),
            revision: self.revision,
            data: self.clone(),
        }
    }

    /// Replace this workflow's state with the contents of a checkpoint.
    /// Used by `resume_from`.
    pub fn restore(&mut self, ck: Checkpoint) -> crate::error::Result<()> {
        if ck.workflow_id != self.id {
            return Err(HermesError::CheckpointMissing(format!(
                "checkpoint id mismatch: ours '{}', theirs '{}'",
                self.id, ck.workflow_id
            )));
        }
        *self = ck.data;
        Ok(())
    }

    /// Recover from a stuck step by resetting it back to Pending and
    /// dropping the attempt counter. Caller must understand this risks
    /// thrashing — Hermes surfaces it as a user-confirmation prompt.
    pub fn unstick(&mut self) -> crate::error::Result<()> {
        let Some(idx) = self
            .steps
            .iter()
            .position(|s| matches!(s.status, WorkflowStatus::Stuck(_)))
        else {
            return Err(HermesError::CheckpointMissing(format!(
                "workflow '{}' has no stuck step",
                self.id
            )));
        };
        self.steps[idx].status = WorkflowStatus::Pending;
        self.steps[idx].attempts = 0;
        self.revision += 1;
        Ok(())
    }
}

/// Persisted snapshot. Each call to [`save`] writes a JSON blob keyed by
/// the workflow id; [`load`] reads it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub workflow_id: String,
    pub revision: u64,
    pub data: Workflow,
}

impl Checkpoint {
    pub fn save(&self, dir: impl AsRef<Path>) -> crate::error::Result<PathBuf> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.json", self.workflow_id));
        let body = serde_json::to_string_pretty(self)?;
        fs::write(&path, body)?;
        Ok(path)
    }

    pub fn load(dir: impl AsRef<Path>, id: &str) -> crate::error::Result<Self> {
        let path = dir.as_ref().join(format!("{id}.json"));
        if !path.exists() {
            return Err(HermesError::CheckpointMissing(id.to_string()));
        }
        let raw = fs::read_to_string(&path)?;
        let ck: Checkpoint = serde_json::from_str(&raw)?;
        Ok(ck)
    }
}

/// Re-execute a journal of saved checkpoints to resume a workflow.
/// Returns the rehydrated [`Workflow`] with `next_pending()` pointing to
/// the first step that did not complete.
pub fn resume_from(dir: impl AsRef<Path>, id: &str) -> crate::error::Result<Workflow> {
    let ck = Checkpoint::load(dir, id)?;
    Ok(ck.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn three_step() -> Workflow {
        Workflow::new(
            "wf-1",
            vec![Step::new("plan"), Step::new("fetch"), Step::new("respond")],
        )
    }

    #[test]
    fn fresh_workflow_starts_at_step_zero() {
        let wf = three_step();
        assert_eq!(wf.next_pending(), Some(0));
        assert!(!wf.is_complete());
    }

    #[test]
    fn start_and_finish_advances_pointer() {
        let mut wf = three_step();
        assert_eq!(wf.start_step().unwrap(), "plan");
        wf.finish_step(Some("planned".into())).unwrap();
        assert_eq!(wf.next_pending(), Some(1));
        assert_eq!(wf.steps[0].status, WorkflowStatus::Done);
        assert_eq!(wf.steps[0].output.as_deref(), Some("planned"));
        assert_eq!(wf.revision, 1);
    }

    #[test]
    fn finish_without_running_step_errors() {
        let mut wf = three_step();
        let err = wf.finish_step(None).unwrap_err();
        assert!(matches!(err, HermesError::CheckpointMissing(_)));
    }

    #[test]
    fn fail_step_retries_until_max_then_sticks() {
        let mut wf = three_step().with_max_retries(2);
        for _ in 0..2 {
            wf.start_step().unwrap();
            wf.fail_step("network").unwrap();
            // Failed attempts reset to Pending until max exhausted.
            assert_eq!(wf.next_pending(), Some(0));
        }
        // 3rd start → attempts becomes 3, > max(2), so fail_step → Stuck.
        wf.start_step().unwrap();
        wf.fail_step("network").unwrap();
        assert!(matches!(wf.steps[0].status, WorkflowStatus::Stuck(_)));
    }

    #[test]
    fn unstick_clears_stuck_state() {
        let mut wf = three_step().with_max_retries(0);
        wf.start_step().unwrap();
        wf.fail_step("boom").unwrap();
        assert!(matches!(wf.steps[0].status, WorkflowStatus::Stuck(_)));
        wf.unstick().unwrap();
        assert_eq!(wf.steps[0].status, WorkflowStatus::Pending);
        assert_eq!(wf.steps[0].attempts, 0);
    }

    #[test]
    fn unstick_on_clean_workflow_errors() {
        let mut wf = three_step();
        let err = wf.unstick().unwrap_err();
        assert!(matches!(err, HermesError::CheckpointMissing(_)));
    }

    #[test]
    fn checkpoint_save_load_roundtrip() {
        let dir = tempdir().unwrap();
        let mut wf = three_step();
        wf.start_step().unwrap();
        wf.finish_step(Some("ok".into())).unwrap();
        wf.checkpoint().save(dir.path()).unwrap();
        let loaded = resume_from(dir.path(), "wf-1").unwrap();
        assert_eq!(loaded.id, wf.id);
        assert_eq!(loaded.revision, wf.revision);
        assert_eq!(loaded.next_pending(), Some(1));
    }

    #[test]
    fn load_missing_returns_checkpoint_missing() {
        let dir = tempdir().unwrap();
        let err = Checkpoint::load(dir.path(), "nope").unwrap_err();
        assert!(matches!(err, HermesError::CheckpointMissing(_)));
    }

    #[test]
    fn restore_mismatched_id_errors() {
        let dir = tempdir().unwrap();
        let wf = three_step();
        wf.checkpoint().save(dir.path()).unwrap();
        let mut other = Workflow::new("wf-other", vec![Step::new("a")]);
        let ck = Checkpoint::load(dir.path(), "wf-1").unwrap();
        let err = other.restore(ck).unwrap_err();
        assert!(matches!(err, HermesError::CheckpointMissing(_)));
    }

    #[test]
    fn is_complete_after_all_steps_done() {
        let mut wf = three_step();
        for _ in 0..3 {
            wf.start_step().unwrap();
            wf.finish_step(None).unwrap();
        }
        assert!(wf.is_complete());
        assert_eq!(wf.next_pending(), None);
    }
}
