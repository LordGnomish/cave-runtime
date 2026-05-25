// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gateway-wide health probe.
//!
//! `check_all` runs the per-provider `health_check()` in parallel and
//! aggregates into a [`HealthReport`]. The report distinguishes
//! `healthy`, `unhealthy`, and `disabled` so an operator running
//! `cavectl llm-gateway health` can see why a provider is missing.

use crate::provider::{LlmProvider, ProviderRegistry};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    Healthy,
    Unhealthy,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub provider: String,
    pub state: HealthState,
    pub latency_ms: u64,
    /// Models the provider claims to support.
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub providers: Vec<ProviderHealth>,
    pub healthy_count: usize,
    pub unhealthy_count: usize,
    pub disabled_count: usize,
}

impl HealthReport {
    /// Returns true if at least one provider is healthy.
    pub fn any_healthy(&self) -> bool {
        self.healthy_count > 0
    }
}

async fn probe(p: Arc<dyn LlmProvider>) -> ProviderHealth {
    let name = p.name().to_string();
    let models = p.supported_models();
    let started = std::time::Instant::now();
    let ok = p.health_check().await;
    let latency_ms = started.elapsed().as_millis() as u64;
    ProviderHealth {
        provider: name,
        state: if ok {
            HealthState::Healthy
        } else {
            HealthState::Unhealthy
        },
        latency_ms,
        models,
    }
}

pub async fn check_all(registry: &ProviderRegistry) -> HealthReport {
    let providers: Vec<Arc<dyn LlmProvider>> = registry
        .list()
        .into_iter()
        .filter_map(|n| registry.get(&n))
        .collect();
    let probes = providers.into_iter().map(probe);
    let mut results: Vec<ProviderHealth> = join_all(probes).await;
    results.sort_by(|a, b| a.provider.cmp(&b.provider));

    let healthy_count = results.iter().filter(|p| p.state == HealthState::Healthy).count();
    let unhealthy_count = results.iter().filter(|p| p.state == HealthState::Unhealthy).count();
    let disabled_count = results.iter().filter(|p| p.state == HealthState::Disabled).count();

    HealthReport {
        providers: results,
        healthy_count,
        unhealthy_count,
        disabled_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;

    #[tokio::test]
    async fn check_all_reports_healthy_for_mock_provider() {
        let reg = ProviderRegistry::new();
        reg.register(Arc::new(MockProvider::new("mock-a")));
        reg.register(Arc::new(MockProvider::new("mock-b")));
        let r = check_all(&reg).await;
        assert_eq!(r.providers.len(), 2);
        assert_eq!(r.healthy_count, 2);
        assert_eq!(r.unhealthy_count, 0);
        assert!(r.any_healthy());
    }

    #[tokio::test]
    async fn check_all_returns_empty_report_when_no_providers_registered() {
        let reg = ProviderRegistry::new();
        let r = check_all(&reg).await;
        assert_eq!(r.providers.len(), 0);
        assert_eq!(r.healthy_count, 0);
        assert!(!r.any_healthy());
    }

    #[test]
    fn health_state_serializes_lowercase() {
        let s = serde_json::to_string(&HealthState::Healthy).unwrap();
        assert_eq!(s, "\"healthy\"");
    }
}
