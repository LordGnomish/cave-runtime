// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Standalone herder. Mirrors upstream
//! `connect/runtime/standalone/StandaloneHerder.java`.
//!
//! A single-process variant of the Connect herder. Connector +
//! task configs live in-memory, lifecycle is synchronous, no
//! group-coordination protocol runs. Distributed-only
//! operations (`put_task_configs`, `fence_zombie_source_tasks`)
//! throw [`HerderError::Unsupported`].

use std::collections::BTreeMap;

use super::offset_store::{OffsetKey, OffsetValue};
use super::task_runtime::TaskKind;

/// Coarse-grained connector state pushed *to* the worker. Mirrors
/// upstream `TargetState` (STARTED / PAUSED / STOPPED). `STARTED`
/// is the default after a successful `put_connector_config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetState {
    Started,
    Paused,
    Stopped,
}

#[derive(Debug, thiserror::Error)]
pub enum HerderError {
    #[error("connector {0} already exists")]
    AlreadyExists(String),
    #[error("connector {0} not found")]
    NotFound(String),
    #[error("bad connector config: {0}")]
    BadConfig(String),
    #[error("illegal state: {0}")]
    IllegalState(String),
    #[error("not supported in standalone mode: {0}")]
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorInfo {
    pub name: String,
    pub created: bool,
    pub kind: TaskKind,
    pub tasks: Vec<TaskInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInfo {
    pub task: u32,
    pub failure_trace: Option<String>,
}

/// Single-process record of a connector — configuration + the
/// task slots that the herder has spawned for it. Tasks are
/// reduced to zero when the target state is `Stopped`.
#[derive(Debug, Clone)]
struct ConnectorEntry {
    config: BTreeMap<String, String>,
    kind: TaskKind,
    tasks_max: u32,
    target_state: TargetState,
    tasks: Vec<TaskInfo>,
    offsets: BTreeMap<BTreeMap<String, String>, BTreeMap<String, String>>,
}

impl ConnectorEntry {
    fn rebuild_tasks(&mut self) {
        if self.target_state == TargetState::Stopped {
            self.tasks.clear();
            return;
        }
        self.tasks = (0..self.tasks_max)
            .map(|i| TaskInfo {
                task: i,
                failure_trace: None,
            })
            .collect();
    }
}

#[derive(Default)]
pub struct StandaloneHerder {
    connectors: BTreeMap<String, ConnectorEntry>,
}

impl StandaloneHerder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_connector_config(
        &mut self,
        name: &str,
        config: BTreeMap<String, String>,
        allow_replace: bool,
    ) -> Result<ConnectorInfo, HerderError> {
        self.put_connector_config_with_state(name, config, TargetState::Started, allow_replace)
    }

    pub fn put_connector_config_with_state(
        &mut self,
        name: &str,
        config: BTreeMap<String, String>,
        initial: TargetState,
        allow_replace: bool,
    ) -> Result<ConnectorInfo, HerderError> {
        if name.is_empty() {
            return Err(HerderError::BadConfig("connector name empty".into()));
        }
        if self.connectors.contains_key(name) && !allow_replace {
            return Err(HerderError::AlreadyExists(name.into()));
        }
        validate_config(&config)?;
        let kind = derive_kind(&config);
        let tasks_max = config
            .get("tasks.max")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1);
        let mut entry = ConnectorEntry {
            config,
            kind,
            tasks_max,
            target_state: initial,
            tasks: Vec::new(),
            offsets: BTreeMap::new(),
        };
        entry.rebuild_tasks();
        let created = !self.connectors.contains_key(name);
        let info = ConnectorInfo {
            name: name.into(),
            created,
            kind,
            tasks: entry.tasks.clone(),
        };
        self.connectors.insert(name.into(), entry);
        Ok(info)
    }

    pub fn delete_connector(&mut self, name: &str) -> Result<(), HerderError> {
        self.connectors
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| HerderError::NotFound(name.into()))
    }

    pub fn patch_connector_config(
        &mut self,
        name: &str,
        patch: BTreeMap<String, String>,
    ) -> Result<ConnectorInfo, HerderError> {
        let entry = self
            .connectors
            .get_mut(name)
            .ok_or_else(|| HerderError::NotFound(name.into()))?;
        for (k, v) in patch {
            entry.config.insert(k, v);
        }
        validate_config(&entry.config)?;
        entry.kind = derive_kind(&entry.config);
        entry.tasks_max = entry
            .config
            .get("tasks.max")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1);
        entry.rebuild_tasks();
        Ok(ConnectorInfo {
            name: name.into(),
            created: false,
            kind: entry.kind,
            tasks: entry.tasks.clone(),
        })
    }

    pub fn set_target_state(
        &mut self,
        name: &str,
        state: TargetState,
    ) -> Result<(), HerderError> {
        let entry = self
            .connectors
            .get_mut(name)
            .ok_or_else(|| HerderError::NotFound(name.into()))?;
        entry.target_state = state;
        entry.rebuild_tasks();
        Ok(())
    }

    pub fn stop_connector(&mut self, name: &str) -> Result<(), HerderError> {
        self.set_target_state(name, TargetState::Stopped)
    }

    pub fn target_state(&self, name: &str) -> Result<TargetState, HerderError> {
        self.connectors
            .get(name)
            .map(|e| e.target_state)
            .ok_or_else(|| HerderError::NotFound(name.into()))
    }

    pub fn restart_connector(&mut self, name: &str) -> Result<(), HerderError> {
        let entry = self
            .connectors
            .get_mut(name)
            .ok_or_else(|| HerderError::NotFound(name.into()))?;
        // Clear any per-task failure traces on the way back up.
        for t in entry.tasks.iter_mut() {
            t.failure_trace = None;
        }
        entry.target_state = TargetState::Started;
        entry.rebuild_tasks();
        Ok(())
    }

    pub fn restart_task(&mut self, name: &str, task: u32) -> Result<(), HerderError> {
        let entry = self
            .connectors
            .get_mut(name)
            .ok_or_else(|| HerderError::NotFound(name.into()))?;
        let slot = entry
            .tasks
            .iter_mut()
            .find(|t| t.task == task)
            .ok_or_else(|| HerderError::NotFound(format!("{name}:{task}")))?;
        slot.failure_trace = None;
        Ok(())
    }

    pub fn fail_task(
        &mut self,
        name: &str,
        task: u32,
        trace: impl Into<String>,
    ) -> Result<(), HerderError> {
        let entry = self
            .connectors
            .get_mut(name)
            .ok_or_else(|| HerderError::NotFound(name.into()))?;
        let slot = entry
            .tasks
            .iter_mut()
            .find(|t| t.task == task)
            .ok_or_else(|| HerderError::NotFound(format!("{name}:{task}")))?;
        slot.failure_trace = Some(trace.into());
        Ok(())
    }

    pub fn connectors(&self) -> Vec<String> {
        self.connectors.keys().cloned().collect()
    }

    pub fn connector_info(&self, name: &str) -> Result<ConnectorInfo, HerderError> {
        let e = self
            .connectors
            .get(name)
            .ok_or_else(|| HerderError::NotFound(name.into()))?;
        Ok(ConnectorInfo {
            name: name.into(),
            created: false,
            kind: e.kind,
            tasks: e.tasks.clone(),
        })
    }

    pub fn connector_config(
        &self,
        name: &str,
    ) -> Result<BTreeMap<String, String>, HerderError> {
        self.connectors
            .get(name)
            .map(|e| e.config.clone())
            .ok_or_else(|| HerderError::NotFound(name.into()))
    }

    pub fn task_configs(&self, name: &str) -> Result<Vec<TaskInfo>, HerderError> {
        self.connectors
            .get(name)
            .map(|e| e.tasks.clone())
            .ok_or_else(|| HerderError::NotFound(name.into()))
    }

    pub fn put_task_configs(
        &mut self,
        _name: &str,
        _configs: Vec<BTreeMap<String, String>>,
    ) -> Result<(), HerderError> {
        Err(HerderError::Unsupported(
            "put_task_configs is distributed-only".into(),
        ))
    }

    pub fn fence_zombie_source_tasks(&mut self, _name: &str) -> Result<(), HerderError> {
        Err(HerderError::Unsupported(
            "fence_zombie_source_tasks is distributed-only".into(),
        ))
    }

    pub fn alter_connector_offsets(
        &mut self,
        name: &str,
        offsets: Vec<(BTreeMap<String, String>, Option<BTreeMap<String, String>>)>,
    ) -> Result<(), HerderError> {
        let entry = self
            .connectors
            .get_mut(name)
            .ok_or_else(|| HerderError::NotFound(name.into()))?;
        if entry.target_state != TargetState::Stopped {
            return Err(HerderError::IllegalState(format!(
                "connector {name} must be STOPPED before altering offsets"
            )));
        }
        for (part, opt) in offsets {
            match opt {
                Some(v) => {
                    entry.offsets.insert(part, v);
                }
                None => {
                    entry.offsets.remove(&part);
                }
            }
        }
        Ok(())
    }

    pub fn connector_offsets(
        &self,
        name: &str,
    ) -> Result<
        BTreeMap<BTreeMap<String, String>, BTreeMap<String, String>>,
        HerderError,
    > {
        self.connectors
            .get(name)
            .map(|e| e.offsets.clone())
            .ok_or_else(|| HerderError::NotFound(name.into()))
    }

    pub fn reset_connector_offsets(&mut self, name: &str) -> Result<(), HerderError> {
        let entry = self
            .connectors
            .get_mut(name)
            .ok_or_else(|| HerderError::NotFound(name.into()))?;
        if entry.target_state != TargetState::Stopped {
            return Err(HerderError::IllegalState(format!(
                "connector {name} must be STOPPED before resetting offsets"
            )));
        }
        entry.offsets.clear();
        Ok(())
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn validate_config(cfg: &BTreeMap<String, String>) -> Result<(), HerderError> {
    if !cfg.contains_key("connector.class") {
        return Err(HerderError::BadConfig(
            "connector.class is required".into(),
        ));
    }
    Ok(())
}

fn derive_kind(cfg: &BTreeMap<String, String>) -> TaskKind {
    let cls = cfg.get("connector.class").map(|s| s.as_str()).unwrap_or("");
    let low = cls.to_lowercase();
    if low.contains("source") {
        TaskKind::Source
    } else if low.contains("sink") {
        TaskKind::Sink
    } else {
        TaskKind::Source
    }
}

// Helper to plug a Kafka-backed offsets store through here. Not
// wired in this batch; kept to keep the seam exposed.
#[doc(hidden)]
pub fn _offset_key_for(connector: &str, partition: BTreeMap<String, String>) -> OffsetKey {
    OffsetKey {
        connector: connector.into(),
        partition,
    }
}
#[doc(hidden)]
pub fn _offset_value_pair(k: &str, v: &str) -> OffsetValue {
    let mut m = BTreeMap::new();
    m.insert(k.into(), v.into());
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(class: &str, n: u32) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("connector.class".into(), class.into());
        m.insert("tasks.max".into(), n.to_string());
        m
    }

    #[test]
    fn create_source_starts_running_with_tasks() {
        let mut h = StandaloneHerder::new();
        let info = h
            .put_connector_config("a", cfg("...JdbcSource", 3), false)
            .unwrap();
        assert!(info.created);
        assert_eq!(info.kind, TaskKind::Source);
        assert_eq!(info.tasks.len(), 3);
        assert_eq!(h.target_state("a").unwrap(), TargetState::Started);
    }

    #[test]
    fn create_sink_classifies_kind_correctly() {
        let mut h = StandaloneHerder::new();
        let info = h.put_connector_config("a", cfg("...HdfsSink", 1), false).unwrap();
        assert_eq!(info.kind, TaskKind::Sink);
    }

    #[test]
    fn duplicate_create_is_rejected_unless_allow_replace() {
        let mut h = StandaloneHerder::new();
        h.put_connector_config("a", cfg("...Source", 1), false).unwrap();
        assert!(h.put_connector_config("a", cfg("...Source", 1), false).is_err());
        assert!(h.put_connector_config("a", cfg("...Source", 2), true).is_ok());
        assert_eq!(h.task_configs("a").unwrap().len(), 2);
    }

    #[test]
    fn empty_name_is_rejected() {
        let mut h = StandaloneHerder::new();
        assert!(h.put_connector_config("", cfg("...Source", 1), false).is_err());
    }

    #[test]
    fn missing_connector_class_is_bad_config() {
        let mut h = StandaloneHerder::new();
        let mut bad = BTreeMap::new();
        bad.insert("tasks.max".into(), "1".into());
        assert!(matches!(
            h.put_connector_config("a", bad, false).unwrap_err(),
            HerderError::BadConfig(_)
        ));
    }

    #[test]
    fn delete_unknown_returns_not_found() {
        let mut h = StandaloneHerder::new();
        assert!(matches!(
            h.delete_connector("nope").unwrap_err(),
            HerderError::NotFound(_)
        ));
    }

    #[test]
    fn patch_merges_and_revalidates() {
        let mut h = StandaloneHerder::new();
        h.put_connector_config("a", cfg("...Source", 1), false).unwrap();
        let mut p = BTreeMap::new();
        p.insert("topics".into(), "orders".into());
        h.patch_connector_config("a", p).unwrap();
        assert_eq!(
            h.connector_config("a").unwrap().get("topics"),
            Some(&"orders".to_string())
        );
    }

    #[test]
    fn stopped_state_drops_task_slots() {
        let mut h = StandaloneHerder::new();
        h.put_connector_config("a", cfg("...Source", 4), false).unwrap();
        h.stop_connector("a").unwrap();
        assert_eq!(h.task_configs("a").unwrap().len(), 0);
    }

    #[test]
    fn paused_keeps_task_slots() {
        let mut h = StandaloneHerder::new();
        h.put_connector_config("a", cfg("...Source", 4), false).unwrap();
        h.set_target_state("a", TargetState::Paused).unwrap();
        assert_eq!(h.task_configs("a").unwrap().len(), 4);
        assert_eq!(h.target_state("a").unwrap(), TargetState::Paused);
    }

    #[test]
    fn restart_task_clears_trace() {
        let mut h = StandaloneHerder::new();
        h.put_connector_config("a", cfg("...Source", 1), false).unwrap();
        h.fail_task("a", 0, "boom").unwrap();
        assert!(h.task_configs("a").unwrap()[0].failure_trace.is_some());
        h.restart_task("a", 0).unwrap();
        assert!(h.task_configs("a").unwrap()[0].failure_trace.is_none());
    }

    #[test]
    fn alter_offsets_rejected_when_running() {
        let mut h = StandaloneHerder::new();
        h.put_connector_config("a", cfg("...Source", 1), false).unwrap();
        let res = h.alter_connector_offsets("a", vec![(BTreeMap::new(), None)]);
        assert!(matches!(res.unwrap_err(), HerderError::IllegalState(_)));
    }

    #[test]
    fn alter_offsets_works_when_stopped() {
        let mut h = StandaloneHerder::new();
        h.put_connector_config("a", cfg("...Source", 1), false).unwrap();
        h.stop_connector("a").unwrap();
        let mut part = BTreeMap::new();
        part.insert("table".into(), "orders".into());
        let mut off = BTreeMap::new();
        off.insert("position".into(), "5".into());
        h.alter_connector_offsets("a", vec![(part.clone(), Some(off.clone()))])
            .unwrap();
        assert_eq!(h.connector_offsets("a").unwrap().get(&part), Some(&off));
    }

    #[test]
    fn put_task_configs_is_unsupported() {
        let mut h = StandaloneHerder::new();
        assert!(matches!(
            h.put_task_configs("a", vec![]).unwrap_err(),
            HerderError::Unsupported(_)
        ));
    }

    #[test]
    fn fence_zombies_is_unsupported() {
        let mut h = StandaloneHerder::new();
        assert!(matches!(
            h.fence_zombie_source_tasks("a").unwrap_err(),
            HerderError::Unsupported(_)
        ));
    }

    #[test]
    fn reset_offsets_requires_stopped() {
        let mut h = StandaloneHerder::new();
        h.put_connector_config("a", cfg("...Source", 1), false).unwrap();
        assert!(matches!(
            h.reset_connector_offsets("a").unwrap_err(),
            HerderError::IllegalState(_)
        ));
        h.stop_connector("a").unwrap();
        assert!(h.reset_connector_offsets("a").is_ok());
    }
}
