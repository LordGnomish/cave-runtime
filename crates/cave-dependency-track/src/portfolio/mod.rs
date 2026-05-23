// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Portfolio — project CRUD + hierarchy + tags.

pub mod hierarchy;
pub mod store;
pub mod tags;

pub use hierarchy::{ProjectNode, build_tree, descendants};
pub use store::{PortfolioStore, ProjectUpdate};
pub use tags::{TagIndex, normalize_tag};
