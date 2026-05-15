// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Release intelligence — git + SBOM diff based release notes
//!
//! Upstream tracking: custom
//! Features: Auto-generated changelogs from git commits + SBOM diffs per deployment

pub mod routes;
pub mod models;
pub mod engine;

use axum::Router;

pub fn router() -> Router {
    routes::create_router()
}

pub const MODULE_NAME: &str = "changelog";
