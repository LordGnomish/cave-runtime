//! cave-scheduler — Pod scheduler.
//!
//! Assigns pods to nodes based on:
//! - Resource availability (CPU, memory, pod count)
//! - Node selectors (label matching)
//! - Taints and tolerations
//! - Affinity preferences
//!
//! Algorithm: Filter → Score (least-allocated + affinity bonus) → Bind

pub mod models;
pub mod scheduler;
pub mod routes;

use scheduler::SchedulerState;
use std::sync::Arc;

pub fn new_state() -> Arc<SchedulerState> {
    Arc::new(SchedulerState::new())
}

pub fn router(state: Arc<SchedulerState>) -> axum::Router {
    routes::create_router(state)
}

/// Calculate parity against the local source tree at compile-time crate root.
pub fn calculate_parity() -> Result<cave_kernel::parity::ParityReport, String> {
    cave_kernel::parity::calculate_from_str(
        include_str!("../parity.manifest.toml"),
        env!("CARGO_MANIFEST_DIR"),
    )
    .map_err(|e| e.to_string())
}
