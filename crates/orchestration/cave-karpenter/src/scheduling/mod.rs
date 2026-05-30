// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Karpenter scheduling primitives — line-by-line port of
//! `pkg/scheduling` from kubernetes-sigs/karpenter v1.12.1.
//!
//! Modules:
//!   requirement   — complement-based Requirement set-algebra with bounds
//!   requirements  — keyed Requirement collection + compatibility
//!   hostport      — HostPort reservation/conflict tracking
//!   taints        — taint/toleration matching + KnownEphemeralTaints
//!   volumeusage   — per-node CSI volume-limit tracking (Volumes/VolumeUsage)

pub mod hostport;
pub mod requirement;
pub mod requirements;
pub mod taints;
pub mod volumeusage;

pub use requirement::{NodeSelectorRequirement, Operator, Requirement};
pub use requirements::Requirements;
