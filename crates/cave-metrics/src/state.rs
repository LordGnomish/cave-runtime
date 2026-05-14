// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared application state for cave-metrics.

use std::sync::Arc;
use parking_lot::RwLock;

use crate::promql::Engine;
use crate::rules::RuleGroup;
use crate::scrape::ScrapeManager;
use crate::tsdb::Tsdb;

/// Shared state passed to all handlers.
pub struct MetricsState {
    pub tsdb:           Arc<Tsdb>,
    pub engine:         Arc<Engine>,
    pub scrape_manager: Arc<ScrapeManager>,
    pub rule_groups:    Arc<RwLock<Vec<RuleGroup>>>,
}

impl MetricsState {
    pub fn new() -> Arc<Self> {
        let tsdb = Arc::new(Tsdb::default());
        let engine = Arc::new(Engine::new(Arc::clone(&tsdb)));
        let scrape_manager = Arc::new(ScrapeManager::new(Arc::clone(&tsdb)));

        Arc::new(Self {
            tsdb,
            engine,
            scrape_manager,
            rule_groups: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Start all background tasks.
    pub fn start_background_tasks(self: &Arc<Self>) {
        // TSDB retention and compaction
        Arc::clone(&self.tsdb).start_background_tasks();

        // Scrape manager
        Arc::clone(&self.scrape_manager).start();

        // Rules evaluation
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                let ts_ms = now_ms();
                let mut groups = state.rule_groups.write();
                for group in groups.iter_mut() {
                    if let Err(e) = group.evaluate(&state.engine, &state.tsdb, ts_ms) {
                        tracing::warn!("Rule group '{}' evaluation error: {}", group.name, e);
                    }
                }
            }
        });
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
