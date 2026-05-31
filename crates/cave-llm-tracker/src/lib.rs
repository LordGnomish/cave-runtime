// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-llm-tracker — daily always-latest tracker for the local-LLM seat.
//!
//! Burak runs a single local model (`qwen3.6:35b-a3b-coding-mxfp8`) as
//! the cave-runtime coding seat. This crate keeps that choice
//! defensible day-over-day:
//!
//!   * **Phase 0** — poll HuggingFace + Ollama library + LMSys leaderboard
//!     + GitHub backend (vLLM, llama.cpp, MLX-LM) releases, run five
//!     cave-specific eval prompts against each viable candidate, score
//!     them deterministically, and write a `daily-<date>.{md,json}`
//!     report. **No automatic baseline swap.**
//!   * **Phase 1** — wire `cavectl llm-tracker apply` to swap the
//!     baseline when a candidate clears both floors on three
//!     consecutive days.
//!
//! See `docs/adr/ADR-152_LLM_Tracker_Daily_Always_Latest.md` for the
//! motivation and the Charter v2 4-track scope.

#![forbid(unsafe_code)]

pub mod bench;
pub mod config;
pub mod error;
pub mod matrix;
pub mod notify;
pub mod poll;
pub mod registry;
pub mod report;
pub mod selection;
pub mod trend;

pub use bench::{cave_prompts, run_bench, score_response, synth_snapshot, BenchSnapshot, EvalPrompt, EvalResult};
pub use config::{BaselineConfig, BenchConfig, GuardsConfig, ReportConfig, SelectionConfig, SourcesConfig, TrackerConfig};
pub use error::{TrackerError, TrackerResult};
pub use notify::{build_notification, NoticeSeverity, Notification};
pub use poll::{poll_all, PollSummary};
pub use registry::{
    default_backend_repos, seed_catalog, Candidate, LiveFetcher, RegistryEndpoints, SourceKind,
};
pub use report::{shortlist, ConfigSummary, DailyReport};
pub use selection::{baseline_verdict, evaluate, SelectionStatus, Verdict};
pub use trend::{load_history, ModelTrend, TrendHistory, TrendPoint};

/// Pinned upstream snapshot. cave-llm-tracker has no single upstream;
/// the per-source pins live in `parity.manifest.toml [upstream] source_sha`
/// as an inline TOML table. This constant is the snapshot date the
/// inline-table was captured on — bump it whenever the manifest is
/// re-pinned.
pub const UPSTREAM_VERSION: &str = "2026-05-21";

/// Convenience constructor used by the binary and integration tests.
pub fn default_config() -> TrackerConfig {
    TrackerConfig::default_config()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_version_matches_pin_date() {
        assert_eq!(UPSTREAM_VERSION, "2026-05-21");
    }

    #[test]
    fn default_config_baseline_is_burak_seat() {
        let c = default_config();
        assert_eq!(c.baseline.model, "qwen3.6:35b-a3b-coding-mxfp8");
        assert!(!c.selection.auto_swap, "Phase 0 mandate violated");
    }

    #[test]
    fn seed_catalog_alone_satisfies_floor_of_five() {
        // Smoke-test the contract for `--mode report` against an offline
        // host: even without any network, the report must surface >= 5
        // candidate rows.
        assert!(seed_catalog().len() >= 5);
    }
}
