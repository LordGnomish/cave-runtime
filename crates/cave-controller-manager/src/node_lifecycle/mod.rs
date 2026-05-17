// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Node lifecycle controller — `pkg/controller/nodelifecycle`.
//!
//! Sub-modules:
//!
//! * [`taints`] — taint set arithmetic + standard NoExecute/NoSchedule
//!   constants used by the controller (`unreachable`, `not-ready`,
//!   `out-of-service`, `unschedulable`).
//! * [`zone_state`] — derives a zone's overall health (Normal /
//!   PartialDisruption / FullDisruption) from per-node Ready states.
//! * [`evictor`] — rate-limited per-zone eviction queue with its own
//!   secondary-evictor for catastrophic disruptions.

pub mod evictor;
pub mod taints;
pub mod zone_state;
