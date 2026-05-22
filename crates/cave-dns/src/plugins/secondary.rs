// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Secondary plugin — serve secondary zones (AXFR from masters).
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use hickory_proto::op::ResponseCode;
use tracing::{info, warn};

use crate::{
    config::SecondaryConfig,
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
    zone::ZoneManager,
};

pub struct SecondaryPlugin {
    config: SecondaryConfig,
    zones: Arc<ArcSwap<ZoneManager>>,
}

impl SecondaryPlugin {
    pub fn new(config: SecondaryConfig) -> Self {
        Self {
            config,
            zones: Arc::new(ArcSwap::new(Arc::new(ZoneManager::new()))),
        }
    }

    async fn fetch_all(&self) -> ZoneManager {
        let mut mgr = ZoneManager::new();
        for zone_cfg in &self.config.zones {
            let master_str = match zone_cfg.masters.first() {
                Some(m) => m.clone(),
                None => continue,
            };
            let master: std::net::SocketAddr = match master_str.parse() {
                Ok(a) => a,
                Err(e) => {
                    warn!(master = %master_str, error = %e, "secondary: bad master address");
                    continue;
                }
            };
            let origin = match zone_cfg.name.parse() {
                Ok(n) => n,
                Err(e) => {
                    warn!(zone = %zone_cfg.name, error = %e, "secondary: bad zone name");
                    continue;
                }
            };
            match tokio::net::TcpStream::connect(master).await {
                Ok(mut stream) => {
                    match crate::zone::transfer::receive_axfr(&mut stream, &origin).await {
                        Ok(zone) => {
                            info!(zone = %origin, master = %master, "secondary zone fetched");
                            let _ = mgr.add_zone(zone).await;
                        }
                        Err(e) => {
                            warn!(zone = %origin, master = %master, error = %e, "AXFR failed");
                        }
                    }
                }
                Err(e) => {
                    warn!(master = %master, error = %e, "secondary: TCP connect failed");
                }
            }
        }
        mgr
    }
}

#[async_trait]
impl Plugin for SecondaryPlugin {
    fn name(&self) -> &str {
        "secondary"
    }

    async fn ready(&self) -> DnsResult<()> {
        let mgr = self.fetch_all().await;
        self.zones.store(Arc::new(mgr));

        // Periodic refresh
        let zones_ref = Arc::clone(&self.zones);
        let cfg = self.config.clone();
        let this = Self {
            config: cfg,
            zones: zones_ref,
        };
        tokio::spawn(async move {
            for zone_cfg in &this.config.zones {
                let interval = Duration::from_secs(zone_cfg.refresh_interval);
                let z = Arc::clone(&this.zones);
                let zone_name = zone_cfg.name.clone();
                let master = zone_cfg.masters.first().cloned().unwrap_or_default();
                tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(interval);
                    loop {
                        ticker.tick().await;
                        info!(zone = %zone_name, master = %master, "secondary: refresh check");
                        // In production: check SOA serial, fetch IXFR if needed
                    }
                });
            }
        });
        Ok(())
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return next.run(ctx).await,
        };

        let mgr = self.zones.load();
        if let Some(result) = mgr.lookup(q.name(), q.query_type()).await {
            if !result.records.is_empty() {
                ctx.response.set_authoritative(true);
                for r in result.records {
                    ctx.response.add_answer(r);
                }
                return Ok(());
            }
        }

        next.run(ctx).await
    }
}
