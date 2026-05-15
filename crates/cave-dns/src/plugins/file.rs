// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// File plugin — serve zones loaded from zone files, with hot-reload.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use hickory_proto::op::ResponseCode;
use hickory_proto::rr::RecordType;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{info, warn};

use crate::{
    config::FilePluginConfig,
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
    zone::{Zone, ZoneManager},
};

pub struct FilePlugin {
    config: FilePluginConfig,
    zones: Arc<ArcSwap<ZoneManager>>,
}

impl FilePlugin {
    pub fn new(config: FilePluginConfig) -> DnsResult<Self> {
        let mgr = ZoneManager::new();
        Ok(Self {
            config,
            zones: Arc::new(ArcSwap::new(Arc::new(mgr))),
        })
    }

    async fn load_all(&self) -> DnsResult<ZoneManager> {
        let mut mgr = ZoneManager::new();
        for path_str in &self.config.zones {
            let path = PathBuf::from(path_str);
            if let Err(e) = load_zone_into_manager(&mut mgr, &path).await {
                warn!(path = %path.display(), error = %e, "failed to load zone file");
            }
        }
        Ok(mgr)
    }
}

async fn load_zone_into_manager(mgr: &mut ZoneManager, path: &Path) -> DnsResult<()> {
    // Infer zone origin from SOA in the file (or file name)
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        DnsError::Zone(format!("read {}: {e}", path.display()))
    })?;

    // Extract $ORIGIN directive or fall back to filename stem
    let origin_str = content
        .lines()
        .find_map(|l| {
            let l = l.trim();
            if l.to_uppercase().starts_with("$ORIGIN") {
                l.split_whitespace().nth(1).map(str::to_owned)
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| format!("{s}."))
                .unwrap_or_else(|| "unknown.".into())
        });

    let origin = origin_str
        .parse()
        .map_err(|e: hickory_proto::error::ProtoError| DnsError::Parse(e.to_string()))?;

    let zone = crate::zone::file::load_zone_file(path, &origin)?;
    mgr.add_zone(zone).await
}

#[async_trait]
impl Plugin for FilePlugin {
    fn name(&self) -> &str {
        "file"
    }

    async fn ready(&self) -> DnsResult<()> {
        let mgr = self.load_all().await?;
        self.zones.store(Arc::new(mgr));
        info!(zones = self.config.zones.len(), "file plugin loaded zones");
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
                ctx.response.set_authoritative(result.authoritative);
                for r in result.records {
                    ctx.response.add_answer(r);
                }
                return Ok(());
            }
            if result.authoritative {
                // Authoritative NXDOMAIN — don't forward
                ctx.response.set_response_code(ResponseCode::NXDomain);
                ctx.response.set_authoritative(true);
                return Ok(());
            }
        }

        next.run(ctx).await
    }
}
