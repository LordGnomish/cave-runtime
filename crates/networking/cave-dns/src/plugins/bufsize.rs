// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Bufsize plugin — clamp the requester's EDNS0 UDP buffer size.
//!
//! Line-by-line port of CoreDNS v1.14.3 `plugin/bufsize/bufsize.go` +
//! `plugin/bufsize/setup.go`. RFC 6891 bounds the value to `[512, 4096]`;
//! the default of 1232 follows <https://dnsflagday.net/2020/>.
use async_trait::async_trait;

use crate::{
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

/// Default EDNS buffer size — dnsflagday.net 2020 recommendation.
const DEFAULT_BUFSIZE: u16 = 1232;

pub struct BufsizePlugin {
    size: u16,
}

impl BufsizePlugin {
    pub fn new(size: u16) -> Self {
        Self { size }
    }

    /// `setup.go`: `const defaultBufSize = 1232`.
    pub fn default_size() -> u16 {
        DEFAULT_BUFSIZE
    }

    /// `setup.go` parse(): RFC 6891 valid range is `[512, 4096]`.
    pub fn validate(size: u16) -> DnsResult<u16> {
        if !(512..=4096).contains(&size) {
            return Err(DnsError::Config(format!(
                "bufsize {size} out of RFC 6891 range [512, 4096]"
            )));
        }
        Ok(size)
    }

    /// `bufsize.go`: clamp an advertised UDP size down to the configured max.
    /// `if int(option.UDPSize()) > buf.Size { option.SetUDPSize(buf.Size) }`
    pub fn clamp(&self, advertised: u16) -> u16 {
        if advertised > self.size {
            self.size
        } else {
            advertised
        }
    }
}

impl Default for BufsizePlugin {
    fn default() -> Self {
        Self::new(DEFAULT_BUFSIZE)
    }
}

#[async_trait]
impl Plugin for BufsizePlugin {
    fn name(&self) -> &str {
        "bufsize"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        // Clamp the request's advertised EDNS0 UDP payload size in place so
        // downstream plugins and the writer honour the reduced ceiling.
        if let Some(edns) = ctx.request.extensions_mut().as_mut() {
            let clamped = self.clamp(edns.max_payload());
            edns.set_max_payload(clamped);
        }
        // Mirror onto the decoded EdnsOptions view.
        if let Some(opts) = ctx.edns.as_mut() {
            opts.udp_payload_size = self.clamp(opts.udp_payload_size);
        }
        next.run(ctx).await
    }
}
