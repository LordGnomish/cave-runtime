//! CAVE KEDA — event-driven autoscaling.

pub mod cooldown;
pub mod error;
pub mod models;
pub mod routes;
pub mod scaler;
pub mod trigger;

use axum::Router;
use std::sync::Arc;

pub use error::{KedaError, KedaResult};

pub const MODULE_NAME: &str = "keda";

pub struct KedaState {
    pub scaled_objects: Arc<scaler::ScaledObjectStore>,
    pub scaled_jobs: Arc<scaler::ScaledJobStore>,
    pub trigger_auths: Arc<trigger::TriggerAuthStore>,
    pub cooldown: Arc<cooldown::CooldownTracker>,
}

impl Default for KedaState {
    fn default() -> Self {
        Self {
            scaled_objects: Arc::new(scaler::ScaledObjectStore::new()),
            scaled_jobs: Arc::new(scaler::ScaledJobStore::new()),
            trigger_auths: Arc::new(trigger::TriggerAuthStore::new()),
            cooldown: Arc::new(cooldown::CooldownTracker::new()),
        }
    }
}

pub fn router(state: Arc<KedaState>) -> Router {
    routes::create_router(state)
}
