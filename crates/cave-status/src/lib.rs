// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Status page — auto-generated from cave-uptime probes
//!
//! Upstream tracking: custom
//! Features: Public/internal status page, auto-generation from probes, incident integration

pub mod routes;
pub mod models;
pub mod engine;

use axum::Router;

pub fn router() -> Router {
    routes::create_router()
}

pub const MODULE_NAME: &str = "status";
