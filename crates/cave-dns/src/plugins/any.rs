// SPDX-License-Identifier: AGPL-3.0-or-later
/// Any plugin — handle or block ANY queries (RFC 8482).
use async_trait::async_trait;
use hickory_proto::{
    op::ResponseCode,
    rr::{DNSClass, RData, Record, RecordType},
};

use crate::{
    config::{AnyConfig, AnyResponse},
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
    protocol::message::make_error_response,
};

pub struct AnyPlugin {
    config: AnyConfig,
}

impl AnyPlugin {
    pub fn new(config: AnyConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Plugin for AnyPlugin {
    fn name(&self) -> &str {
        "any"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let is_any = ctx
            .request
            .queries()
            .first()
            .map(|q| q.query_type() == RecordType::ANY)
            .unwrap_or(false);

        if !is_any {
            return next.run(ctx).await;
        }

        match self.config.response {
            AnyResponse::All => next.run(ctx).await,

            AnyResponse::Refuse => {
                ctx.response = make_error_response(&ctx.request, ResponseCode::Refused);
                Ok(())
            }

            AnyResponse::Minimal => {
                // RFC 8482: return HINFO "RFC8482" ""
                let q = ctx.request.queries().first().cloned().unwrap();
                let mut r = Record::new();
                r.set_name(q.name().clone());
                r.set_ttl(3600);
                r.set_record_type(RecordType::HINFO);
                r.set_dns_class(DNSClass::IN);
                r.set_data(Some(RData::HINFO(hickory_proto::rr::rdata::HINFO::new(
                    "RFC8482".to_string(),
                    String::new(),
                ))));
                ctx.response.add_answer(r);
                Ok(())
            }
        }
    }
}
