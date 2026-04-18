//! CAVE Hubble — eBPF network flow observation, Cilium Hubble-compatible API.

pub mod aggregator;
pub mod dns;
pub mod error;
pub mod filter;
pub mod flow;
pub mod models;
pub mod routes;

use axum::Router;
use std::sync::Arc;

pub use error::{HubbleError, HubbleResult};

pub const MODULE_NAME: &str = "hubble";

pub struct HubbleState {
    pub flows: Arc<flow::FlowStore>,
    pub dns: Arc<dns::DnsCache>,
    pub aggregator: Arc<aggregator::FlowAggregator>,
}

impl Default for HubbleState {
    fn default() -> Self {
        Self {
            flows: Arc::new(flow::FlowStore::new()),
            dns: Arc::new(dns::DnsCache::new()),
            aggregator: Arc::new(aggregator::FlowAggregator::new()),
        }
    }
}

pub fn router(state: Arc<HubbleState>) -> Router {
    routes::create_router(state)
}
