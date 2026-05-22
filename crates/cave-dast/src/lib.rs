// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//! Dynamic application security testing — OWASP ZAP 2.14.0 parity port.
//!
//! Compatible with: OWASP ZAP (Apache-2.0 upstream).
//! Module tree mirrors `zap/src/main/java/org/zaproxy/zap/`:
//!
//! * [`http`]    — HTTP request/response model, header/cookie/url parsers
//! * [`context`] — Context (in-scope include/exclude regex)
//! * [`ascan`]   — Active scan plugin framework + 6 baseline rules
//! * [`pscan`]   — Passive scan plugin framework + 5 baseline rules
//! * [`spider`]  — BFS link discovery with robots.txt + depth limits
//! * [`auth`]    — Form-based and bearer-token authentication
//! * [`alert`]   — Alert/CWE/OWASP Top 10 taxonomy
//! * [`report`]  — HTML report renderer
//! * [`cli`]     — `zap-cli` compatible subcommand parser
//! * [`engine`]  — scan helpers (risk arithmetic)
//! * [`routes`]  — axum REST API surface

use std::sync::Arc;

pub mod alert;
pub mod ascan;
pub mod auth;
pub mod cli;
pub mod context;
pub mod engine;
pub mod extension;
pub mod http;
pub mod models;
pub mod pscan;
pub mod report;
pub mod routes;
pub mod spider;

use axum::Router;

/// Module state.
#[derive(Default)]
pub struct State {}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "dast";

/// Upstream tag pinned for parity port.
pub const UPSTREAM_VERSION: &str = "v2.14.0";
pub const UPSTREAM_SHA: &str = "v2.14.0";
