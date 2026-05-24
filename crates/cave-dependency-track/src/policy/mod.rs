// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Policy engine.
//!
//! Mirrors `org.dependencytrack.policy.PolicyEngine` and the family of
//! `*PolicyEvaluator` classes.

pub mod age;
pub mod coordinates;
pub mod engine;
pub mod license;
pub mod store;
pub mod vulnerability;

pub use age::evaluate_age;
pub use coordinates::evaluate_coordinates;
pub use engine::{Policy, PolicyCondition, PolicyOperator, PolicyResult, Subject, ViolationKind};
pub use license::evaluate_license;
pub use store::PolicyStore;
pub use vulnerability::evaluate_vulnerability;
