// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OpenJarvis primitives — local-first, on-device personal agent layer.
//!
//! Ports the composable on-device agent patterns of [open-jarvis/OpenJarvis]
//! (Stanford SAIL / Hazy Research, Apache-2.0) into cave-hermes as a
//! *local-first* sub-layer per ADR-RUNTIME-OPENJARVIS-ADOPTION-001.
//!
//! Where the Hermes parent surface ([`crate::planner`], [`crate::tool`],
//! [`crate::memory`], [`crate::router`]) is positioned for *enterprise*
//! server-side orchestration, this sub-layer is positioned for *personal,
//! local-first* on-device agents: no cloud dependency by default, persistent
//! per-agent state on the local filesystem, and backend selection driven by
//! measured energy / latency / cost / accuracy.
//!
//! [open-jarvis/OpenJarvis]: https://github.com/open-jarvis/OpenJarvis
//!
//! ## Layout
//!
//! | upstream concept                  | module                          |
//! |-----------------------------------|---------------------------------|
//! | backend orchestration glue        | [`backend`]                     |
//! | evaluation harness (energy/cost…) | [`eval`]                        |
//! | persistent agent state            | [`agent_state`]                 |
//! | plan-and-execute loop             | [`plan_execute`]                |
//! | inter-agent communication         | [`multi_agent`]                 |

pub mod backend;
pub mod eval;

/// Upstream pin for the OpenJarvis primitive set. Tracked alongside the
/// Hermes upstream pin in [`crate::UPSTREAM_VERSION`]; cave-hermes is a
/// dual-upstream crate (Hermes MIT + OpenJarvis Apache-2.0).
pub const OPENJARVIS_UPSTREAM_VERSION: &str = "v2026.5.20";
pub const OPENJARVIS_UPSTREAM_REPO: &str = "https://github.com/open-jarvis/OpenJarvis";
