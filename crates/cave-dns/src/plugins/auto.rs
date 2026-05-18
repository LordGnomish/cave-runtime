// SPDX-License-Identifier: AGPL-3.0-or-later
/// Auto plugin — automatically load all zone files from a directory.
use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use hickory_proto::op::ResponseCode;
use tracing::{info, warn};

use crate::{
    config::AutoConfig,
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
    zone::ZoneManager,
};

pub struct AutoPlugin {
    config: AutoConfig,
    zones: Arc<ArcSwap<ZoneManager>>,
}

impl AutoPlugin {
    pub fn new(config: AutoConfig) -> Self {
        Self {
            config,
            zones: Arc::new(ArcSwap::new(Arc::new(ZoneManager::new()))),
        }
    }

    async fn load_directory(&self) -> DnsResult<ZoneManager> {
        let dir = PathBuf::from(&self.config.directory);
        let mut mgr = ZoneManager::new();

        let mut entries = tokio::fs::read_dir(&dir).await.map_err(|e| {
            DnsError::Zone(format!("auto: cannot read directory {}: {e}", dir.display()))
        })?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            // Match template glob (simple *.zone check)
            if !glob_match(&self.config.template, name) {
                continue;
            }
            let origin_str = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| {
                    if s.ends_with('.') {
                        s.to_owned()
                    } else {
                        format!("{s}.")
                    }
                })
                .unwrap_or_else(|| "unknown.".into());
            let origin = match origin_str.parse() {
                Ok(n) => n,
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "skipping zone file");
                    continue;
                }
            };
            match crate::zone::file::load_zone_file(&path, &origin) {
                Ok(zone) => {
                    info!(zone = %origin, path = %path.display(), "auto loaded zone");
                    let _ = mgr.add_zone(zone).await;
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "auto: zone load error");
                }
            }
        }

        Ok(mgr)
    }
}

/// Simple glob match supporting only `*` wildcard.
fn glob_match(pattern: &str, name: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        pattern == name
    }
}

#[async_trait]
impl Plugin for AutoPlugin {
    fn name(&self) -> &str {
        "auto"
    }

    async fn ready(&self) -> DnsResult<()> {
        let mgr = self.load_directory().await?;
        info!(zones = mgr.len(), directory = %self.config.directory, "auto plugin ready");
        self.zones.store(Arc::new(mgr));
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
                ctx.response.set_response_code(ResponseCode::NXDomain);
                ctx.response.set_authoritative(true);
                return Ok(());
            }
        }

        next.run(ctx).await
    }
}
