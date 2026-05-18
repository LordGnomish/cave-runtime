// SPDX-License-Identifier: AGPL-3.0-or-later
/// Whoami plugin — respond with the client's IP address.
use std::net::IpAddr;

use async_trait::async_trait;
use hickory_proto::rr::{DNSClass, RData, Record, RecordType};
use tracing::debug;

use crate::{
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

pub struct WhoamiPlugin;

impl WhoamiPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WhoamiPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for WhoamiPlugin {
    fn name(&self) -> &str {
        "whoami"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return next.run(ctx).await,
        };

        let client_ip = ctx.client_addr.ip();
        debug!(client = %client_ip, "whoami");

        let (rdata, rtype) = match client_ip {
            IpAddr::V4(v4) => (
                RData::A(hickory_proto::rr::rdata::A(v4)),
                RecordType::A,
            ),
            IpAddr::V6(v6) => (
                RData::AAAA(hickory_proto::rr::rdata::AAAA(v6)),
                RecordType::AAAA,
            ),
        };

        // Only respond when query type matches what we can return
        if q.query_type() == rtype || q.query_type() == RecordType::ANY {
            let mut r = Record::new();
            r.set_name(q.name().clone());
            r.set_ttl(0); // whoami is non-cacheable
            r.set_record_type(rtype);
            r.set_dns_class(DNSClass::IN);
            r.set_data(Some(rdata));
            ctx.response.add_answer(r);
            return Ok(());
        }

        next.run(ctx).await
    }
}
