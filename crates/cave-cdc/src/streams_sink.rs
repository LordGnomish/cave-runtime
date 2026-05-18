// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-streams sink — direct producer integration with NO Kafka
//! Connect intermediary.
//!
//! Cite: debezium-embedded `EmbeddedEngine` / `AsyncEmbeddedEngine`
//! (the standalone runtime that bypasses Kafka Connect). cave-cdc's
//! sink writes batches straight into a cave-streams broker via a
//! mock-friendly `SinkBackend` trait so the unit tests can exercise
//! the dispatch logic without booting the full broker.

use crate::connector::SourceRecord;
use crate::error::{CdcError, CdcResult};
use crate::routing::TopicRouter;
use serde::{Deserialize, Serialize};

/// Cite: cave-streams `Broker::produce` signature — every produce
/// returns the assigned base offset on success. cave-cdc's sink
/// surfaces that via `ProduceResult` so downstream observers (e.g.
/// the snapshot watermark) can correlate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProduceResult {
    pub topic: String,
    pub partition: i32,
    pub base_offset: i64,
    pub records: u32,
}

/// Mockable sink backend. The production impl will wrap a
/// `cave_streams::Broker` reference; tests use the `MemorySink`
/// implementation in this module.
pub trait SinkBackend {
    fn produce(
        &mut self,
        topic: &str,
        partition: i32,
        key: &[u8],
        value: &[u8],
    ) -> CdcResult<i64>;
}

/// Tenant-scoped sink. Cite: debezium `EventDispatcher::dispatch` —
/// the dispatch fans every SourceRecord through the topic router and
/// then into the backing producer.
#[derive(Debug)]
pub struct StreamsSink<B: SinkBackend> {
    pub tenant_id: String,
    pub router: TopicRouter,
    pub partitions_per_topic: i32,
    pub backend: B,
}

impl<B: SinkBackend> StreamsSink<B> {
    pub fn new(router: TopicRouter, partitions_per_topic: i32, backend: B) -> Self {
        Self {
            tenant_id: router.tenant_id.clone(),
            router,
            partitions_per_topic,
            backend,
        }
    }

    /// Cite: debezium `EventDispatcher::dispatch` cross-tenant guard
    /// (cave extension): every record's tenant_id MUST match the
    /// sink's tenant_id; otherwise reject before touching the backend.
    pub fn dispatch(&mut self, record: &SourceRecord) -> CdcResult<ProduceResult> {
        if record.tenant_id != self.tenant_id {
            return Err(CdcError::CrossTenantDenied {
                store: self.tenant_id.clone(),
                req: record.tenant_id.clone(),
            });
        }
        self.router.assert_tenant_prefix(&record.topic)?;

        let partition = self.router.partition_for(&record.key, self.partitions_per_topic);
        let offset = self.backend.produce(
            &record.topic, partition, &record.key, &record.value,
        )?;
        Ok(ProduceResult {
            topic: record.topic.clone(),
            partition,
            base_offset: offset,
            records: 1,
        })
    }

    /// Batch dispatch — short-circuits on first error and returns
    /// per-record results up to the failure point.
    pub fn dispatch_batch(&mut self, records: &[SourceRecord]) -> CdcResult<Vec<ProduceResult>> {
        let mut results = Vec::with_capacity(records.len());
        for r in records {
            results.push(self.dispatch(r)?);
        }
        Ok(results)
    }
}

/// In-memory backend for tests + integration smoke runs. Cite:
/// debezium `EmbeddedEngine` test harness (`InMemoryOffsetBackingStore`).
#[derive(Debug, Default)]
pub struct MemorySink {
    /// (topic, partition) → next offset.
    next_offset: std::collections::HashMap<(String, i32), i64>,
    /// (topic, partition) → produced records (key, value) in order.
    pub log: std::collections::HashMap<(String, i32), Vec<(Vec<u8>, Vec<u8>)>>,
}

impl SinkBackend for MemorySink {
    fn produce(
        &mut self,
        topic: &str,
        partition: i32,
        key: &[u8],
        value: &[u8],
    ) -> CdcResult<i64> {
        let key_pair = (topic.to_string(), partition);
        let entry = self.next_offset.entry(key_pair.clone()).or_insert(0);
        let offset = *entry;
        *entry += 1;
        self.log.entry(key_pair).or_default()
            .push((key.to_vec(), value.to_vec()));
        Ok(offset)
    }
}

impl MemorySink {
    pub fn new() -> Self { Self::default() }

    pub fn count_for(&self, topic: &str, partition: i32) -> usize {
        self.log.get(&(topic.to_string(), partition)).map(Vec::len).unwrap_or(0)
    }
}
