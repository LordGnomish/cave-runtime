/// Errors plugin — log DNS errors (SERVFAIL etc.).
use async_trait::async_trait;
use hickory_proto::op::ResponseCode;
use tracing::error;

use crate::{
    config::ErrorsConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

pub struct ErrorsPlugin {
    config: ErrorsConfig,
}

impl ErrorsPlugin {
    pub fn new(config: ErrorsConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Plugin for ErrorsPlugin {
    fn name(&self) -> &str {
        "errors"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        next.run(ctx).await?;

        let rcode = ctx.response.response_code();
        if rcode == ResponseCode::ServFail || rcode == ResponseCode::FormErr {
            let qname = ctx
                .request
                .queries()
                .first()
                .map(|q| q.name().to_string())
                .unwrap_or_default();
            error!(
                rcode = ?rcode,
                name = %qname,
                client = %ctx.client_addr,
                "DNS error"
            );
        }
        Ok(())
    }
}
