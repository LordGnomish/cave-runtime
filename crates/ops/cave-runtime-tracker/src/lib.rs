// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-runtime-tracker — daily always-latest tracker for the
//! cave-runtime platform seat.
//!
//! cave-runtime reimplements ~80 upstream OSS projects as `cave-*`
//! crates. This crate keeps those ports honest day-over-day:
//!
//!   * **Phase 0** (this release) — poll the latest GitHub release/tag
//!     of every tracked upstream, compare against the version we have
//!     pinned, and write a `daily-<date>.{md,json}` drift report.
//!     **No automatic version bump.**
//!   * **Phase 1** (future) — wire `cavectl runtime-tracker apply` to
//!     open a port-loop task when an upstream drifts past a threshold.
//!
//! It is the sibling of [`cave-llm-tracker`] (the local-LLM seat) and
//! shares its daily-report shape, but is a deliberately separate copy:
//! the cave-runtime ↔ cave-home isolation rule forbids one tracker
//! binary serving both platforms. cave-home gets its own copy.

#![forbid(unsafe_code)]

pub mod config;
pub mod error;
pub mod measure;
pub mod poll;
pub mod registry;
pub mod report;

pub use config::TrackerConfig;
pub use error::{TrackerError, TrackerResult};
pub use measure::{
    measure_subset, parse_tokei_json, port_ratio, LocSource, LocStats, Measurement, TokeiLoc,
    DEFAULT_MEASURE_REPOS,
};
pub use poll::{poll_all, PollResult, PollSummary};
pub use registry::{
    default_registry, drift, DriftStatus, GithubFetcher, ReleaseFetcher, Upstream,
};
pub use report::{DailyReport, Totals};

/// Snapshot date of the curated registry. Bump whenever
/// [`default_registry`] gains/loses entries or repos are re-targeted.
pub const REGISTRY_SNAPSHOT: &str = "2026-06-07";

/// Convenience constructor used by the binary and integration tests.
pub fn default_config() -> TrackerConfig {
    TrackerConfig::default_config()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_polls_the_full_registry() {
        assert_eq!(default_config().upstreams.len(), default_registry().len());
    }

    #[test]
    fn snapshot_date_is_iso() {
        assert_eq!(REGISTRY_SNAPSHOT.len(), 10);
        assert!(REGISTRY_SNAPSHOT.starts_with("2026-"));
    }
}
