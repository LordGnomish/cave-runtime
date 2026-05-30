// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DNS64 plugin — synthesize AAAA from A records (RFC 6147 / RFC 6052).
//!
//! Line-by-line port of CoreDNS v1.14.3 `plugin/dns64/dns64.go`: the `to6`
//! address embedding, `requestShouldIntercept` / `responseShouldDNS64`
//! predicates, and `Synthesize`. The default well-known prefix is
//! `64:ff9b::/96`.
use std::net::{Ipv4Addr, Ipv6Addr};

use async_trait::async_trait;
use hickory_proto::{
    op::ResponseCode,
    rr::{RData, RecordType},
};

use crate::{
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
    protocol::message::aaaa_record,
};

const TTL: u32 = 600;

pub struct Dns64Plugin {
    prefix: Ipv6Addr,
    prefix_len: u8,
    allow_ipv4: bool,
}

impl Dns64Plugin {
    pub fn new(prefix: Ipv6Addr, prefix_len: u8, allow_ipv4: bool) -> Self {
        Self {
            prefix,
            prefix_len,
            allow_ipv4,
        }
    }

    /// `dns64.go` to6(): embed an IPv4 address into the configured prefix per
    /// RFC 6052 (skipping the reserved `u` octet at index 8 for /40../88).
    pub fn to6(&self, v4: Ipv4Addr) -> Ipv6Addr {
        let prefix = self.prefix.octets();
        let addr = v4.octets();
        let n = (self.prefix_len / 8) as usize;
        let mut v6 = [0u8; 16];

        let mut i = 0usize;
        while i < n {
            v6[i] = prefix[i];
            i += 1;
        }
        let mut j = 0usize;
        while i < 8 {
            v6[i] = addr[j];
            i += 1;
            j += 1;
        }
        if i == 8 {
            i += 1;
        }
        while j < 4 {
            v6[i] = addr[j];
            i += 1;
            j += 1;
        }
        Ipv6Addr::from(v6)
    }

    /// `dns64.go` requestShouldIntercept(): AAAA/IN queries that arrived over
    /// IPv6 (or IPv4 when `allow_ipv4` is set).
    pub fn request_should_intercept(
        qtype: RecordType,
        client_is_ipv4: bool,
        allow_ipv4: bool,
    ) -> bool {
        if !allow_ipv4 && client_is_ipv4 {
            return false;
        }
        qtype == RecordType::AAAA
    }

    /// `dns64.go` responseShouldDNS64(): translate when the response is not a
    /// NameError and carries no AAAA records.
    pub fn response_should_dns64(is_name_error: bool, has_aaaa: bool) -> bool {
        if is_name_error {
            return false;
        }
        !has_aaaa
    }

    /// `dns64.go` Synthesize(): map every A answer through `to6`.
    pub fn synthesize_aaaa(&self, v4s: &[Ipv4Addr]) -> Vec<Ipv6Addr> {
        v4s.iter().map(|v4| self.to6(*v4)).collect()
    }
}

impl Default for Dns64Plugin {
    fn default() -> Self {
        // 64:ff9b::/96 — RFC 6052 well-known prefix.
        Self::new("64:ff9b::".parse().unwrap(), 96, false)
    }
}

#[async_trait]
impl Plugin for Dns64Plugin {
    fn name(&self) -> &str {
        "dns64"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let qtype = ctx.request.queries().first().map(|q| q.query_type());
        let client_is_ipv4 = ctx.client_addr.is_ipv4();
        let intercept = qtype
            .map(|t| Self::request_should_intercept(t, client_is_ipv4, self.allow_ipv4))
            .unwrap_or(false);

        next.run(ctx).await?;

        if !intercept {
            return Ok(());
        }

        let is_name_error = ctx.response.response_code() == ResponseCode::NXDomain;
        let has_aaaa = ctx
            .response
            .answers()
            .iter()
            .any(|r| r.record_type() == RecordType::AAAA);
        if !Self::response_should_dns64(is_name_error, has_aaaa) {
            return Ok(());
        }

        // Synthesize AAAA from any A answers present in the response.
        let synthesized: Vec<(String, Ipv6Addr)> = ctx
            .response
            .answers()
            .iter()
            .filter_map(|r| match r.data() {
                Some(RData::A(a)) => Some((r.name().to_string(), self.to6(a.0))),
                _ => None,
            })
            .collect();
        for (name, v6) in synthesized {
            if let Ok(rec) = aaaa_record(&name, TTL, v6) {
                ctx.response.add_answer(rec);
            }
        }
        Ok(())
    }
}
