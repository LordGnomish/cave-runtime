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
