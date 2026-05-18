// SPDX-License-Identifier: AGPL-3.0-or-later
/// Reload plugin — signal-triggered or timed config hot-reload.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use tracing::{info, warn};

use crate::{
    config::ReloadConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

pub struct ReloadPlugin {
    config: ReloadConfig,
    reload_pending: Arc<AtomicBool>,
}

impl ReloadPlugin {
    pub fn new(config: ReloadConfig) -> Self {
        Self {
            config,
            reload_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Mark a reload as pending (call from external signal handler or test).
    pub fn request_reload(&self) {
        self.reload_pending.store(true, Ordering::Relaxed);
    }
}

#[async_trait]
impl Plugin for ReloadPlugin {
    fn name(&self) -> &str {
        "reload"
    }

    async fn ready(&self) -> DnsResult<()> {
        let pending = Arc::clone(&self.reload_pending);
        let interval = Duration::from_secs(self.config.interval_secs);

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                if pending.swap(false, Ordering::Relaxed) {
                    info!("reload: reloading configuration");
                    // In a full implementation this would signal the server to
                    // re-read its Corefile / DnsConfig and rebuild the plugin chain.
                }
            }
        });
        Ok(())
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        next.run(ctx).await
    }
}
