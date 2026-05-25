// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus plugin — Corefile alias for the metrics plugin.
//!
//! CoreDNS upstream registers a `prometheus` plugin name that is an exact
//! alias for the `metrics` plugin's Prometheus exporter (see
//! `plugin/metrics/setup.go`'s `init()` call). cave-dns keeps the alias so
//! existing Corefiles continue to work — the plugin is a thin pass-through
//! that bumps an alias-hit counter and delegates to `Next`.

use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    config::MetricsConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

/// Alias-style plugin: delegates DNS handling to `Next` (the metrics plugin
/// will already be earlier in the chain), but exposes a publishable counter
/// so operators can verify the Corefile alias path is taken.
pub struct PrometheusPlugin {
    config: MetricsConfig,
    alias_hits: AtomicU64,
}

impl PrometheusPlugin {
    pub fn new(config: MetricsConfig) -> Self {
        Self {
            config,
            alias_hits: AtomicU64::new(0),
        }
    }

    pub fn from_metrics_config() -> Self {
        Self::new(MetricsConfig::default())
    }

    pub fn alias_hits(&self) -> u64 {
        self.alias_hits.load(Ordering::Relaxed)
    }

    /// Exporter address pulled from the underlying MetricsConfig.
    pub fn exporter_addr(&self) -> &str {
        &self.config.addr
    }

    /// Exporter path pulled from the underlying MetricsConfig.
    pub fn exporter_path(&self) -> &str {
        &self.config.path
    }
}

#[async_trait]
impl Plugin for PrometheusPlugin {
    fn name(&self) -> &str {
        "prometheus"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        self.alias_hits.fetch_add(1, Ordering::Relaxed);
        next.run(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prometheus_alias_defaults_to_metrics_addr() {
        let p = PrometheusPlugin::from_metrics_config();
        assert_eq!(p.exporter_addr(), "0.0.0.0:9153");
        assert_eq!(p.exporter_path(), "/metrics");
        assert_eq!(p.alias_hits(), 0);
        assert_eq!(p.name(), "prometheus");
    }

    #[test]
    fn prometheus_alias_overrides_addr() {
        let cfg = MetricsConfig {
            addr: "127.0.0.1:19153".into(),
            path: "/m".into(),
        };
        let p = PrometheusPlugin::new(cfg);
        assert_eq!(p.exporter_addr(), "127.0.0.1:19153");
        assert_eq!(p.exporter_path(), "/m");
    }
}
