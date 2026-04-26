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
pub mod framework;
pub mod plugins;
pub mod preempt;
pub mod priority_queue;
pub mod topology;
pub mod profiles;
pub mod dra;

use scheduler::SchedulerState;
use std::sync::Arc;

pub fn new_state() -> Arc<SchedulerState> {
    Arc::new(SchedulerState::new())
}

pub fn router(state: Arc<SchedulerState>) -> axum::Router {
    routes::create_router(state)
}
