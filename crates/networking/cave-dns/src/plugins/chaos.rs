// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Chaos plugin — version.bind and hostname.bind (CHAOS class).
use async_trait::async_trait;
use hickory_proto::rr::{DNSClass, RData, Record, RecordType};
use tracing::debug;

use crate::{
    config::ChaosConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

pub struct ChaosPlugin {
    config: ChaosConfig,
}

impl ChaosPlugin {
    pub fn new(config: ChaosConfig) -> Self {
        Self { config }
    }

    fn txt_reply(ctx: &mut QueryContext, text: &str) {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return,
        };
        let mut r = Record::new();
        r.set_name(q.name().clone());
        r.set_ttl(0);
        r.set_record_type(RecordType::TXT);
        r.set_dns_class(DNSClass::CH);
        r.set_data(Some(RData::TXT(hickory_proto::rr::rdata::TXT::new(vec![
            text.to_string(),
        ]))));
        ctx.response.add_answer(r);
    }
}

#[async_trait]
impl Plugin for ChaosPlugin {
    fn name(&self) -> &str {
        "chaos"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return next.run(ctx).await,
        };

        // Only intercept CHAOS class queries
        if q.query_class() != DNSClass::CH {
            return next.run(ctx).await;
        }

        let name = q.name().to_string().to_lowercase();
        debug!(name = %name, "chaos query");

        match name.trim_end_matches('.') {
            "version.bind" | "version.server" => {
                Self::txt_reply(ctx, &self.config.version);
                Ok(())
            }
            "hostname.bind" | "id.server" => {
                Self::txt_reply(ctx, &self.config.hostname);
                Ok(())
            }
            _ => next.run(ctx).await,
        }
    }
}
