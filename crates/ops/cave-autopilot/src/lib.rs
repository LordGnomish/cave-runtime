// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! # cave-autopilot
//!
//! A 7/24 autonomous code-generation orchestrator for the Cave Runtime
//! monorepo. It closes the loop Burak asked for three months ago — *"lokal
//! llm ile bir workflow çıkar güzel 7/24 hiç durmadan çalışın"* — by turning
//! tracker state into a never-ending stream of Charter-compliant TDD port
//! jobs.
//!
//! ## Pipeline
//!
//! ```text
//!   parity-index.json (tracker state)
//!         │  tracker::TrackerState::load
//!         ▼
//!   ranked Vec<Subsystem>  ── queue::TaskQueue ──►  Task (one under-complete crate)
//!         │
//!         ▼  escalation::EscalationLadder
//!   L1 route (Mellum2) → L2 code (Qwen3-Coder-Next) → L3 Claude API → L4 human
//!         │
//!         ▼  worktree::WorktreeJob
//!   git worktree add → cargo build → cargo test → charter::audit → commit → merge (no-ff)
//!         │
//!         ▼
//!   metrics (:9101) + daily report (docs/audit/autopilot-daily-*.md)
//! ```
//!
//! Every sub-module is independently unit-testable; the [`daemon`] module only
//! wires them together on a tokio scheduler. Network-touching modules
//! ([`ollama`], [`claude`]) keep request *construction* pure and testable,
//! isolating the actual HTTP call behind a thin async method.

pub mod charter;
pub mod claude;
pub mod codegen;
pub mod config;
pub mod daemon;
pub mod error;
pub mod escalation;
pub mod executor;
pub mod launchagent;
pub mod metrics;
pub mod ollama;
pub mod queue;
pub mod report;
pub mod router;
pub mod tracker;
pub mod worktree;

pub use error::{AutopilotError, Result};
