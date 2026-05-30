// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave agent` CLI surface.
//!
//! A clap-free, side-effect-light presenter so the cave-cli front-end can
//! delegate the whole `agent` subcommand here and stay thin (and so this
//! surface is unit-testable without spawning a process). It reflects the
//! OpenJarvis local-first capability set and the self-improvement layer.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openjarvis::agent_state::{AgentState, AgentStateStore};

    fn temp_store() -> (tempfile::TempDir, AgentStateStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = AgentStateStore::new(dir.path().join("agents"));
        (dir, store)
    }

    #[test]
    fn info_mentions_openjarvis_and_versions() {
        let s = info();
        assert!(s.contains("OpenJarvis"));
        assert!(s.contains(crate::openjarvis::OPENJARVIS_UPSTREAM_VERSION));
        assert!(s.contains("self-improve"));
    }

    #[test]
    fn backends_lists_all_four() {
        let s = backends();
        for b in ["ollama", "mlx", "vllm", "hermes"] {
            assert!(s.contains(b), "missing backend {b} in:\n{s}");
        }
    }

    #[test]
    fn default_dispatch_is_info() {
        let (_d, store) = temp_store();
        assert_eq!(dispatch(&[], &store), info());
    }

    #[test]
    fn agents_lists_persisted_names() {
        let (_d, store) = temp_store();
        store.save(&AgentState::new("jarvis")).unwrap();
        let out = dispatch(&["agents".to_string()], &store);
        assert!(out.contains("jarvis"));
    }

    #[test]
    fn unknown_command_shows_usage() {
        let (_d, store) = temp_store();
        let out = dispatch(&["bogus".to_string()], &store);
        assert!(out.contains("unknown agent command"));
        assert!(out.contains("backends"));
    }
}
