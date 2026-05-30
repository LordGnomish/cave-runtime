// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
pub mod acl;
pub mod any;
pub mod auto;
pub mod bufsize;
pub mod cache;
pub mod chaos;
pub mod errors;
pub mod etcd;
pub mod file;
pub mod forward;
pub mod header;
pub mod health;
pub mod hosts;
pub mod kubernetes;
pub mod loadbalance;
pub mod log;
pub mod loop_detect;
pub mod metrics;
pub mod minimal;
pub mod prometheus;
pub mod ready;
pub mod reload;
pub mod rewrite;
pub mod root;
pub mod route53;
pub mod secondary;
pub mod template;
pub mod tls;
pub mod trace;
pub mod whoami;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use hickory_proto::op::Message;

use crate::{error::DnsResult, protocol::edns::EdnsOptions};

// ─── Transport protocol enum ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Udp,
    Tcp,
    Dot,
    Doh,
}

// ─── Query context (shared between plugins) ──────────────────────────────────

pub struct QueryContext {
    pub request: Message,
    pub response: Message,
    pub client_addr: SocketAddr,
    pub proto: Protocol,
    pub edns: Option<EdnsOptions>,
    pub dnssec_ok: bool,
    pub start_time: Instant,
}

impl QueryContext {
    pub fn new(request: Message, client_addr: SocketAddr, proto: Protocol) -> Self {
        let edns = EdnsOptions::from_message(&request);
        let dnssec_ok = edns.dnssec_ok;
        let response = crate::protocol::message::make_response(&request);
        Self {
            request,
            response,
            client_addr,
            proto,
            edns: Some(edns),
            dnssec_ok,
            start_time: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }
}

// ─── Plugin trait ────────────────────────────────────────────────────────────

/// A DNS plugin that can inspect and mutate a QueryContext.
///
/// Plugins are chained: calling `next.run(ctx)` passes control to the next
/// plugin in the chain.  A plugin that does NOT call `next.run()` short-
/// circuits the chain (e.g. the cache plugin on a cache hit).
#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()>;

    /// Called once after all plugins are constructed; plugins can use this
    /// to spawn background tasks.
    async fn ready(&self) -> DnsResult<()> {
        Ok(())
    }
}

// ─── Chain-continuation handle ───────────────────────────────────────────────

pub struct Next<'a> {
    plugins: &'a [Arc<dyn Plugin>],
}

impl<'a> Next<'a> {
    pub async fn run(self, ctx: &mut QueryContext) -> DnsResult<()> {
        match self.plugins.split_first() {
            None => Ok(()),
            Some((first, rest)) => first.handle(ctx, Next { plugins: rest }).await,
        }
    }
}

// ─── Plugin chain ────────────────────────────────────────────────────────────

pub struct PluginChain {
    plugins: Vec<Arc<dyn Plugin>>,
}

impl PluginChain {
    pub fn new(plugins: Vec<Arc<dyn Plugin>>) -> Self {
        Self { plugins }
    }

    pub async fn execute(&self, ctx: &mut QueryContext) -> DnsResult<()> {
        let next = Next {
            plugins: &self.plugins,
        };
        next.run(ctx).await
    }

    pub async fn ready_all(&self) -> DnsResult<()> {
        for plugin in &self.plugins {
            plugin.ready().await?;
        }
        Ok(())
    }
}
