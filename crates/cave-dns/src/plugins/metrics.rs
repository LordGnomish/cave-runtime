// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Metrics plugin — Prometheus /metrics endpoint.
use std::sync::Arc;

use async_trait::async_trait;
use axum::{Router, routing::get};
use prometheus_client::{
    encoding::text::encode,
    metrics::{
        counter::Counter,
        family::Family,
        histogram::{Histogram, exponential_buckets},
    },
    registry::Registry,
};
use tokio::sync::Mutex;
use tracing::info;

use crate::{
    config::MetricsConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

#[derive(Clone, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet, Debug)]
struct QueryLabels {
    proto: String,
    qtype: String,
    rcode: String,
}

pub struct MetricsPlugin {
    config: MetricsConfig,
    registry: Arc<Mutex<Registry>>,
    requests_total: Family<QueryLabels, Counter>,
    request_duration: Family<QueryLabels, Histogram>,
}

impl MetricsPlugin {
    pub fn new(config: MetricsConfig) -> Self {
        let mut registry = Registry::default();

        let requests_total: Family<QueryLabels, Counter> = Family::default();
        registry.register("dns_requests", "Total DNS requests", requests_total.clone());

        let request_duration: Family<QueryLabels, Histogram> =
            Family::new_with_constructor(|| Histogram::new(exponential_buckets(0.001, 2.0, 12)));
        registry.register(
            "dns_request_duration_seconds",
            "DNS request latency",
            request_duration.clone(),
        );

        Self {
            config,
            registry: Arc::new(Mutex::new(registry)),
            requests_total,
            request_duration,
        }
    }
}

#[async_trait]
impl Plugin for MetricsPlugin {
    fn name(&self) -> &str {
        "metrics"
    }

    async fn ready(&self) -> DnsResult<()> {
        let registry = Arc::clone(&self.registry);
        let addr = self.config.addr.clone();
        let path = self.config.path.clone();

        tokio::spawn(async move {
            let app = Router::new().route(
                &path,
                get(move || {
                    let reg = Arc::clone(&registry);
                    async move {
                        let reg = reg.lock().await;
                        let mut buf = String::new();
                        encode(&mut buf, &reg).unwrap_or(());
                        buf
                    }
                }),
            );
            let listener = match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(addr = %addr, error = %e, "metrics: bind failed");
                    return;
                }
            };
            info!(addr = %addr, "metrics endpoint listening");
            if let Err(e) = axum::serve(listener, app).await {
                tracing::warn!(error = %e, "metrics server error");
            }
        });
        Ok(())
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        next.run(ctx).await?;

        let q = ctx.request.queries().first();
        let labels = QueryLabels {
            proto: format!("{:?}", ctx.proto).to_lowercase(),
            qtype: q.map(|q| q.query_type().to_string()).unwrap_or_default(),
            rcode: format!("{:?}", ctx.response.response_code()),
        };

        self.requests_total.get_or_create(&labels).inc();
        let latency = ctx.start_time.elapsed().as_secs_f64();
        self.request_duration
            .get_or_create(&labels)
            .observe(latency);

        Ok(())
    }
}
