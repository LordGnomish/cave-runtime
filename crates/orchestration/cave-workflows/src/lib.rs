// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-workflows — Argo Workflows parity.
//!
//! Mirrors `argoproj/argo-workflows v4.0.5`:
//! * Workflow + WorkflowSpec + Template CRDs
//! * Six template variants (Container / Script / Resource / Suspend / DAG / Steps)
//! * Parameter + Artifact pipelines (7 first-party repositories: S3 / GCS /
//!   HTTP / Git / OSS / Raw / HDFS)
//! * RetryStrategy with Always / OnFailure / OnError / OnTransientError + backoff
//! * Pure-function executor — emits NodeAction (Schedule / Retry / Suspend /
//!   Complete) the downstream `cave-cri` runtime applies.
//! * In-memory CRUD store + axum HTTP control plane.

use std::sync::Arc;

pub mod conditions;
pub mod cron;
pub mod engine;
pub mod events;
pub mod executor;
pub mod models;
pub mod persistence;
pub mod routes;
pub mod store;
pub mod sync;
pub mod workflow_crd;

use axum::Router;

/// Module state.
#[derive(Default)]
pub struct State {}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "workflows";
