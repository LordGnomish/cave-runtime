// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! # cave-agent
//!
//! OpenJarvis-parity composable on-device agent runtime.
//!
//! Upstream: [open-jarvis/OpenJarvis](https://github.com/open-jarvis/OpenJarvis)
//! pinned at `v2026.5.20` (Apache-2.0). cave-agent ports the *control-plane*
//! surface of OpenJarvis to dependency-light Rust:
//!
//! ## OpenJarvis primitives (composable on-device patterns)
//! - [`tool`] — a self-registering tool registry with JSON-schema specs and
//!   pure built-in tools (calculator, string ops, clock).
//! - [`memory`] — windowed conversation memory with budget-aware eviction and
//!   keyword recall.
//! - [`plan`] — a plan-and-execute loop: heuristic decomposition, a step state
//!   machine, and replan-on-failure.
//! - [`eval`] — evaluation tools that score a run on energy, latency, cost, and
//!   accuracy and aggregate fleets of runs.
//! - [`compose`] — composable execution patterns (chain / parallel / fallback /
//!   retry) over the tool registry.
//!
//! ## Self-improvement loop (ADR-SELF-IMPROVE-001)
//! - [`observe`] — observability ingestion + percentile / regression analysis.
//! - [`tune`] — a deterministic self-tuning policy that proposes knob changes
//!   from observed metrics.
//! - [`changelog`] — upstream changelog watch: semver compare + entry
//!   classification + actionable diff.
//! - [`hotpatch`] — hot-patch ingestion: checksum-validated staging, apply, and
//!   rollback against a config registry.
//!
//! The HTTP surface ([`router`]) mounts `/api/agent/*` JSON endpoints and a
//! self-contained `/admin/agent` operator page.

pub mod error;
pub mod tool;
pub mod memory;
pub mod plan;
pub mod eval;
pub mod compose;
pub mod observe;
pub mod tune;
pub mod changelog;
pub mod hotpatch;
pub mod router;

pub use error::{AgentError, Result};
pub use router::{router, AgentState};

/// Upstream OpenJarvis version this crate is pinned to. Asserted by the
/// Charter v2 self-audit against the manifest.
pub const UPSTREAM_VERSION: &str = "v2026.5.20";

/// An assembled agent runtime: the tool registry, the current tunable knobs,
/// and the hot-patchable config registry. Constructed by [`default_runtime`].
pub struct Runtime {
    /// The composable tool registry (pre-loaded with the pure built-ins).
    pub tools: tool::ToolRegistry,
    /// The current self-tuning knobs.
    pub knobs: tune::Knobs,
    /// The hot-patchable config registry.
    pub config: hotpatch::PatchRegistry,
}

/// The default on-device runtime: built-in tools, sane starting knobs, and an
/// empty config registry.
pub fn default_runtime() -> Runtime {
    Runtime {
        tools: tool::builtins(),
        knobs: tune::Knobs {
            max_tokens: 2048,
            temperature: 0.7,
            concurrency: 8,
        },
        config: hotpatch::PatchRegistry::new(),
    }
}
