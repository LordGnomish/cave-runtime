// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-memory + optional CavePool-backed sandbox store.
//!
//! For the in-memory deep-port we keep state in a `DashMap`; persistence
//! through `cave_db::CavePool` is wired through the public constructor so
//! callers can opt-in. SQL DDL is provided for reference.

use crate::lifecycle::{LifecycleEvent, LifecycleMachine, LifecycleState};
use crate::models::{Runtime, Sandbox, SandboxState};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// SQL schema applied at boot (against the `cave_sandbox` Postgres schema).
pub const MIGRATION_V1: &str = "
CREATE TABLE IF NOT EXISTS sandboxes (
    id TEXT PRIMARY KEY,
    runtime TEXT NOT NULL,
    state TEXT NOT NULL,
    bundle TEXT NOT NULL,
    annotations JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS sandbox_lifecycle (
    id BIGSERIAL PRIMARY KEY,
    sandbox_id TEXT NOT NULL REFERENCES sandboxes(id) ON DELETE CASCADE,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    reason TEXT
);
CREATE INDEX IF NOT EXISTS idx_sandbox_lifecycle_sandbox ON sandbox_lifecycle(sandbox_id, at);
";

/// Persisted record (per sandbox).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredSandbox {
    pub sandbox: Sandbox,
    pub lifecycle: LifecycleMachine,
}

/// In-memory store. Concurrent via `DashMap`.
#[derive(Default, Clone)]
pub struct SandboxStore {
    inner: Arc<DashMap<String, StoredSandbox>>,
}

impl SandboxStore {
    pub fn new() -> Self { SandboxStore::default() }

    pub fn put(&self, sandbox: Sandbox) {
        let lifecycle = LifecycleMachine::new(sandbox.runtime.clone());
        let id = sandbox.id.clone();
        self.inner.insert(id, StoredSandbox { sandbox, lifecycle });
    }

    pub fn get(&self, id: &str) -> Option<StoredSandbox> {
        self.inner.get(id).map(|v| v.value().clone())
    }

    pub fn list(&self) -> Vec<StoredSandbox> {
        self.inner.iter().map(|v| v.value().clone()).collect()
    }

    pub fn remove(&self, id: &str) -> Option<StoredSandbox> {
        self.inner.remove(id).map(|(_, v)| v)
    }

    pub fn len(&self) -> usize { self.inner.len() }
    pub fn is_empty(&self) -> bool { self.inner.is_empty() }

    /// Drive a lifecycle transition; updates the sandbox's surface state too.
    pub fn transition(&self, id: &str, to: LifecycleState, reason: Option<String>) -> Result<(), String> {
        let mut entry = self.inner.get_mut(id).ok_or("not-found")?;
        entry.lifecycle.transition(to, reason)?;
        entry.sandbox.state = entry.lifecycle.state.to_sandbox_state();
        Ok(())
    }

    /// Lookup the lifecycle event log for one sandbox.
    pub fn history(&self, id: &str) -> Vec<LifecycleEvent> {
        self.inner.get(id).map(|v| v.lifecycle.history.clone()).unwrap_or_default()
    }

    /// Filter by runtime.
    pub fn by_runtime(&self, r: &Runtime) -> Vec<StoredSandbox> {
        self.inner.iter().filter(|v| &v.value().sandbox.runtime == r).map(|v| v.value().clone()).collect()
    }

    /// Snapshot count by state — for observability.
    pub fn count_by_state(&self) -> std::collections::BTreeMap<String, usize> {
        let mut out: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for v in self.inner.iter() {
            let s = match v.value().sandbox.state {
                SandboxState::Created => "created",
                SandboxState::Running => "running",
                SandboxState::Paused => "paused",
                SandboxState::Stopped => "stopped",
            };
            *out.entry(s.into()).or_insert(0) += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, r: Runtime) -> Sandbox {
        Sandbox {
            id: id.into(),
            runtime: r,
            state: SandboxState::Created,
            bundle: "/bundle".into(),
            annotations: Default::default(),
        }
    }

    #[test]
    fn put_get_remove() {
        let s = SandboxStore::new();
        s.put(sample("a", Runtime::Gvisor));
        assert_eq!(s.len(), 1);
        assert!(s.get("a").is_some());
        s.remove("a");
        assert!(s.is_empty());
    }

    #[test]
    fn list_returns_all() {
        let s = SandboxStore::new();
        s.put(sample("a", Runtime::Gvisor));
        s.put(sample("b", Runtime::Kata));
        s.put(sample("c", Runtime::Firecracker));
        assert_eq!(s.list().len(), 3);
    }

    #[test]
    fn transition_updates_surface_state() {
        let s = SandboxStore::new();
        s.put(sample("a", Runtime::Gvisor));
        s.transition("a", LifecycleState::Running, None).unwrap();
        assert_eq!(s.get("a").unwrap().sandbox.state, SandboxState::Running);
    }

    #[test]
    fn transition_illegal_keeps_state() {
        let s = SandboxStore::new();
        s.put(sample("a", Runtime::Kata));
        let err = s.transition("a", LifecycleState::Paused, None);
        assert!(err.is_err());
        assert_eq!(s.get("a").unwrap().sandbox.state, SandboxState::Created);
    }

    #[test]
    fn history_after_chain() {
        let s = SandboxStore::new();
        s.put(sample("a", Runtime::Firecracker));
        s.transition("a", LifecycleState::Running, None).unwrap();
        s.transition("a", LifecycleState::Stopped, None).unwrap();
        assert_eq!(s.history("a").len(), 2);
    }

    #[test]
    fn by_runtime_filters() {
        let s = SandboxStore::new();
        s.put(sample("a", Runtime::Gvisor));
        s.put(sample("b", Runtime::Kata));
        s.put(sample("c", Runtime::Gvisor));
        assert_eq!(s.by_runtime(&Runtime::Gvisor).len(), 2);
        assert_eq!(s.by_runtime(&Runtime::Kata).len(), 1);
    }

    #[test]
    fn count_by_state_aggregates() {
        let s = SandboxStore::new();
        s.put(sample("a", Runtime::Gvisor));
        s.put(sample("b", Runtime::Kata));
        s.transition("a", LifecycleState::Running, None).unwrap();
        let c = s.count_by_state();
        assert_eq!(c.get("running"), Some(&1));
        assert_eq!(c.get("created"), Some(&1));
    }

    #[test]
    fn migration_v1_has_tables() {
        assert!(MIGRATION_V1.contains("CREATE TABLE IF NOT EXISTS sandboxes"));
        assert!(MIGRATION_V1.contains("CREATE TABLE IF NOT EXISTS sandbox_lifecycle"));
    }
}
