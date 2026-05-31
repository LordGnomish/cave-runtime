// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Portal-facing handlers for cave-runtime.
//!
//! Provides the user-facing surface: persona login (Keycloak / dev mode),
//! upstream tracker (ADR-aware), ADR browser, and attribution dashboard.
//!
//! All handlers live in this module so the runtime binary stays focused on
//! wiring; each sub-module owns its routes and inline tests.

pub mod adr;
pub mod artifacts;
pub mod attribution;
pub mod auth;
pub mod llm_tracker;
pub mod upstream;

use axum::Router;

/// Build the combined portal router (auth + upstream + ADR + attribution +
/// artifacts + llm-tracker).
pub fn router() -> Router {
    Router::new()
        .merge(auth::router())
        .merge(upstream::router())
        .merge(adr::router())
        .merge(attribution::router())
        .merge(artifacts::router())
        .merge(llm_tracker::router())
}

/// Resolve the workspace root used by upstream/ADR/attribution handlers.
/// Honours `CAVE_WORKSPACE_ROOT`, falling back to the current directory.
pub fn workspace_root() -> std::path::PathBuf {
    std::env::var("CAVE_WORKSPACE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// Process-wide test-only mutex that the
/// `adr::tests` / `upstream::tests` / `attribution::tests` modules
/// all lock before mutating the `CAVE_WORKSPACE_ROOT` env var.
///
/// `cargo test` runs `#[tokio::test]`s in parallel; without this
/// guard one test's `set_var` races another's read, producing flaky
/// `assertion failed: v["total"].as_u64().unwrap() >= 3` panics.
/// The guard serialises only the env-var-mutating tests; the pure
/// unit tests above don't take it.
#[cfg(test)]
pub(crate) static WORKSPACE_ROOT_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
