/// Loop plugin — detect forwarding loops.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use hickory_proto::{op::ResponseCode, rr::{Name, RecordType}};
use tracing::warn;

use crate::{
    config::LoopConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
    protocol::message::make_error_response,
};

type InFlightKey = (Name, RecordType, SocketAddr);

pub struct LoopPlugin {
    config: LoopConfig,
    in_flight: Arc<DashMap<InFlightKey, Instant>>,
}

impl LoopPlugin {
    pub fn new(config: LoopConfig) -> Self {
        Self {
            config,
            in_flight: Arc::new(DashMap::new()),
        }
    }
}

#[async_trait]
impl Plugin for LoopPlugin {
    fn name(&self) -> &str {
        "loop"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return next.run(ctx).await,
        };

        let key: InFlightKey = (q.name().clone(), q.query_type(), ctx.client_addr);
        let timeout = Duration::from_millis(self.config.timeout_ms);
        let now = Instant::now();

        // Expire stale entries first
        self.in_flight
            .retain(|_, t| now.duration_since(*t) < timeout);

        if self.in_flight.contains_key(&key) {
            warn!(name = %q.name(), client = %ctx.client_addr, "loop detected");
            ctx.response = make_error_response(&ctx.request, ResponseCode::ServFail);
            return Ok(());
        }

        self.in_flight.insert(key.clone(), now);
        let result = next.run(ctx).await;
        self.in_flight.remove(&key);
        result
    }
}
