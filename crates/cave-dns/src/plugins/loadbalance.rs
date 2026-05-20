// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Loadbalance plugin — round-robin shuffle of A/AAAA answer records.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use dashmap::DashMap;
use hickory_proto::rr::{Name, Record, RecordType};

use crate::{
    config::{LbPolicy, LoadbalanceConfig},
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

pub struct LoadbalancePlugin {
    config: LoadbalanceConfig,
    counter: Arc<DashMap<Name, AtomicUsize>>,
}

impl LoadbalancePlugin {
    pub fn new(config: LoadbalanceConfig) -> Self {
        Self {
            config,
            counter: Arc::new(DashMap::new()),
        }
    }

    fn rotate(&self, name: &Name, records: Vec<Record>) -> Vec<Record> {
        if records.len() <= 1 {
            return records;
        }
        match self.config.policy {
            LbPolicy::RoundRobin => {
                let idx = self
                    .counter
                    .entry(name.clone())
                    .or_insert_with(|| AtomicUsize::new(0))
                    .fetch_add(1, Ordering::Relaxed)
                    % records.len();
                let mut out = Vec::with_capacity(records.len());
                for i in 0..records.len() {
                    out.push(records[(idx + i) % records.len()].clone());
                }
                out
            }
            LbPolicy::Random => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let seed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() as usize)
                    .unwrap_or(0);
                let idx = seed % records.len();
                let mut out = Vec::with_capacity(records.len());
                for i in 0..records.len() {
                    out.push(records[(idx + i) % records.len()].clone());
                }
                out
            }
            LbPolicy::Weighted => records, // TODO: weight-aware rotation
        }
    }
}

#[async_trait]
impl Plugin for LoadbalancePlugin {
    fn name(&self) -> &str {
        "loadbalance"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        next.run(ctx).await?;

        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return Ok(()),
        };

        let answers = ctx.response.take_answers();
        if answers.is_empty() {
            return Ok(());
        }

        // Separate A/AAAA records (rotatable) from others (fixed)
        let (mut rotatable, fixed): (Vec<Record>, Vec<Record>) = answers
            .into_iter()
            .partition(|r| r.record_type() == RecordType::A || r.record_type() == RecordType::AAAA);

        rotatable = self.rotate(q.name(), rotatable);

        for r in rotatable.into_iter().chain(fixed) {
            ctx.response.add_answer(r);
        }
        Ok(())
    }
}
