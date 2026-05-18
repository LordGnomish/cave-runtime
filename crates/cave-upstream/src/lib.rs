// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE Upstream — Continuous open source project tracking + self-improving port loop.
//!
//! ## Two halves
//!
//! 1. **Legacy registry/tracker** (`tracker`, `projects`, `engine`, `models`,
//!    `store`, `routes`) — original weekly-cron driven scan + HTTP API surface.
//!    Kept intact.
//!
//! 2. **Watch daemon** (`state`, `delta`, `pump`, `daemon`) — production poller
//!    that detects new upstream releases at a tiered cadence (15-minute for
//!    high-priority modules, 60-minute for the rest), computes a delta, and
//!    emits a payload that drives the Qwen TDD port loop. This is the heart of
//!    the Charter "self-improving" article.
//!
//! See `docs/adr/ADR-RUNTIME-UPSTREAM-WATCH-001.md` for the design rationale.
//!
//! Both halves share `projects::TRACKED_PROJECTS` as the single source of truth.

pub mod tracker;
pub mod projects;
pub mod engine;
pub mod models;
pub mod store;
pub mod routes;
pub mod adr_links;

// ── Watch daemon ────────────────────────────────────────────────────────────

pub mod state;
pub mod delta;
pub mod pump;
pub mod daemon;

/// All upstream projects we track.
pub use projects::TRACKED_PROJECTS;

/// High-priority CAVE modules that get the fast (15-minute) tracking cadence.
///
/// Everything else falls back to the normal (60-minute) cadence. Match is by
/// exact equality against `TrackedProject::cave_module`.
///
/// Source: Charter "self-improving" article — these 12 modules form the
/// runtime kernel. New releases here should reach the port queue first.
pub const HIGH_PRIORITY_MODULES: &[&str] = &[
    "cave-apiserver",
    "cave-etcd",
    "cave-scheduler",
    "cave-cri",
    "cave-net",
    "cave-mesh",
    "cave-streams",
    "cave-pg",
    "cave-docdb",
    "cave-vault",
    "cave-cache",
    "cave-registry",
];

/// Returns `true` if the given `cave_module` is on the high-priority list.
pub fn is_high_priority(cave_module: &str) -> bool {
    HIGH_PRIORITY_MODULES.iter().any(|m| *m == cave_module)
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn high_priority_contains_kernel_modules() {
        assert!(is_high_priority("cave-apiserver"));
        assert!(is_high_priority("cave-etcd"));
        assert!(is_high_priority("cave-scheduler"));
        assert!(is_high_priority("cave-cri"));
        assert!(is_high_priority("cave-cache"));
        assert!(is_high_priority("cave-registry"));
    }

    #[test]
    fn high_priority_excludes_non_kernel_modules() {
        assert!(!is_high_priority("cave-portal"));
        assert!(!is_high_priority("cave-changelog"));
        assert!(!is_high_priority(""));
    }

    #[test]
    fn high_priority_list_size_matches_charter() {
        assert_eq!(HIGH_PRIORITY_MODULES.len(), 12);
    }
}
