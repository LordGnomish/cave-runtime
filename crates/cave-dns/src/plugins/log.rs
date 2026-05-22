// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Log plugin — query access logging.
use async_trait::async_trait;
use tracing::info;

use crate::{
    config::LogConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

pub struct LogPlugin {
    config: LogConfig,
}

impl LogPlugin {
    pub fn new(config: LogConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Plugin for LogPlugin {
    fn name(&self) -> &str {
        "log"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        next.run(ctx).await?;

        let q = ctx.request.queries().first();
        let qname = q.map(|q| q.name().to_string()).unwrap_or_default();
        let qtype = q.map(|q| q.query_type().to_string()).unwrap_or_default();
        let qclass = q.map(|q| q.query_class().to_string()).unwrap_or_default();
        let rcode = ctx.response.response_code();
        let latency_ms = ctx.elapsed_ms();
        let client = ctx.client_addr;
        let proto = format!("{:?}", ctx.proto).to_lowercase();

        info!(
            client = %client,
            name   = %qname,
            type   = %qtype,
            class  = %qclass,
            proto  = %proto,
            rcode  = ?rcode,
            latency_ms,
            "dns query"
        );
        Ok(())
    }
}
