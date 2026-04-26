//! CAVE SPIRE — SPIFFE/SPIRE workload identity.

pub mod agent;
pub mod error;
pub mod federation;
pub mod models;
pub mod routes;
pub mod svid;
pub mod trust;

use axum::Router;
use std::sync::Arc;

pub use error::{SpireError, SpireResult};

pub const MODULE_NAME: &str = "spire";

pub struct SpireState {
    pub trust_domains: Arc<trust::TrustDomainStore>,
    pub registrations: Arc<svid::RegistrationStore>,
    pub svids: Arc<svid::SvidStore>,
    pub agents: Arc<agent::AgentStore>,
    pub federation: Arc<federation::FederationStore>,
}

impl Default for SpireState {
    fn default() -> Self {
        Self {
            trust_domains: Arc::new(trust::TrustDomainStore::new()),
            registrations: Arc::new(svid::RegistrationStore::new()),
            svids: Arc::new(svid::SvidStore::new()),
            agents: Arc::new(agent::AgentStore::new()),
            federation: Arc::new(federation::FederationStore::new()),
        }
    }
}

pub fn router(state: Arc<SpireState>) -> Router {
    routes::create_router(state)
}
