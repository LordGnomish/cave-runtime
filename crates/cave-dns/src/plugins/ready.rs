// SPDX-License-Identifier: AGPL-3.0-or-later
/// Ready plugin — HTTP /ready endpoint.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use axum::{routing::get, Router};
use tracing::info;

use crate::{
    config::ReadyConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

pub struct ReadyPlugin {
    config: ReadyConfig,
    ready: Arc<AtomicBool>,
}

impl ReadyPlugin {
    pub fn new(config: ReadyConfig) -> Self {
        Self {
            config,
            ready: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl Plugin for ReadyPlugin {
    fn name(&self) -> &str {
        "ready"
    }

    async fn ready(&self) -> DnsResult<()> {
        let ready = Arc::clone(&self.ready);
        let addr = self.config.addr.clone();
        let path = self.config.path.clone();

        ready.store(true, Ordering::Relaxed);

        tokio::spawn(async move {
            let r = Arc::clone(&ready);
            let app = Router::new().route(
                &path,
                get(move || {
                    let r2 = Arc::clone(&r);
                    async move {
                        if r2.load(Ordering::Relaxed) {
                            (axum::http::StatusCode::OK, "OK")
                        } else {
                            (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Not ready")
                        }
                    }
                }),
            );
            let listener = match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(addr = %addr, error = %e, "ready: bind failed");
                    return;
                }
            };
            info!(addr = %addr, "ready endpoint listening");
            if let Err(e) = axum::serve(listener, app).await {
                tracing::warn!(error = %e, "ready server error");
            }
        });
        Ok(())
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        next.run(ctx).await
    }
}
