// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pulsar partitioned-topic metadata + persistent vs non-persistent
//! semantics.
//!
//! A *partitioned* topic in Pulsar is a virtual umbrella over N internal
//! topics named `<topic>-partition-0`, `<topic>-partition-1`, …,
//! `<topic>-partition-(N-1)`.  Producers route by key (sticky) or
//! round-robin; consumers see the partitions as one logical topic.
//!
//! Mirrors Apache Pulsar 4.2.0
//!   `pulsar-broker/src/main/java/org/apache/pulsar/broker/admin/impl/PersistentTopicsBase.java`
//!   `pulsar-common/src/main/java/org/apache/pulsar/common/partition/PartitionedTopicMetadata.java`

use crate::error::{StreamsError, StreamsResult};
use crate::pulsar_topic::{TopicDomain, TopicName};
use crate::tenant::TenantRegistry;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// `PartitionedTopicMetadata` shape.  When `partitions == 0` the topic is
/// not partitioned (a regular single-partition topic).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionedTopicMetadata {
    pub partitions: u32,
    /// `properties` mirrors Pulsar admin metadata blobs.
    #[serde(default)]
    pub properties: std::collections::BTreeMap<String, String>,
}

/// Routing mode for partitioned-topic producers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartitionRoutingMode {
    /// Key-hash router: same key always lands on the same partition.
    SinglePartitionByKey,
    /// Round-robin partition selection (used when no key is set).
    RoundRobin,
    /// Pin all messages to a single partition for the producer's lifetime.
    SinglePartition,
}

/// Per-namespace partitioned-topic registry.
pub struct PartitionedTopicRegistry {
    pub tenants: Arc<TenantRegistry>,
    /// Key = fully-qualified `persistent://t/ns/local`.
    metadata: DashMap<String, PartitionedTopicMetadata>,
    /// Key = same; value = next round-robin index for the producer.
    rr_cursor: DashMap<String, u32>,
}

impl PartitionedTopicRegistry {
    pub fn new(tenants: Arc<TenantRegistry>) -> Self {
        Self {
            tenants,
            metadata: DashMap::new(),
            rr_cursor: DashMap::new(),
        }
    }

    /// Create a partitioned topic with N partitions.  Validates that
    /// `n >= 1` and that the namespace exists.
    pub fn create_partitioned_topic(
        &self,
        topic: &TopicName,
        partitions: u32,
    ) -> StreamsResult<()> {
        if partitions == 0 {
            return Err(StreamsError::InvalidTopicName(format!(
                "partitions must be ≥ 1, got 0 for {topic}"
            )));
        }
        self.tenants
            .ensure_namespace(&topic.tenant, &topic.namespace)?;
        let key = topic.to_string_full();
        if self.metadata.contains_key(&key) {
            return Err(StreamsError::Internal(format!(
                "partitioned topic already exists: {key}"
            )));
        }
        self.metadata.insert(
            key,
            PartitionedTopicMetadata {
                partitions,
                properties: Default::default(),
            },
        );
        Ok(())
    }

    pub fn delete_partitioned_topic(&self, topic: &TopicName) -> StreamsResult<()> {
        let key = topic.to_string_full();
        self.metadata
            .remove(&key)
            .ok_or_else(|| StreamsError::Internal(format!("not found: {key}")))?;
        self.rr_cursor.remove(&key);
        Ok(())
    }

    /// Return the umbrella topic metadata.  `None` for non-partitioned.
    pub fn get_metadata(&self, topic: &TopicName) -> Option<PartitionedTopicMetadata> {
        self.metadata
            .get(&topic.to_string_full())
            .map(|r| r.clone())
    }

    /// Return all child partition `TopicName`s for a partitioned topic.
    pub fn list_partitions(&self, topic: &TopicName) -> StreamsResult<Vec<TopicName>> {
        let meta = self.get_metadata(topic).ok_or_else(|| {
            StreamsError::Internal(format!("not partitioned: {}", topic.to_string_full()))
        })?;
        let mut out = Vec::with_capacity(meta.partitions as usize);
        for i in 0..meta.partitions as i32 {
            out.push(topic.partition_of(i)?);
        }
        Ok(out)
    }

    /// Pick the partition `TopicName` to route a message to.
    pub fn route_message(
        &self,
        topic: &TopicName,
        mode: PartitionRoutingMode,
        key: Option<&[u8]>,
    ) -> StreamsResult<TopicName> {
        let meta = self.get_metadata(topic).ok_or_else(|| {
            StreamsError::Internal(format!("not partitioned: {}", topic.to_string_full()))
        })?;
        let n = meta.partitions;
        let pick = match mode {
            PartitionRoutingMode::SinglePartitionByKey => {
                let key = key.ok_or_else(|| {
                    StreamsError::InvalidTopicName("SinglePartitionByKey requires a key".into())
                })?;
                key_hash(key) % n
            }
            PartitionRoutingMode::RoundRobin => {
                let mut cur = self.rr_cursor.entry(topic.to_string_full()).or_insert(0);
                let pick = *cur % n;
                *cur = (*cur + 1) % n;
                pick
            }
            PartitionRoutingMode::SinglePartition => 0,
        };
        topic.partition_of(pick as i32)
    }

    /// `true` when the topic is partitioned (i.e. registered here).
    pub fn is_partitioned(&self, topic: &TopicName) -> bool {
        self.metadata.contains_key(&topic.to_string_full())
    }
}

fn key_hash(bytes: &[u8]) -> u32 {
    // Same FNV-1a 32-bit hash Pulsar uses for default-routing partition
    // selection (`pulsar-client/.../impl/conf/ProducerConfigurationData`
    // uses Murmur3 in production; FNV is a deterministic stand-in here).
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

// ── Persistent vs non-persistent semantics ────────────────────────────────

/// Per-topic persistence policy.  In Pulsar, the policy is encoded in the
/// scheme of the [`TopicName`] (`persistent://` vs `non-persistent://`);
/// cave-streams enforces the matching dispatch rules at this level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistencePolicy {
    /// Backed by the Bookkeeper-style segment log.  Messages survive
    /// broker restart and are subject to retention.
    Persistent,
    /// In-memory only.  Messages are dropped when no consumer is ready
    /// to receive them, and survive only as long as the broker process.
    NonPersistent,
}

impl PersistencePolicy {
    pub fn for_topic(t: &TopicName) -> Self {
        match t.domain {
            TopicDomain::Persistent => Self::Persistent,
            TopicDomain::NonPersistent => Self::NonPersistent,
        }
    }

    /// Returns `true` when a message should be dropped if no consumer is
    /// currently registered.  Persistent topics never drop on no-consumer
    /// (the message stays in the log); non-persistent topics drop.
    pub fn drop_when_no_consumer(self) -> bool {
        matches!(self, Self::NonPersistent)
    }

    /// Returns `true` when retention/cleanup machinery applies.  Only
    /// persistent topics participate in retention sweeps.
    pub fn participates_in_retention(self) -> bool {
        matches!(self, Self::Persistent)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Partitioned-topic + persistence tests — feat/cave-streams-deeper-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(_tenant_id: &str) -> PartitionedTopicRegistry {
        let tenants = Arc::new(TenantRegistry::default());
        PartitionedTopicRegistry::new(tenants)
    }

    #[test]
    fn test_create_partitioned_topic_with_n() {
        // cite: pulsar 4.2.0 .../impl/PersistentTopicsBase#createPartitionedTopic
        let tenant_id = "pt-001";
        let r = registry(tenant_id);
        let t = TopicName::persistent(tenant_id, "ns", "orders").unwrap();
        r.create_partitioned_topic(&t, 4).unwrap();
        let meta = r.get_metadata(&t).unwrap();
        assert_eq!(meta.partitions, 4);
        assert!(r.is_partitioned(&t));
    }

    #[test]
    fn test_create_partitioned_topic_zero_rejected() {
        // cite: pulsar 4.2.0 (partitions must be ≥ 1)
        let tenant_id = "pt-002";
        let r = registry(tenant_id);
        let t = TopicName::persistent(tenant_id, "ns", "x").unwrap();
        let err = r.create_partitioned_topic(&t, 0);
        assert!(err.is_err());
    }

    #[test]
    fn test_list_partitions_returns_n_children() {
        // cite: pulsar 4.2.0 (TopicName.getPartition(idx) for 0..N)
        let tenant_id = "pt-003";
        let r = registry(tenant_id);
        let t = TopicName::persistent(tenant_id, "ns", "events").unwrap();
        r.create_partitioned_topic(&t, 5).unwrap();
        let parts = r.list_partitions(&t).unwrap();
        assert_eq!(parts.len(), 5);
        for (i, p) in parts.iter().enumerate() {
            assert_eq!(p.partition, Some(i as i32));
        }
    }

    #[test]
    fn test_route_message_round_robin_cycles() {
        // cite: pulsar 4.2.0 RoundRobinPartitionMessageRouterImpl
        let tenant_id = "pt-004";
        let r = registry(tenant_id);
        let t = TopicName::persistent(tenant_id, "ns", "rr").unwrap();
        r.create_partitioned_topic(&t, 3).unwrap();
        let p1 = r
            .route_message(&t, PartitionRoutingMode::RoundRobin, None)
            .unwrap();
        let p2 = r
            .route_message(&t, PartitionRoutingMode::RoundRobin, None)
            .unwrap();
        let p3 = r
            .route_message(&t, PartitionRoutingMode::RoundRobin, None)
            .unwrap();
        let p4 = r
            .route_message(&t, PartitionRoutingMode::RoundRobin, None)
            .unwrap();
        let mut got = vec![
            p1.partition.unwrap(),
            p2.partition.unwrap(),
            p3.partition.unwrap(),
        ];
        got.sort();
        assert_eq!(got, vec![0, 1, 2]);
        assert_eq!(p4.partition, p1.partition, "RR wraps around");
    }

    #[test]
    fn test_route_message_by_key_is_sticky() {
        // cite: pulsar 4.2.0 SinglePartitionMessageRouterImpl (sticky-by-hash)
        let tenant_id = "pt-005";
        let r = registry(tenant_id);
        let t = TopicName::persistent(tenant_id, "ns", "sticky").unwrap();
        r.create_partitioned_topic(&t, 7).unwrap();
        let p1 = r
            .route_message(
                &t,
                PartitionRoutingMode::SinglePartitionByKey,
                Some(b"order-77"),
            )
            .unwrap();
        let p2 = r
            .route_message(
                &t,
                PartitionRoutingMode::SinglePartitionByKey,
                Some(b"order-77"),
            )
            .unwrap();
        assert_eq!(p1.partition, p2.partition);
    }

    #[test]
    fn test_route_by_key_requires_key() {
        // cite: pulsar 4.2.0 (key-router rejects null key)
        let tenant_id = "pt-006";
        let r = registry(tenant_id);
        let t = TopicName::persistent(tenant_id, "ns", "k").unwrap();
        r.create_partitioned_topic(&t, 2).unwrap();
        let err = r.route_message(&t, PartitionRoutingMode::SinglePartitionByKey, None);
        assert!(err.is_err());
    }

    #[test]
    fn test_persistence_policy_for_persistent_topic() {
        // cite: pulsar 4.2.0 TopicDomain (persistent vs non-persistent)
        let tenant_id = "pt-007";
        let t = TopicName::persistent(tenant_id, "ns", "k").unwrap();
        assert_eq!(
            PersistencePolicy::for_topic(&t),
            PersistencePolicy::Persistent
        );
        assert!(!PersistencePolicy::for_topic(&t).drop_when_no_consumer());
        assert!(PersistencePolicy::for_topic(&t).participates_in_retention());
    }

    #[test]
    fn test_persistence_policy_for_non_persistent_topic() {
        // cite: pulsar 4.2.0 NonPersistentTopic (drop on no consumer)
        let tenant_id = "pt-008";
        let s = format!("non-persistent://{}/ns/transient", tenant_id);
        let t = TopicName::parse(&s).unwrap();
        assert_eq!(
            PersistencePolicy::for_topic(&t),
            PersistencePolicy::NonPersistent
        );
        assert!(PersistencePolicy::for_topic(&t).drop_when_no_consumer());
        assert!(!PersistencePolicy::for_topic(&t).participates_in_retention());
    }

    #[test]
    fn test_delete_partitioned_topic_removes_metadata() {
        // cite: pulsar 4.2.0 PersistentTopicsBase#deletePartitionedTopic
        let tenant_id = "pt-009";
        let r = registry(tenant_id);
        let t = TopicName::persistent(tenant_id, "ns", "doomed").unwrap();
        r.create_partitioned_topic(&t, 2).unwrap();
        r.delete_partitioned_topic(&t).unwrap();
        assert!(!r.is_partitioned(&t));
    }
}
