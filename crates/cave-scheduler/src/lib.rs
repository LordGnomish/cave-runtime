//! cave-scheduler — Pod scheduler.
//!
//! Architecture: kube-scheduler v1.31 framework parity. Pods flow through 13
//! extension points: QueueSort → PreEnqueue → PreFilter → Filter → PostFilter
//! → PreScore → Score → NormalizeScore → Reserve → Permit → PreBind → Bind →
//! PostBind. Each step is implemented as one or more plugins; profiles
//! configure which plugins are enabled per scheduler name.

pub mod bind;
pub mod cycle_state;
pub mod default_preemption;
pub mod dra;
pub mod dra_scheduler;
pub mod events;
pub mod extender;
pub mod extension_points;
pub mod framework;
pub mod gates;
pub mod models;
pub mod noderesources;
pub mod plugins;
pub mod preempt;
pub mod priority_queue;
pub mod priority_sort;
pub mod profiles;
pub mod routes;
pub mod scheduler;
pub mod topology;
pub mod volume;

use scheduler::SchedulerState;
use std::sync::Arc;

pub fn new_state() -> Arc<SchedulerState> {
    Arc::new(SchedulerState::new())
}

pub fn router(state: Arc<SchedulerState>) -> axum::Router {
    routes::create_router(state)
}
