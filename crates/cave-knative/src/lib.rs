//! CAVE Knative — serverless workloads.
//!
//! Compatible with: knative/serving, knative/eventing
//!
//! Features:
//! - Knative Serving: Service, Revision, Route, scale-to-zero, traffic splitting
//! - Knative Eventing: Broker, Trigger, EventSource, Channel, Subscription
//! - CloudEvents 1.0 spec ingestion and fan-out
//! - Simulated autoscaler (KPA) — scale-to-zero, scale-up on request

pub mod error;
pub mod eventing;
pub mod models;
pub mod routes;
pub mod serving;

use axum::Router;
use std::sync::Arc;

pub use error::{KnativeError, KnativeResult};
pub use eventing::EventingStore;
pub use serving::ServingStore;

pub const MODULE_NAME: &str = "knative";

/// Shared state for the cave-knative module.
pub struct KnativeState {
    pub serving: Arc<ServingStore>,
    pub eventing: Arc<EventingStore>,
}

impl Default for KnativeState {
    fn default() -> Self {
        Self {
            serving: Arc::new(ServingStore::new()),
            eventing: Arc::new(EventingStore::new()),
        }
    }
}

/// Build the Axum router for the Knative API.
pub fn router(state: Arc<KnativeState>) -> Router {
    routes::create_router(state)
}
