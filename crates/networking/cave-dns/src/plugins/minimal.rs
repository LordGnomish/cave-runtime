// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Minimal-responses plugin — strip authority + additional sections from
//! positive answers.
//!
//! Line-by-line port of CoreDNS v1.14.3 `plugin/minimal/minimal.go`.
//! Denial / error / delegation responses are passed through untouched; only
//! NOERROR responses that actually carry answers are minimized.
use async_trait::async_trait;
use hickory_proto::op::ResponseCode;

use crate::{
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
    protocol::message::make_response,
};

pub struct MinimalPlugin;

impl MinimalPlugin {
    pub fn new() -> Self {
        Self
    }

    /// `minimal.go`: only a positive NOERROR response *with answers* is
    /// minimized. Denial (NXDOMAIN / NODATA), error (SERVFAIL, …) and
    /// delegation responses are returned unchanged.
    pub fn should_minimize(rcode: ResponseCode, has_answers: bool) -> bool {
        rcode == ResponseCode::NoError && has_answers
    }
}

impl Default for MinimalPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for MinimalPlugin {
    fn name(&self) -> &str {
        "minimal"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        next.run(ctx).await?;

        let rcode = ctx.response.response_code();
        let has_answers = !ctx.response.answers().is_empty();
        if Self::should_minimize(rcode, has_answers) {
            // Rebuild the response keeping only header + queries + answers,
            // dropping the authority and additional sections.
            let mut minimal = make_response(&ctx.request);
            minimal.set_response_code(rcode);
            minimal.set_authoritative(ctx.response.authoritative());
            for answer in ctx.response.answers() {
                minimal.add_answer(answer.clone());
            }
            ctx.response = minimal;
        }
        Ok(())
    }
}
