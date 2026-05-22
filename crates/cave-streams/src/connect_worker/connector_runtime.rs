// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-connector lifecycle — start a connector, generate its
//! tasks, stop and tear it down. Mirrors
//! `org.apache.kafka.connect.runtime.WorkerConnector` from
//! upstream. Sits one level above [`super::task_runtime`] (a
//! Connector produces N TaskRuntimes).

use std::collections::HashMap;

use crate::error::StreamsError;

use super::task_runtime::TaskKind;

/// Connector configuration as the REST API received it, with
/// derived task spawn information.
#[derive(Debug, Clone)]
pub struct ConnectorSpec {
    pub name: String,
    pub kind: TaskKind,
    /// Max tasks the connector can spawn (`tasks.max`).
    pub tasks_max: u32,
    /// Original config map.
    pub config: HashMap<String, String>,
}

impl ConnectorSpec {
    pub fn new(name: impl Into<String>, kind: TaskKind, tasks_max: u32) -> Self {
        Self {
            name: name.into(),
            kind,
            tasks_max,
            config: HashMap::new(),
        }
    }

    pub fn with_config(mut self, kvs: impl IntoIterator<Item = (String, String)>) -> Self {
        for (k, v) in kvs {
            self.config.insert(k, v);
        }
        self
    }
}

/// Lifecycle phase the connector is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorLifecycle {
    /// Newly registered; tasks not generated yet.
    Created,
    /// `generate_tasks` has been called; tasks are ready for
    /// the Worker to pick up.
    Generated,
    /// Running normally.
    Running,
    /// Operator-paused.
    Paused,
    /// `stop` was called; lifecycle is final.
    Stopped,
}

/// One connector inside the runtime.
pub struct ConnectorRuntime {
    pub spec: ConnectorSpec,
    pub lifecycle: ConnectorLifecycle,
    /// Task ids generated for this connector — stable
    /// `<name>:<index>` form. Empty until `generate_tasks`
    /// fires.
    pub task_ids: Vec<String>,
}

impl ConnectorRuntime {
    pub fn new(spec: ConnectorSpec) -> Self {
        Self {
            spec,
            lifecycle: ConnectorLifecycle::Created,
            task_ids: Vec::new(),
        }
    }

    /// Mint task ids per the spec's `tasks_max`. Returns the
    /// id list so the caller can hand them to the
    /// [`AssignmentTable`](super::AssignmentTable).
    pub fn generate_tasks(&mut self) -> Result<Vec<String>, StreamsError> {
        if self.spec.tasks_max == 0 {
            return Err(StreamsError::Internal("tasks.max must be ≥ 1".into()));
        }
        let mut ids = Vec::with_capacity(self.spec.tasks_max as usize);
        for i in 0..self.spec.tasks_max {
            ids.push(format!("{}:{}", self.spec.name, i));
        }
        self.task_ids = ids.clone();
        self.lifecycle = ConnectorLifecycle::Generated;
        Ok(ids)
    }

    /// Transition to Running once the Worker has picked up its
    /// tasks. Idempotent.
    pub fn mark_running(&mut self) {
        if matches!(
            self.lifecycle,
            ConnectorLifecycle::Generated | ConnectorLifecycle::Paused
        ) {
            self.lifecycle = ConnectorLifecycle::Running;
        }
    }

    /// Pause — Workers stop ticking the tasks but the spec
    /// stays. `resume` reverses.
    pub fn pause(&mut self) {
        if self.lifecycle == ConnectorLifecycle::Running {
            self.lifecycle = ConnectorLifecycle::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.lifecycle == ConnectorLifecycle::Paused {
            self.lifecycle = ConnectorLifecycle::Running;
        }
    }

    /// Stop the connector and clear task ids. After this the
    /// connector is final — recreate to restart.
    pub fn stop(&mut self) {
        self.lifecycle = ConnectorLifecycle::Stopped;
        self.task_ids.clear();
    }

    /// Validate the spec — runs the upstream
    /// `Connector.validate()` shape: tasks_max in [1, 1024],
    /// connector name non-empty, no whitespace in name.
    pub fn validate(&self) -> Result<(), StreamsError> {
        if self.spec.name.is_empty() {
            return Err(StreamsError::Internal(
                "connector name must not be empty".into(),
            ));
        }
        if self.spec.name.chars().any(|c| c.is_whitespace()) {
            return Err(StreamsError::Internal(format!(
                "connector name must not contain whitespace: '{}'",
                self.spec.name
            )));
        }
        if self.spec.tasks_max == 0 || self.spec.tasks_max > 1024 {
            return Err(StreamsError::Internal(format!(
                "tasks.max must be in [1, 1024], got {}",
                self.spec.tasks_max
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, max: u32) -> ConnectorSpec {
        ConnectorSpec::new(name, TaskKind::Source, max)
    }

    #[test]
    fn new_starts_created_with_no_tasks() {
        let c = ConnectorRuntime::new(spec("c", 4));
        assert_eq!(c.lifecycle, ConnectorLifecycle::Created);
        assert!(c.task_ids.is_empty());
    }

    #[test]
    fn generate_tasks_mints_n_ids() {
        let mut c = ConnectorRuntime::new(spec("c", 4));
        let ids = c.generate_tasks().unwrap();
        assert_eq!(ids, vec!["c:0", "c:1", "c:2", "c:3"]);
        assert_eq!(c.lifecycle, ConnectorLifecycle::Generated);
    }

    #[test]
    fn generate_tasks_zero_fails() {
        let mut c = ConnectorRuntime::new(spec("c", 0));
        assert!(c.generate_tasks().is_err());
    }

    #[test]
    fn mark_running_only_from_generated_or_paused() {
        let mut c = ConnectorRuntime::new(spec("c", 1));
        // From Created — no-op.
        c.mark_running();
        assert_eq!(c.lifecycle, ConnectorLifecycle::Created);
        c.generate_tasks().unwrap();
        c.mark_running();
        assert_eq!(c.lifecycle, ConnectorLifecycle::Running);
    }

    #[test]
    fn pause_resume_round_trips() {
        let mut c = ConnectorRuntime::new(spec("c", 1));
        c.generate_tasks().unwrap();
        c.mark_running();
        c.pause();
        assert_eq!(c.lifecycle, ConnectorLifecycle::Paused);
        c.resume();
        assert_eq!(c.lifecycle, ConnectorLifecycle::Running);
    }

    #[test]
    fn pause_no_op_if_not_running() {
        let mut c = ConnectorRuntime::new(spec("c", 1));
        c.pause();
        assert_eq!(c.lifecycle, ConnectorLifecycle::Created);
    }

    #[test]
    fn stop_terminates_and_clears_tasks() {
        let mut c = ConnectorRuntime::new(spec("c", 3));
        c.generate_tasks().unwrap();
        c.stop();
        assert_eq!(c.lifecycle, ConnectorLifecycle::Stopped);
        assert!(c.task_ids.is_empty());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let c = ConnectorRuntime::new(spec("", 1));
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_whitespace_name() {
        let c = ConnectorRuntime::new(spec("bad name", 1));
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_tasks_max_zero_or_huge() {
        let c0 = ConnectorRuntime::new(spec("c", 0));
        assert!(c0.validate().is_err());
        let cbig = ConnectorRuntime::new(spec("c", 100_000));
        assert!(cbig.validate().is_err());
    }

    #[test]
    fn validate_accepts_normal_spec() {
        let c = ConnectorRuntime::new(spec("orders-source", 16));
        c.validate().unwrap();
    }

    #[test]
    fn spec_with_config_stores_kvs() {
        let s = ConnectorSpec::new("c", TaskKind::Source, 1).with_config([
            ("connector.class".into(), "Jdbc".into()),
            ("topic".into(), "orders".into()),
        ]);
        assert_eq!(s.config.len(), 2);
        assert_eq!(s.config.get("topic"), Some(&"orders".to_string()));
    }
}
