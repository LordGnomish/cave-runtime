// SPDX-License-Identifier: AGPL-3.0-or-later
/// Health plugin — HTTP /health endpoint.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use axum::{
    routing::get,
    Router,
};
use tracing::info;

use crate::{
    config::HealthConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

pub struct HealthPlugin {
    config: HealthConfig,
    healthy: Arc<AtomicBool>,
}

impl HealthPlugin {
    pub fn new(config: HealthConfig) -> Self {
        Self {
            config,
            healthy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl Plugin for HealthPlugin {
    fn name(&self) -> &str {
        "health"
    }

    async fn ready(&self) -> DnsResult<()> {
        let healthy = Arc::clone(&self.healthy);
        let addr = self.config.addr.clone();
        let path = self.config.path.clone();

        healthy.store(true, Ordering::Relaxed);

        tokio::spawn(async move {
            let h = Arc::clone(&healthy);
            let app = Router::new().route(
                &path,
                get(move || {
                    let h2 = Arc::clone(&h);
                    async move {
                        if h2.load(Ordering::Relaxed) {
                            axum::http::StatusCode::OK
                        } else {
                            axum::http::StatusCode::SERVICE_UNAVAILABLE
                        }
                    }
                }),
            );
            let listener = match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(addr = %addr, error = %e, "health: bind failed");
                    return;
                }
            };
            info!(addr = %addr, "health endpoint listening");
            if let Err(e) = axum::serve(listener, app).await {
                tracing::warn!(error = %e, "health server error");
            }
        });
        Ok(())
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        next.run(ctx).await
    }
}
