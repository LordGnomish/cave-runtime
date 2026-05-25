// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Forward plugin — recursive resolution with multiple upstreams.
use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use hickory_proto::op::ResponseCode;
use hickory_resolver::{
    TokioAsyncResolver,
    config::{NameServerConfig, Protocol as RProto, ResolverConfig, ResolverOpts},
};
use tracing::{debug, warn};

use crate::{
    config::{ForwardConfig, ForwardPolicy},
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
    protocol::message::{encode_message, make_error_response, parse_message},
};

struct UpstreamState {
    addr: SocketAddr,
    healthy: AtomicBool,
    fail_count: AtomicUsize,
}

pub struct ForwardPlugin {
    config: ForwardConfig,
    upstreams: Vec<Arc<UpstreamState>>,
    round_robin_idx: AtomicUsize,
}

impl ForwardPlugin {
    pub fn new(config: ForwardConfig) -> DnsResult<Self> {
        let upstreams = config
            .upstreams
            .iter()
            .map(|addr| {
                let sa: SocketAddr = addr.parse().map_err(|e| {
                    DnsError::Config(format!("invalid upstream address {addr}: {e}"))
                })?;
                Ok(Arc::new(UpstreamState {
                    addr: sa,
                    healthy: AtomicBool::new(true),
                    fail_count: AtomicUsize::new(0),
                }))
            })
            .collect::<DnsResult<Vec<_>>>()?;

        Ok(Self {
            config,
            upstreams,
            round_robin_idx: AtomicUsize::new(0),
        })
    }

    fn select_upstream(&self) -> Option<Arc<UpstreamState>> {
        let healthy: Vec<_> = self
            .upstreams
            .iter()
            .filter(|u| u.healthy.load(Ordering::Relaxed))
            .cloned()
            .collect();

        if healthy.is_empty() {
            return self.upstreams.first().cloned(); // fallback: use all
        }

        match self.config.policy {
            ForwardPolicy::Sequential => healthy.into_iter().next(),
            ForwardPolicy::RoundRobin => {
                let idx = self.round_robin_idx.fetch_add(1, Ordering::Relaxed) % healthy.len();
                healthy.into_iter().nth(idx)
            }
            ForwardPolicy::Random => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let seed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() as usize)
                    .unwrap_or(0);
                let idx = seed % healthy.len();
                healthy.into_iter().nth(idx)
            }
        }
    }

    async fn forward_query(&self, ctx: &mut QueryContext) -> DnsResult<()> {
        let upstream = self
            .select_upstream()
            .ok_or_else(|| DnsError::Plugin("no upstreams available".into()))?;

        let query_bytes = encode_message(&ctx.request)?;
        let timeout = Duration::from_millis(self.config.timeout_ms);

        let result = tokio::time::timeout(timeout, async {
            use tokio::net::UdpSocket;
            let sock = UdpSocket::bind("0.0.0.0:0").await?;
            sock.send_to(&query_bytes, upstream.addr).await?;
            let mut buf = vec![0u8; 4096];
            let (n, _) = sock.recv_from(&mut buf).await?;
            buf.truncate(n);
            Ok::<Vec<u8>, std::io::Error>(buf)
        })
        .await;

        match result {
            Ok(Ok(resp_bytes)) => {
                upstream.fail_count.store(0, Ordering::Relaxed);
                upstream.healthy.store(true, Ordering::Relaxed);
                ctx.response = parse_message(&resp_bytes)?;
                Ok(())
            }
            Ok(Err(_)) | Err(_) => {
                let fails = upstream.fail_count.fetch_add(1, Ordering::Relaxed) + 1;
                if fails >= self.config.max_fails as usize {
                    upstream.healthy.store(false, Ordering::Relaxed);
                    warn!(addr = %upstream.addr, "upstream marked unhealthy");
                }
                Err(DnsError::Plugin(format!(
                    "upstream {} failed",
                    upstream.addr
                )))
            }
        }
    }
}

#[async_trait]
impl Plugin for ForwardPlugin {
    fn name(&self) -> &str {
        "forward"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        match self.forward_query(ctx).await {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!(error = %e, "forward failed, returning SERVFAIL");
                ctx.response = make_error_response(&ctx.request, ResponseCode::ServFail);
                Ok(())
            }
        }
    }

    async fn ready(&self) -> DnsResult<()> {
        if self.upstreams.is_empty() {
            return Err(DnsError::Config("forward: no upstreams configured".into()));
        }
        Ok(())
    }
}
