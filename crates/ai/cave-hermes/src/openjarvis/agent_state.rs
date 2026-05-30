// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Persistent agent state — OpenJarvis primitive.
//!
//! Each named local agent owns a directory on the developer's machine,
//! `~/.cave/agents/{name}/`, holding its serialized [`AgentState`]
//! (`state.json`) and an append-only activity journal (`journal.jsonl`).
//! This is what makes a personal agent *durable* across process restarts
//! without any server — the local-first counterpart to the server-side
//! [`crate::memory`] backends.
//!
//! Root resolution honours `CAVE_HOME` (so a cluster or a test can redirect
//! it), then falls back to the user's home `~/.cave`, then a cwd-relative
//! `.cave` as a last resort.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::HermesError;

/// Durable state for one named local agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentState {
    pub name: String,
    /// Selected backend name (`ollama` / `vllm` / `mlx` / `hermes`), if pinned.
    #[serde(default)]
    pub backend: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl AgentState {
    pub fn new(name: impl Into<String>) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            name: name.into(),
            backend: None,
            created_at: now.clone(),
            updated_at: now,
            metadata: BTreeMap::new(),
        }
    }
}

/// Resolve the agents root directory. `CAVE_HOME` wins (`{cave_home}/agents`),
/// then `{home}/.cave/agents`, then a cwd-relative `.cave/agents`.
pub fn resolve_agents_root(cave_home: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    if let Some(ch) = cave_home {
        return ch.join("agents");
    }
    if let Some(h) = home {
        return h.join(".cave").join("agents");
    }
    PathBuf::from(".cave").join("agents")
}

/// Validate an agent name: non-empty, no path separators or traversal.
fn check_name(name: &str) -> crate::error::Result<()> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.contains('\0')
    {
        return Err(HermesError::AgentState(format!("invalid agent name {name:?}")));
    }
    Ok(())
}

/// Filesystem-backed store of [`AgentState`] rooted at one directory.
#[derive(Debug, Clone)]
pub struct AgentStateStore {
    root: PathBuf,
}

impl AgentStateStore {
    /// Construct over an explicit root (tests pass a tempdir).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Construct over the process-resolved root (`CAVE_HOME` → home → cwd).
    pub fn default_root() -> Self {
        let cave_home = std::env::var_os("CAVE_HOME").map(PathBuf::from);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self::new(resolve_agents_root(cave_home, home))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn agent_dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn state_path(&self, name: &str) -> PathBuf {
        self.agent_dir(name).join("state.json")
    }

    fn journal_path(&self, name: &str) -> PathBuf {
        self.agent_dir(name).join("journal.jsonl")
    }

    /// Persist `state` to `{root}/{name}/state.json`, refreshing `updated_at`.
    pub fn save(&self, state: &AgentState) -> crate::error::Result<()> {
        check_name(&state.name)?;
        let mut state = state.clone();
        state.updated_at = chrono::Utc::now().to_rfc3339();
        std::fs::create_dir_all(self.agent_dir(&state.name))?;
        let json = serde_json::to_string_pretty(&state)?;
        std::fs::write(self.state_path(&state.name), json)?;
        Ok(())
    }

    pub fn load(&self, name: &str) -> crate::error::Result<AgentState> {
        check_name(name)?;
        let path = self.state_path(name);
        if !path.exists() {
            return Err(HermesError::AgentState(format!("agent '{name}' not found")));
        }
        let raw = std::fs::read_to_string(path)?;
        let state: AgentState = serde_json::from_str(&raw)?;
        Ok(state)
    }

    pub fn exists(&self, name: &str) -> bool {
        check_name(name).is_ok() && self.state_path(name).exists()
    }

    pub fn delete(&self, name: &str) -> crate::error::Result<()> {
        check_name(name)?;
        let dir = self.agent_dir(name);
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    /// All agent names with a persisted `state.json`, sorted.
    pub fn list(&self) -> crate::error::Result<Vec<String>> {
        let mut names = Vec::new();
        if !self.root.exists() {
            return Ok(names);
        }
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().join("state.json").exists() {
                    names.push(name);
                }
            }
        }
        names.sort();
        Ok(names)
    }

    /// Append one line to the agent's activity journal.
    pub fn append_journal(&self, name: &str, line: &str) -> crate::error::Result<()> {
        use std::io::Write;
        check_name(name)?;
        std::fs::create_dir_all(self.agent_dir(name))?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.journal_path(name))?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// Read the agent's journal lines in append order.
    pub fn read_journal(&self, name: &str) -> crate::error::Result<Vec<String>> {
        check_name(name)?;
        let path = self.journal_path(name);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(raw.lines().map(str::to_string).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_store() -> (tempfile::TempDir, AgentStateStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = AgentStateStore::new(dir.path().join("agents"));
        (dir, store)
    }

    #[test]
    fn resolve_root_prefers_cave_home() {
        let root = resolve_agents_root(Some(PathBuf::from("/srv/cave")), Some(PathBuf::from("/home/x")));
        assert_eq!(root, PathBuf::from("/srv/cave/agents"));
    }

    #[test]
    fn resolve_root_falls_back_to_home_dotcave() {
        let root = resolve_agents_root(None, Some(PathBuf::from("/home/x")));
        assert_eq!(root, PathBuf::from("/home/x/.cave/agents"));
    }

    #[test]
    fn save_then_load_roundtrips() {
        let (_d, store) = temp_store();
        let mut st = AgentState::new("jarvis");
        st.backend = Some("ollama".into());
        st.metadata.insert("model".into(), "qwen3".into());
        store.save(&st).unwrap();

        let back = store.load("jarvis").unwrap();
        assert_eq!(back.name, "jarvis");
        assert_eq!(back.backend.as_deref(), Some("ollama"));
        assert_eq!(back.metadata.get("model").map(String::as_str), Some("qwen3"));
    }

    #[test]
    fn agent_dir_contains_name_and_agents_segment() {
        let (_d, store) = temp_store();
        let p = store.agent_dir("jarvis");
        assert!(p.ends_with("agents/jarvis"), "got {p:?}");
    }

    #[test]
    fn list_returns_saved_agent_names_sorted() {
        let (_d, store) = temp_store();
        store.save(&AgentState::new("zeta")).unwrap();
        store.save(&AgentState::new("alpha")).unwrap();
        assert_eq!(store.list().unwrap(), vec!["alpha".to_string(), "zeta".to_string()]);
    }

    #[test]
    fn exists_and_delete() {
        let (_d, store) = temp_store();
        store.save(&AgentState::new("tmp")).unwrap();
        assert!(store.exists("tmp"));
        store.delete("tmp").unwrap();
        assert!(!store.exists("tmp"));
    }

    #[test]
    fn load_missing_agent_errors() {
        let (_d, store) = temp_store();
        let err = store.load("ghost").unwrap_err();
        assert!(matches!(err, crate::error::HermesError::AgentState(_)));
    }

    #[test]
    fn empty_or_traversal_name_rejected() {
        let (_d, store) = temp_store();
        assert!(store.save(&AgentState::new("")).is_err());
        assert!(store.save(&AgentState::new("../escape")).is_err());
    }

    #[test]
    fn journal_appends_and_reads_back_in_order() {
        let (_d, store) = temp_store();
        store.save(&AgentState::new("jarvis")).unwrap();
        store.append_journal("jarvis", "started").unwrap();
        store.append_journal("jarvis", "ran plan").unwrap();
        let lines = store.read_journal("jarvis").unwrap();
        assert_eq!(lines, vec!["started".to_string(), "ran plan".to_string()]);
    }
}
