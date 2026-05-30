// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Local plugin — canned replies for loopback names.
//!
//! Line-by-line port of CoreDNS v1.14.3 `plugin/local/local.go`. Answers
//! `localhost.` (and sub-domains) with `127.0.0.1` / `::1`, and intercepts the
//! loopback reverse zones (`0/127/255.in-addr.arpa.`).
use std::net::{Ipv4Addr, Ipv6Addr};

use async_trait::async_trait;
use hickory_proto::rr::RecordType;

use crate::{
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
    protocol::message::{a_record, aaaa_record},
};

/// `local.go` const ttl = 604800.
const TTL: u32 = 604_800;

/// The static zones local intercepts (`local.go` var zones).
const ZONES: [&str; 4] = [
    "localhost.",
    "0.in-addr.arpa.",
    "127.in-addr.arpa.",
    "255.in-addr.arpa.",
];

pub struct LocalPlugin;

impl LocalPlugin {
    pub fn new() -> Self {
        Self
    }

    /// `local.go` ServeDNS(): a name belongs to `local` when it is (or sits
    /// under) `localhost.` or one of the loopback reverse zones.
    pub fn is_local_name(name: &str) -> bool {
        // `localhost.` and any `localhost.<domain>` / `<label>.localhost.`.
        if name.starts_with("localhost.") || name.ends_with(".localhost.") {
            return true;
        }
        for zone in ZONES {
            if name == zone || name.ends_with(&format!(".{zone}")) {
                return true;
            }
        }
        false
    }
}

impl Default for LocalPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for LocalPlugin {
    fn name(&self) -> &str {
        "local"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let Some(query) = ctx.request.queries().first().cloned() else {
            return next.run(ctx).await;
        };
        let qname = query.name().to_string();
        if !Self::is_local_name(&qname) {
            return next.run(ctx).await;
        }

        ctx.response.set_authoritative(true);
        match query.query_type() {
            RecordType::A if qname.starts_with("localhost") => {
                if let Ok(rec) = a_record(&qname, TTL, Ipv4Addr::new(127, 0, 0, 1)) {
                    ctx.response.add_answer(rec);
                }
            }
            RecordType::AAAA if qname.starts_with("localhost") => {
                if let Ok(rec) = aaaa_record(&qname, TTL, Ipv6Addr::LOCALHOST) {
                    ctx.response.add_answer(rec);
                }
            }
            _ => {
                // NODATA — authoritative empty answer for the loopback zones.
            }
        }
        Ok(())
    }
}
