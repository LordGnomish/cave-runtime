// SPDX-License-Identifier: AGPL-3.0-or-later
//! Storage abstraction for cave-streams.
//!
//! [`StreamStorage`] is the core trait that decouples the streaming engine from
//! any specific persistence backend.  The default implementation is
//! [`MemoryStorage`] (in-process, non-durable, useful for testing and
//! single-node deployments).  A [`PostgresStorage`] skeleton is also provided
//! to show how the cave-db `CavePool` would be used for durable persistence.

use crate::error::{StreamError, StreamResult};
use crate::models::{
    CompatibilityMode, ConnectorConfig, ConsumerGroup, PartitionLog, ProducerState, Schema,
    StorageTierConfig, StreamPipelineConfig, TopicInfo, Transaction, TopicPartition,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ─── Storage trait ────────────────────────────────────────────────────────────

/// Asynchronous storage contract for the streaming engine.
///
/// Implementors must be `Send + Sync + 'static` so they can be shared across
/// Tokio tasks.
pub trait StreamStorage: Send + Sync + 'static {
    // ── Topic management ─────────────────────────────────────────────────────

    fn create_topic(&self, topic: TopicInfo) -> StreamResult<()>;
    fn delete_topic(&self, name: &str) -> StreamResult<()>;
    fn get_topic(&self, name: &str) -> StreamResult<Option<TopicInfo>>;
    fn list_topics(&self) -> StreamResult<Vec<TopicInfo>>;
    fn update_topic(&self, topic: TopicInfo) -> StreamResult<()>;

    // ── Partition log ────────────────────────────────────────────────────────

    fn append_to_partition(
        &self,
        topic: &str,
        partition: u32,
        record: crate::models::Record,
    ) -> StreamResult<i64>;

    fn fetch_from_partition(
        &self,
        topic: &str,
        partition: u32,
        offset: i64,
        max_count: usize,
    ) -> StreamResult<Vec<crate::models::Record>>;

    fn high_watermark(&self, topic: &str, partition: u32) -> StreamResult<i64>;
    fn log_start_offset(&self, topic: &str, partition: u32) -> StreamResult<i64>;

    /// Replace the in-memory partition log (used by compaction).
    fn replace_partition_log(&self, topic: &str, partition: u32, log: PartitionLog) -> StreamResult<()>;

    fn get_partition_log(&self, topic: &str, partition: u32) -> StreamResult<Option<PartitionLog>>;

    // ── Consumer group offsets ───────────────────────────────────────────────

    fn commit_offset(&self, group: &str, topic: &str, partition: u32, offset: i64) -> StreamResult<()>;
    fn get_offset(&self, group: &str, topic: &str, partition: u32) -> StreamResult<i64>;

    // ── Consumer group management ────────────────────────────────────────────

    fn get_or_create_group(&self, group_id: &str) -> StreamResult<ConsumerGroup>;
    fn update_group(&self, group: ConsumerGroup) -> StreamResult<()>;
    fn get_group(&self, group_id: &str) -> StreamResult<Option<ConsumerGroup>>;
    fn list_groups(&self) -> StreamResult<Vec<ConsumerGroup>>;
    fn delete_group(&self, group_id: &str) -> StreamResult<()>;

    // ── Idempotent producer ──────────────────────────────────────────────────

    fn allocate_producer_id(&self) -> StreamResult<i64>;
    fn get_producer_state(&self, producer_id: i64) -> StreamResult<Option<ProducerState>>;
    fn update_producer_state(&self, state: ProducerState) -> StreamResult<()>;

    // ── Transactions ─────────────────────────────────────────────────────────

    fn begin_transaction(&self, txn: Transaction) -> StreamResult<()>;
    fn get_transaction(&self, txn_id: &str) -> StreamResult<Option<Transaction>>;
    fn update_transaction(&self, txn: Transaction) -> StreamResult<()>;

    // ── Schema registry ──────────────────────────────────────────────────────

    fn register_schema(&self, schema: Schema) -> StreamResult<u32>;
    fn get_schema(&self, id: u32) -> StreamResult<Option<Schema>>;
    fn get_latest_schema(&self, subject: &str) -> StreamResult<Option<Schema>>;
    fn get_schema_by_version(&self, subject: &str, version: u32) -> StreamResult<Option<Schema>>;
    fn list_subject_versions(&self, subject: &str) -> StreamResult<Vec<u32>>;
    fn list_subjects(&self) -> StreamResult<Vec<String>>;
    fn delete_schema(&self, subject: &str, version: u32) -> StreamResult<()>;
    fn get_subject_compat(&self, subject: &str) -> StreamResult<CompatibilityMode>;
    fn set_subject_compat(&self, subject: &str, mode: CompatibilityMode) -> StreamResult<()>;
    fn next_schema_id(&self) -> StreamResult<u32>;

    // ── Connectors ───────────────────────────────────────────────────────────

    fn create_connector(&self, cfg: ConnectorConfig) -> StreamResult<()>;
    fn get_connector(&self, name: &str) -> StreamResult<Option<ConnectorConfig>>;
    fn list_connectors(&self) -> StreamResult<Vec<ConnectorConfig>>;
    fn update_connector(&self, cfg: ConnectorConfig) -> StreamResult<()>;
    fn delete_connector(&self, name: &str) -> StreamResult<()>;

    // ── Pipelines ────────────────────────────────────────────────────────────

    fn create_pipeline(&self, cfg: StreamPipelineConfig) -> StreamResult<()>;
    fn get_pipeline(&self, id: Uuid) -> StreamResult<Option<StreamPipelineConfig>>;
    fn list_pipelines(&self) -> StreamResult<Vec<StreamPipelineConfig>>;
    fn update_pipeline(&self, cfg: StreamPipelineConfig) -> StreamResult<()>;
    fn delete_pipeline(&self, id: Uuid) -> StreamResult<()>;

    // ── Tiered storage config ─────────────────────────────────────────────────

    fn get_tier_config(&self) -> StreamResult<StorageTierConfig>;
    fn set_tier_config(&self, cfg: StorageTierConfig) -> StreamResult<()>;
}

// ─── In-memory state ─────────────────────────────────────────────────────────

#[derive(Default)]
struct Inner {
    topics: HashMap<String, TopicInfo>,
    /// (topic, partition) → log
    logs: HashMap<(String, u32), PartitionLog>,
    /// (group, topic, partition) → committed offset
    offsets: HashMap<(String, String, u32), i64>,
    groups: HashMap<String, ConsumerGroup>,
    producers: HashMap<i64, ProducerState>,
    next_producer_id: i64,
    transactions: HashMap<String, Transaction>,
    schemas: HashMap<u32, Schema>,
    /// (subject, version) → schema_id
    schema_versions: HashMap<(String, u32), u32>,
    /// subject → latest version number
    subject_latest: HashMap<String, u32>,
    subject_compat: HashMap<String, CompatibilityMode>,
    next_schema_id: u32,
    connectors: HashMap<String, ConnectorConfig>,
    pipelines: HashMap<Uuid, StreamPipelineConfig>,
    tier_config: StorageTierConfig,
}

/// In-memory, thread-safe storage backend.
///
/// All state lives inside an `Arc<RwLock<Inner>>` so it can be cloned cheaply
/// and shared across threads/tasks.
#[derive(Clone)]
pub struct MemoryStorage {
    inner: Arc<RwLock<Inner>>,
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner::default())),
        }
    }
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_read<T, F: FnOnce(&Inner) -> T>(&self, f: F) -> T {
        let guard = self.inner.read().expect("MemoryStorage RwLock poisoned");
        f(&guard)
    }

    fn with_write<T, F: FnOnce(&mut Inner) -> T>(&self, f: F) -> T {
        let mut guard = self.inner.write().expect("MemoryStorage RwLock poisoned");
        f(&mut guard)
    }
}

impl StreamStorage for MemoryStorage {
    // ── Topic ────────────────────────────────────────────────────────────────

    fn create_topic(&self, topic: TopicInfo) -> StreamResult<()> {
        self.with_write(|s| {
            if s.topics.contains_key(&topic.name) {
                return Err(StreamError::TopicExists(topic.name.clone()));
            }
            // Pre-allocate partition logs.
            for p in 0..topic.partitions {
                s.logs
                    .entry((topic.name.clone(), p))
                    .or_insert_with(|| PartitionLog::new(topic.name.clone(), p));
            }
            s.topics.insert(topic.name.clone(), topic);
            Ok(())
        })
    }

    fn delete_topic(&self, name: &str) -> StreamResult<()> {
        self.with_write(|s| {
            if s.topics.remove(name).is_none() {
                return Err(StreamError::TopicNotFound(name.into()));
            }
            s.logs.retain(|(t, _), _| t != name);
            Ok(())
        })
    }

    fn get_topic(&self, name: &str) -> StreamResult<Option<TopicInfo>> {
        Ok(self.with_read(|s| s.topics.get(name).cloned()))
    }

    fn list_topics(&self) -> StreamResult<Vec<TopicInfo>> {
        Ok(self.with_read(|s| s.topics.values().cloned().collect()))
    }

    fn update_topic(&self, topic: TopicInfo) -> StreamResult<()> {
        self.with_write(|s| {
            if !s.topics.contains_key(&topic.name) {
                return Err(StreamError::TopicNotFound(topic.name.clone()));
            }
            s.topics.insert(topic.name.clone(), topic);
            Ok(())
        })
    }

    // ── Partition log ────────────────────────────────────────────────────────

    fn append_to_partition(
        &self,
        topic: &str,
        partition: u32,
        record: crate::models::Record,
    ) -> StreamResult<i64> {
        self.with_write(|s| {
            let log = s
                .logs
                .get_mut(&(topic.to_string(), partition))
                .ok_or_else(|| StreamError::PartitionNotFound {
                    topic: topic.into(),
                    partition,
                })?;
            Ok(log.append(record))
        })
    }

    fn fetch_from_partition(
        &self,
        topic: &str,
        partition: u32,
        offset: i64,
        max_count: usize,
    ) -> StreamResult<Vec<crate::models::Record>> {
        self.with_read(|s| {
            let log = s
                .logs
                .get(&(topic.to_string(), partition))
                .ok_or_else(|| StreamError::PartitionNotFound {
                    topic: topic.into(),
                    partition,
                })?;
            Ok(log.fetch(offset, max_count))
        })
    }

    fn high_watermark(&self, topic: &str, partition: u32) -> StreamResult<i64> {
        self.with_read(|s| {
            Ok(s.logs
                .get(&(topic.to_string(), partition))
                .map(|l| l.high_watermark)
                .unwrap_or(0))
        })
    }

    fn log_start_offset(&self, topic: &str, partition: u32) -> StreamResult<i64> {
        self.with_read(|s| {
            Ok(s.logs
                .get(&(topic.to_string(), partition))
                .map(|l| l.log_start_offset)
                .unwrap_or(0))
        })
    }

    fn replace_partition_log(
        &self,
        topic: &str,
        partition: u32,
        log: PartitionLog,
    ) -> StreamResult<()> {
        self.with_write(|s| {
            s.logs.insert((topic.to_string(), partition), log);
            Ok(())
        })
    }

    fn get_partition_log(&self, topic: &str, partition: u32) -> StreamResult<Option<PartitionLog>> {
        Ok(self.with_read(|s| s.logs.get(&(topic.to_string(), partition)).cloned()))
    }

    // ── Consumer group offsets ───────────────────────────────────────────────

    fn commit_offset(
        &self,
        group: &str,
        topic: &str,
        partition: u32,
        offset: i64,
    ) -> StreamResult<()> {
        self.with_write(|s| {
            s.offsets
                .insert((group.into(), topic.into(), partition), offset);
            Ok(())
        })
    }

    fn get_offset(&self, group: &str, topic: &str, partition: u32) -> StreamResult<i64> {
        Ok(self.with_read(|s| {
            *s.offsets
                .get(&(group.to_string(), topic.to_string(), partition))
                .unwrap_or(&0)
        }))
    }

    // ── Consumer group management ────────────────────────────────────────────

    fn get_or_create_group(&self, group_id: &str) -> StreamResult<ConsumerGroup> {
        self.with_write(|s| {
            Ok(s.groups
                .entry(group_id.to_string())
                .or_insert_with(|| ConsumerGroup::new(group_id))
                .clone())
        })
    }

    fn update_group(&self, group: ConsumerGroup) -> StreamResult<()> {
        self.with_write(|s| {
            s.groups.insert(group.group_id.clone(), group);
            Ok(())
        })
    }

    fn get_group(&self, group_id: &str) -> StreamResult<Option<ConsumerGroup>> {
        Ok(self.with_read(|s| s.groups.get(group_id).cloned()))
    }

    fn list_groups(&self) -> StreamResult<Vec<ConsumerGroup>> {
        Ok(self.with_read(|s| s.groups.values().cloned().collect()))
    }

    fn delete_group(&self, group_id: &str) -> StreamResult<()> {
        self.with_write(|s| {
            s.groups.remove(group_id);
            s.offsets.retain(|(g, _, _), _| g != group_id);
            Ok(())
        })
    }

    // ── Idempotent producer ──────────────────────────────────────────────────

    fn allocate_producer_id(&self) -> StreamResult<i64> {
        self.with_write(|s| {
            let id = s.next_producer_id;
            s.next_producer_id += 1;
            Ok(id)
        })
    }

    fn get_producer_state(&self, producer_id: i64) -> StreamResult<Option<ProducerState>> {
        Ok(self.with_read(|s| s.producers.get(&producer_id).cloned()))
    }

    fn update_producer_state(&self, state: ProducerState) -> StreamResult<()> {
        self.with_write(|s| {
            s.producers.insert(state.producer_id, state);
            Ok(())
        })
    }

    // ── Transactions ─────────────────────────────────────────────────────────

    fn begin_transaction(&self, txn: Transaction) -> StreamResult<()> {
        self.with_write(|s| {
            s.transactions.insert(txn.transactional_id.clone(), txn);
            Ok(())
        })
    }

    fn get_transaction(&self, txn_id: &str) -> StreamResult<Option<Transaction>> {
        Ok(self.with_read(|s| s.transactions.get(txn_id).cloned()))
    }

    fn update_transaction(&self, txn: Transaction) -> StreamResult<()> {
        self.with_write(|s| {
            s.transactions.insert(txn.transactional_id.clone(), txn);
            Ok(())
        })
    }

    // ── Schema registry ──────────────────────────────────────────────────────

    fn register_schema(&self, schema: Schema) -> StreamResult<u32> {
        self.with_write(|s| {
            let id = schema.id;
            let subject = schema.subject.clone();
            let version = schema.version;
            s.schemas.insert(id, schema);
            s.schema_versions.insert((subject.clone(), version), id);
            let latest = s.subject_latest.entry(subject).or_insert(0);
            if version > *latest {
                *latest = version;
            }
            Ok(id)
        })
    }

    fn get_schema(&self, id: u32) -> StreamResult<Option<Schema>> {
        Ok(self.with_read(|s| s.schemas.get(&id).cloned()))
    }

    fn get_latest_schema(&self, subject: &str) -> StreamResult<Option<Schema>> {
        self.with_read(|s| {
            let version = match s.subject_latest.get(subject) {
                Some(v) => *v,
                None => return Ok(None),
            };
            let id = s
                .schema_versions
                .get(&(subject.to_string(), version))
                .copied();
            Ok(id.and_then(|i| s.schemas.get(&i).cloned()))
        })
    }

    fn get_schema_by_version(&self, subject: &str, version: u32) -> StreamResult<Option<Schema>> {
        self.with_read(|s| {
            let id = s
                .schema_versions
                .get(&(subject.to_string(), version))
                .copied();
            Ok(id.and_then(|i| s.schemas.get(&i).cloned()))
        })
    }

    fn list_subject_versions(&self, subject: &str) -> StreamResult<Vec<u32>> {
        Ok(self.with_read(|s| {
            let mut versions: Vec<u32> = s
                .schema_versions
                .keys()
                .filter(|(sub, _)| sub == subject)
                .map(|(_, v)| *v)
                .collect();
            versions.sort_unstable();
            versions
        }))
    }

    fn list_subjects(&self) -> StreamResult<Vec<String>> {
        Ok(self.with_read(|s| {
            let mut subjects: Vec<String> = s.subject_latest.keys().cloned().collect();
            subjects.sort();
            subjects
        }))
    }

    fn delete_schema(&self, subject: &str, version: u32) -> StreamResult<()> {
        self.with_write(|s| {
            let id = s
                .schema_versions
                .remove(&(subject.to_string(), version))
                .ok_or_else(|| StreamError::SubjectNotFound(subject.into()))?;
            s.schemas.remove(&id);
            // Update latest pointer if needed.
            if let Some(latest) = s.subject_latest.get(subject).copied() {
                if latest == version {
                    let new_latest = s
                        .schema_versions
                        .keys()
                        .filter(|(sub, _)| sub == subject)
                        .map(|(_, v)| *v)
                        .max()
                        .unwrap_or(0);
                    if new_latest == 0 {
                        s.subject_latest.remove(subject);
                    } else {
                        s.subject_latest.insert(subject.to_string(), new_latest);
                    }
                }
            }
            Ok(())
        })
    }

    fn get_subject_compat(&self, subject: &str) -> StreamResult<CompatibilityMode> {
        Ok(self.with_read(|s| {
            s.subject_compat
                .get(subject)
                .cloned()
                .unwrap_or_default()
        }))
    }

    fn set_subject_compat(&self, subject: &str, mode: CompatibilityMode) -> StreamResult<()> {
        self.with_write(|s| {
            s.subject_compat.insert(subject.to_string(), mode);
            Ok(())
        })
    }

    fn next_schema_id(&self) -> StreamResult<u32> {
        self.with_write(|s| {
            let id = s.next_schema_id + 1;
            s.next_schema_id = id;
            Ok(id)
        })
    }

    // ── Connectors ───────────────────────────────────────────────────────────

    fn create_connector(&self, cfg: ConnectorConfig) -> StreamResult<()> {
        self.with_write(|s| {
            if s.connectors.contains_key(&cfg.name) {
                return Err(StreamError::ConnectorExists(cfg.name.clone()));
            }
            s.connectors.insert(cfg.name.clone(), cfg);
            Ok(())
        })
    }

    fn get_connector(&self, name: &str) -> StreamResult<Option<ConnectorConfig>> {
        Ok(self.with_read(|s| s.connectors.get(name).cloned()))
    }

    fn list_connectors(&self) -> StreamResult<Vec<ConnectorConfig>> {
        Ok(self.with_read(|s| s.connectors.values().cloned().collect()))
    }

    fn update_connector(&self, cfg: ConnectorConfig) -> StreamResult<()> {
        self.with_write(|s| {
            if !s.connectors.contains_key(&cfg.name) {
                return Err(StreamError::ConnectorNotFound(cfg.name.clone()));
            }
            s.connectors.insert(cfg.name.clone(), cfg);
            Ok(())
        })
    }

    fn delete_connector(&self, name: &str) -> StreamResult<()> {
        self.with_write(|s| {
            s.connectors
                .remove(name)
                .ok_or_else(|| StreamError::ConnectorNotFound(name.into()))
                .map(|_| ())
        })
    }

    // ── Pipelines ────────────────────────────────────────────────────────────

    fn create_pipeline(&self, cfg: StreamPipelineConfig) -> StreamResult<()> {
        self.with_write(|s| {
            s.pipelines.insert(cfg.id, cfg);
            Ok(())
        })
    }

    fn get_pipeline(&self, id: Uuid) -> StreamResult<Option<StreamPipelineConfig>> {
        Ok(self.with_read(|s| s.pipelines.get(&id).cloned()))
    }

    fn list_pipelines(&self) -> StreamResult<Vec<StreamPipelineConfig>> {
        Ok(self.with_read(|s| s.pipelines.values().cloned().collect()))
    }

    fn update_pipeline(&self, cfg: StreamPipelineConfig) -> StreamResult<()> {
        self.with_write(|s| {
            if !s.pipelines.contains_key(&cfg.id) {
                return Err(StreamError::PipelineNotFound(cfg.id.to_string()));
            }
            s.pipelines.insert(cfg.id, cfg);
            Ok(())
        })
    }

    fn delete_pipeline(&self, id: Uuid) -> StreamResult<()> {
        self.with_write(|s| {
            s.pipelines
                .remove(&id)
                .ok_or_else(|| StreamError::PipelineNotFound(id.to_string()))
                .map(|_| ())
        })
    }

    // ── Tiered storage config ─────────────────────────────────────────────────

    fn get_tier_config(&self) -> StreamResult<StorageTierConfig> {
        Ok(self.with_read(|s| s.tier_config.clone()))
    }

    fn set_tier_config(&self, cfg: StorageTierConfig) -> StreamResult<()> {
        self.with_write(|s| {
            s.tier_config = cfg;
            Ok(())
        })
    }
}

// ─── PostgreSQL storage skeleton ──────────────────────────────────────────────

/// Durable PostgreSQL-backed storage.
///
/// ## cave-db integration
///
/// Wire in the `cave_db::CavePool` as the backing store once SQL migrations are
/// ready.  Example (not compiled unless the `postgres` feature is enabled):
///
/// ```ignore
/// use cave_db::CavePool;
/// use cave_streams::PostgresStorage;
///
/// let pool = CavePool::new(&config.database).expect("pool");
/// // run cave_streams migrations:
/// cave_db::migrate::run_migrations(&pool, "streams", MIGRATIONS).await?;
/// let storage = PostgresStorage::with_pool(pool);
/// ```
///
/// Until full SQL persistence is wired every method delegates to the in-memory
/// cache, making `PostgresStorage` a drop-in replacement with durability added
/// incrementally.
pub struct PostgresStorage {
    /// In-memory cache used until full SQL persistence is wired.
    cache: MemoryStorage,
}

impl PostgresStorage {
    pub fn new() -> Self {
        Self {
            cache: MemoryStorage::new(),
        }
    }
}

impl Default for PostgresStorage {
    fn default() -> Self {
        Self::new()
    }
}

// Delegate everything to the in-memory cache until SQL persistence is
// implemented.  Each method can be replaced incrementally.
macro_rules! delegate {
    ($method:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty) => {
        fn $method(&self, $($arg: $ty),*) -> $ret {
            self.cache.$method($($arg),*)
        }
    };
}

impl StreamStorage for PostgresStorage {
    delegate!(create_topic(topic: TopicInfo) -> StreamResult<()>);
    delegate!(delete_topic(name: &str) -> StreamResult<()>);
    delegate!(get_topic(name: &str) -> StreamResult<Option<TopicInfo>>);
    delegate!(list_topics() -> StreamResult<Vec<TopicInfo>>);
    delegate!(update_topic(topic: TopicInfo) -> StreamResult<()>);

    fn append_to_partition(
        &self,
        topic: &str,
        partition: u32,
        record: crate::models::Record,
    ) -> StreamResult<i64> {
        self.cache.append_to_partition(topic, partition, record)
    }

    fn fetch_from_partition(
        &self,
        topic: &str,
        partition: u32,
        offset: i64,
        max_count: usize,
    ) -> StreamResult<Vec<crate::models::Record>> {
        self.cache.fetch_from_partition(topic, partition, offset, max_count)
    }

    delegate!(high_watermark(topic: &str, partition: u32) -> StreamResult<i64>);
    delegate!(log_start_offset(topic: &str, partition: u32) -> StreamResult<i64>);
    delegate!(replace_partition_log(topic: &str, partition: u32, log: PartitionLog) -> StreamResult<()>);
    delegate!(get_partition_log(topic: &str, partition: u32) -> StreamResult<Option<PartitionLog>>);
    delegate!(commit_offset(group: &str, topic: &str, partition: u32, offset: i64) -> StreamResult<()>);
    delegate!(get_offset(group: &str, topic: &str, partition: u32) -> StreamResult<i64>);
    delegate!(get_or_create_group(group_id: &str) -> StreamResult<ConsumerGroup>);
    delegate!(update_group(group: ConsumerGroup) -> StreamResult<()>);
    delegate!(get_group(group_id: &str) -> StreamResult<Option<ConsumerGroup>>);
    delegate!(list_groups() -> StreamResult<Vec<ConsumerGroup>>);
    delegate!(delete_group(group_id: &str) -> StreamResult<()>);
    delegate!(allocate_producer_id() -> StreamResult<i64>);
    delegate!(get_producer_state(producer_id: i64) -> StreamResult<Option<ProducerState>>);
    delegate!(update_producer_state(state: ProducerState) -> StreamResult<()>);
    delegate!(begin_transaction(txn: Transaction) -> StreamResult<()>);
    delegate!(get_transaction(txn_id: &str) -> StreamResult<Option<Transaction>>);
    delegate!(update_transaction(txn: Transaction) -> StreamResult<()>);
    delegate!(register_schema(schema: Schema) -> StreamResult<u32>);
    delegate!(get_schema(id: u32) -> StreamResult<Option<Schema>>);
    delegate!(get_latest_schema(subject: &str) -> StreamResult<Option<Schema>>);
    delegate!(get_schema_by_version(subject: &str, version: u32) -> StreamResult<Option<Schema>>);
    delegate!(list_subject_versions(subject: &str) -> StreamResult<Vec<u32>>);
    delegate!(list_subjects() -> StreamResult<Vec<String>>);
    delegate!(delete_schema(subject: &str, version: u32) -> StreamResult<()>);
    delegate!(get_subject_compat(subject: &str) -> StreamResult<CompatibilityMode>);
    delegate!(set_subject_compat(subject: &str, mode: CompatibilityMode) -> StreamResult<()>);
    delegate!(next_schema_id() -> StreamResult<u32>);
    delegate!(create_connector(cfg: ConnectorConfig) -> StreamResult<()>);
    delegate!(get_connector(name: &str) -> StreamResult<Option<ConnectorConfig>>);
    delegate!(list_connectors() -> StreamResult<Vec<ConnectorConfig>>);
    delegate!(update_connector(cfg: ConnectorConfig) -> StreamResult<()>);
    delegate!(delete_connector(name: &str) -> StreamResult<()>);
    delegate!(create_pipeline(cfg: StreamPipelineConfig) -> StreamResult<()>);
    delegate!(get_pipeline(id: Uuid) -> StreamResult<Option<StreamPipelineConfig>>);
    delegate!(list_pipelines() -> StreamResult<Vec<StreamPipelineConfig>>);
    delegate!(update_pipeline(cfg: StreamPipelineConfig) -> StreamResult<()>);
    delegate!(delete_pipeline(id: Uuid) -> StreamResult<()>);
    delegate!(get_tier_config() -> StreamResult<StorageTierConfig>);
    delegate!(set_tier_config(cfg: StorageTierConfig) -> StreamResult<()>);
}
