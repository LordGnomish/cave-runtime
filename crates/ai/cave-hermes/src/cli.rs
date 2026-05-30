// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave agent` CLI surface.
//!
//! A clap-free, side-effect-light presenter so the cave-cli front-end can
//! delegate the whole `agent` subcommand here and stay thin (and so this
//! surface is unit-testable without spawning a process). It reflects the
//! OpenJarvis local-first capability set and the self-improvement layer.

use crate::openjarvis::agent_state::AgentStateStore;
use crate::openjarvis::backend::{Backend, BackendProfile, BackendRegistry};

/// The default local-first backend registry: the four switchable on-device
/// backends Cave ports, ranked Ollama > MLX > vLLM > Hermes by default.
pub fn default_backends() -> BackendRegistry {
    let mut r = BackendRegistry::new();
    r.register(BackendProfile::new(Backend::Ollama, "qwen3.6:35b-a3b-coding-mxfp8").priority(30));
    r.register(BackendProfile::new(Backend::Mlx, "mlx-community/Qwen3-7B").priority(20));
    r.register(BackendProfile::new(Backend::Vllm, "Qwen/Qwen3-7B").priority(10));
    r.register(BackendProfile::new(Backend::Hermes, "in-process").priority(0));
    r
}

/// Render the `agent` subcommand for `args` (the tokens after `cave agent`).
/// `store` supplies the persisted agent list for the `agents` verb.
pub fn dispatch(args: &[String], store: &AgentStateStore) -> String {
    match args.first().map(String::as_str) {
        Some("info") | None => info(),
        Some("backends") => backends(),
        Some("agents") => agents(store),
        Some(other) => format!("unknown agent command '{other}'\n\n{}", usage()),
    }
}

fn usage() -> String {
    [
        "cave agent — local-first OpenJarvis personal agent (ADR-RUNTIME-OPENJARVIS-ADOPTION-001)",
        "",
        "  info       capability + upstream summary (default)",
        "  backends   list switchable local-first inference backends",
        "  agents     list persisted agents under ~/.cave/agents/",
    ]
    .join("\n")
}

pub fn info() -> String {
    format!(
        "cave-agent (OpenJarvis local-first layer)\n\
         upstream: {} {}\n\
         hermes:   {} {}\n\
         primitives: backend-orchestration, eval(energy/latency/cost/accuracy), \
         persistent-state(~/.cave/agents), plan-and-execute, multi-agent\n\
         self-improve: observe(metrics/logs/trace), tune(opt-in), upstream-watch+hot-patch",
        crate::openjarvis::OPENJARVIS_UPSTREAM_REPO,
        crate::openjarvis::OPENJARVIS_UPSTREAM_VERSION,
        crate::UPSTREAM_REPO,
        crate::UPSTREAM_VERSION,
    )
}

pub fn backends() -> String {
    let r = default_backends();
    let mut lines = vec!["switchable local-first backends (priority desc):".to_string()];
    let mut cands = r.candidates();
    cands.sort_by(|a, b| b.priority.cmp(&a.priority));
    for p in cands {
        lines.push(format!(
            "  {:<7} {:<32} device={:?}",
            p.backend.as_str(),
            p.model,
            p.device
        ));
    }
    lines.join("\n")
}

fn agents(store: &AgentStateStore) -> String {
    match store.list() {
        Ok(names) if names.is_empty() => "no persisted agents under ~/.cave/agents/".to_string(),
        Ok(names) => {
            let mut out = vec![format!("{} persisted agent(s):", names.len())];
            out.extend(names.into_iter().map(|n| format!("  {n}")));
            out.join("\n")
        }
        Err(e) => format!("error listing agents: {e}"),
    }
}

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
