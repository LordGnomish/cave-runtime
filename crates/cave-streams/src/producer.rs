//! Producer API — send messages with key, value, headers, timestamp.
//!
//! Supports three partitioning strategies and exactly-once semantics via:
//!   * **Idempotent producer** — PID + sequence number deduplication.
//!   * **Transactional API** — atomic multi-partition produce.

use crate::error::{StreamError, StreamResult};
use crate::models::{
    Header, PartitionerStrategy, ProducerRecord, ProducerState, Record, RecordMetadata,
    TopicPartition, Transaction, TransactionState,
};
use crate::storage::StreamStorage;
use std::collections::HashMap;

// ─── Partitioner ─────────────────────────────────────────────────────────────

/// Choose a target partition for a record.
pub fn choose_partition(
    strategy: &PartitionerStrategy,
    key: Option<&[u8]>,
    partition_count: u32,
    round_robin_counter: u32,
) -> u32 {
    match strategy {
        PartitionerStrategy::Manual(p) => p % partition_count,
        PartitionerStrategy::KeyHash => {
            if let Some(k) = key.filter(|k| !k.is_empty()) {
                fnv1a(k) % partition_count
            } else {
                round_robin_counter % partition_count
            }
        }
        PartitionerStrategy::RoundRobin => round_robin_counter % partition_count,
    }
}

/// FNV-1a 32-bit hash (no external crate needed).
fn fnv1a(data: &[u8]) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in data {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

// ─── Producer ────────────────────────────────────────────────────────────────

/// Stateful producer engine that wraps a [`StreamStorage`] backend.
///
/// A single `Producer` may be shared across async tasks (it is `Clone` if `S:
/// Clone`).
pub struct Producer<S: StreamStorage> {
    storage: S,
    producer_id: i64,
    producer_epoch: i16,
    transactional_id: Option<String>,
    /// Sticky round-robin counter, incremented per send.
    rr_counter: std::sync::atomic::AtomicU32,
    /// True when an active transaction is in progress.
    in_transaction: std::sync::Mutex<bool>,
}

impl<S: StreamStorage> Producer<S> {
    // ── Constructors ─────────────────────────────────────────────────────────

    /// Create a plain (non-idempotent) producer.
    pub fn new(storage: S) -> StreamResult<Self> {
        Ok(Self {
            storage,
            producer_id: -1,
            producer_epoch: 0,
            transactional_id: None,
            rr_counter: std::sync::atomic::AtomicU32::new(0),
            in_transaction: std::sync::Mutex::new(false),
        })
    }

    /// Create an **idempotent** producer (guaranteed at-least-once-then-deduped).
    pub fn new_idempotent(storage: S) -> StreamResult<Self> {
        let producer_id = storage.allocate_producer_id()?;
        let state = ProducerState {
            producer_id,
            producer_epoch: 0,
            transactional_id: None,
            last_sequence: HashMap::new(),
        };
        storage.update_producer_state(state)?;
        Ok(Self {
            storage,
            producer_id,
            producer_epoch: 0,
            transactional_id: None,
            rr_counter: std::sync::atomic::AtomicU32::new(0),
            in_transaction: std::sync::Mutex::new(false),
        })
    }

    /// Create a **transactional** producer.
    pub fn new_transactional(storage: S, transactional_id: impl Into<String>) -> StreamResult<Self> {
        let txn_id = transactional_id.into();
        let producer_id = storage.allocate_producer_id()?;
        let state = ProducerState {
            producer_id,
            producer_epoch: 0,
            transactional_id: Some(txn_id.clone()),
            last_sequence: HashMap::new(),
        };
        storage.update_producer_state(state)?;
        Ok(Self {
            storage,
            producer_id,
            producer_epoch: 0,
            transactional_id: Some(txn_id),
            rr_counter: std::sync::atomic::AtomicU32::new(0),
            in_transaction: std::sync::Mutex::new(false),
        })
    }

    // ── Sending records ───────────────────────────────────────────────────────

    /// Send a single record.  Returns the partition and offset it landed on.
    pub fn send(&self, record: ProducerRecord) -> StreamResult<RecordMetadata> {
        let topic_info = self
            .storage
            .get_topic(&record.topic)?
            .ok_or_else(|| StreamError::TopicNotFound(record.topic.clone()))?;

        // Validate message size.
        let size = record.value.as_deref().map(|v| v.len()).unwrap_or(0);
        if size > topic_info.config.max_message_bytes {
            return Err(StreamError::MessageTooLarge {
                size,
                max: topic_info.config.max_message_bytes,
            });
        }

        let rr = self
            .rr_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partition = choose_partition(
            &record.partitioner,
            record.key.as_deref(),
            topic_info.partitions,
            rr,
        );

        let timestamp_ms = record
            .timestamp_ms
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

        let (sequence, epoch) = if self.producer_id >= 0 {
            let seq = self.next_sequence(TopicPartition::new(&record.topic, partition))?;
            (Some(seq), Some(self.producer_epoch))
        } else {
            (None, None)
        };

        let mut r = Record::new(&record.topic, partition, record.key, record.value);
        r.headers = record.headers;
        r.timestamp_ms = timestamp_ms;
        r.producer_id = if self.producer_id >= 0 {
            Some(self.producer_id)
        } else {
            None
        };
        r.producer_epoch = epoch;
        r.sequence = sequence;

        let offset = if self.transactional_id.is_some()
            && *self.in_transaction.lock().unwrap()
        {
            // Buffer into the open transaction.
            self.buffer_transactional(record.topic.clone(), partition, r.clone())?;
            -1 // offset unknown until commit
        } else {
            self.storage
                .append_to_partition(&record.topic, partition, r)?
        };

        Ok(RecordMetadata {
            topic: record.topic,
            partition,
            offset,
            timestamp_ms,
        })
    }

    /// Send multiple records atomically (batch).
    pub fn send_batch(
        &self,
        records: Vec<ProducerRecord>,
    ) -> StreamResult<Vec<RecordMetadata>> {
        records.into_iter().map(|r| self.send(r)).collect()
    }

    // ── Idempotent helpers ────────────────────────────────────────────────────

    fn next_sequence(&self, tp: TopicPartition) -> StreamResult<i32> {
        let mut state = self
            .storage
            .get_producer_state(self.producer_id)?
            .ok_or_else(|| {
                StreamError::Producer(format!(
                    "Producer state not found for id={}",
                    self.producer_id
                ))
            })?;
        let seq = state.last_sequence.entry(tp).or_insert(-1);
        *seq += 1;
        let next = *seq;
        self.storage.update_producer_state(state)?;
        Ok(next)
    }

    // ── Transactional API ─────────────────────────────────────────────────────

    /// Begin an atomic transaction.
    pub fn begin_transaction(&self) -> StreamResult<()> {
        let txn_id = self
            .transactional_id
            .as_deref()
            .ok_or_else(|| StreamError::Transaction("Producer is not transactional".into()))?;

        let mut flag = self.in_transaction.lock().unwrap();
        if *flag {
            return Err(StreamError::Transaction(
                "Transaction already in progress".into(),
            ));
        }

        let txn = Transaction {
            transactional_id: txn_id.to_string(),
            producer_id: self.producer_id,
            producer_epoch: self.producer_epoch,
            pending: Vec::new(),
            state: TransactionState::Ongoing,
            timeout_ms: 60_000,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
        };
        self.storage.begin_transaction(txn)?;
        *flag = true;
        Ok(())
    }

    /// Commit the open transaction — all buffered records become visible.
    pub fn commit_transaction(&self) -> StreamResult<Vec<RecordMetadata>> {
        let txn_id = self
            .transactional_id
            .as_deref()
            .ok_or_else(|| StreamError::Transaction("Producer is not transactional".into()))?;

        let mut flag = self.in_transaction.lock().unwrap();
        if !*flag {
            return Err(StreamError::Transaction("No transaction in progress".into()));
        }

        let mut txn = self
            .storage
            .get_transaction(txn_id)?
            .ok_or_else(|| StreamError::TransactionNotFound(txn_id.into()))?;

        txn.state = TransactionState::PrepareCommit;
        self.storage.update_transaction(txn.clone())?;

        // Flush buffered records to the actual logs.
        let mut metas = Vec::new();
        for (tp, records) in txn.pending {
            for record in records {
                let offset =
                    self.storage
                        .append_to_partition(&tp.topic, tp.partition, record.clone())?;
                metas.push(RecordMetadata {
                    topic: tp.topic.clone(),
                    partition: tp.partition,
                    offset,
                    timestamp_ms: record.timestamp_ms,
                });
            }
        }

        let mut txn = self
            .storage
            .get_transaction(txn_id)?
            .ok_or_else(|| StreamError::TransactionNotFound(txn_id.into()))?;
        txn.state = TransactionState::CompleteCommit;
        txn.pending = Vec::new();
        self.storage.update_transaction(txn)?;

        *flag = false;
        Ok(metas)
    }

    /// Abort the open transaction — all buffered records are discarded.
    pub fn abort_transaction(&self) -> StreamResult<()> {
        let txn_id = self
            .transactional_id
            .as_deref()
            .ok_or_else(|| StreamError::Transaction("Producer is not transactional".into()))?;

        let mut flag = self.in_transaction.lock().unwrap();
        if !*flag {
            return Err(StreamError::Transaction("No transaction in progress".into()));
        }

        let mut txn = self
            .storage
            .get_transaction(txn_id)?
            .ok_or_else(|| StreamError::TransactionNotFound(txn_id.into()))?;

        txn.state = TransactionState::CompleteAbort;
        txn.pending = Vec::new();
        self.storage.update_transaction(txn)?;

        *flag = false;
        Ok(())
    }

    fn buffer_transactional(
        &self,
        topic: String,
        partition: u32,
        record: Record,
    ) -> StreamResult<()> {
        let txn_id = self.transactional_id.as_deref().unwrap();
        let mut txn = self
            .storage
            .get_transaction(txn_id)?
            .ok_or_else(|| StreamError::TransactionNotFound(txn_id.into()))?;

        let tp = TopicPartition::new(topic, partition);
        if let Some(entry) = txn.pending.iter_mut().find(|(t, _)| t == &tp) {
            entry.1.push(record);
        } else {
            txn.pending.push((tp, vec![record]));
        }
        self.storage.update_transaction(txn)
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn producer_id(&self) -> i64 {
        self.producer_id
    }

    pub fn producer_epoch(&self) -> i16 {
        self.producer_epoch
    }
}

// ─── Builder ─────────────────────────────────────────────────────────────────

/// Fluent builder for [`ProducerRecord`].
#[derive(Default)]
pub struct ProducerRecordBuilder {
    topic: String,
    key: Option<Vec<u8>>,
    value: Option<Vec<u8>>,
    headers: Vec<Header>,
    timestamp_ms: Option<i64>,
    partitioner: PartitionerStrategy,
}

impl ProducerRecordBuilder {
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            partitioner: PartitionerStrategy::KeyHash,
            ..Default::default()
        }
    }

    pub fn key(mut self, key: impl Into<Vec<u8>>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn value(mut self, value: impl Into<Vec<u8>>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<Vec<u8>>) -> Self {
        self.headers.push(Header {
            key: key.into(),
            value: value.into(),
        });
        self
    }

    pub fn timestamp_ms(mut self, ts: i64) -> Self {
        self.timestamp_ms = Some(ts);
        self
    }

    pub fn partitioner(mut self, p: PartitionerStrategy) -> Self {
        self.partitioner = p;
        self
    }

    pub fn build(self) -> ProducerRecord {
        ProducerRecord {
            topic: self.topic,
            key: self.key,
            value: self.value,
            headers: self.headers,
            timestamp_ms: self.timestamp_ms,
            partitioner: self.partitioner,
        }
    }
}

impl Default for PartitionerStrategy {
    fn default() -> Self {
        PartitionerStrategy::KeyHash
    }
}
