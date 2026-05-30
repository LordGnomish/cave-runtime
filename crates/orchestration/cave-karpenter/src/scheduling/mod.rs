// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Karpenter scheduling primitives — line-by-line port of
//! `pkg/scheduling` from kubernetes-sigs/karpenter v1.12.1.
//!
//! Modules:
//!   requirement   — complement-based Requirement set-algebra with bounds

pub mod hostport;
pub mod requirement;
pub mod requirements;

pub use requirement::{NodeSelectorRequirement, Operator, Requirement};
pub use requirements::Requirements;
