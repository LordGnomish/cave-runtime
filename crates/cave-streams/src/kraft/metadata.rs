// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `MetadataRecord` — one entry in the compacted metadata log.
//!
//! Mirrors `org.apache.kafka.common.metadata.*Record` from
//! upstream — every cluster-metadata mutation is expressed as
//! one of these records and replicated through the KRaft quorum.
//!
//! cave-streams ports the four record types that the running
//! broker actually consumes today (topic + partition lifecycle +
//! broker membership + config). The full KIP-595 record schema
//! includes Access Control, Producer ID, and DelegationToken
//! records — those are tracked-not-shipped, intentionally out of
//! scope until the matching surfaces in the broker are wired
//! through the metadata log.

use std::collections::BTreeMap;

use super::epoch::ControllerEpoch;

/// Identifier of which metadata record we have in hand. The
/// compactor uses this — only the latest record per key wins.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataKey {
    Topic(String),
    /// Partition keyed by `(topic, partition_id)`.
    Partition(String, i32),
    Broker(i32),
    /// Configuration entry keyed by `(scope, target, key)`.
    /// `scope` is e.g. "broker" or "topic", `target` is the
    /// broker-id or topic-name the config applies to.
    Config(String, String, String),
}

/// `TopicRecord` — KIP-595 §3.2. Created when a topic is
/// created, deleted when the topic is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicRecord {
    pub name: String,
    /// Stable UUID — survives topic re-creation under the same
    /// name (so consumers can detect "different topic, same
    /// name").
    pub topic_id: uuid::Uuid,
    pub partition_count: i32,
    pub replication_factor: i16,
}

/// `PartitionRecord` — current leader + ISR for one partition.
/// Emitted on partition creation and on every leadership change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionRecord {
    pub topic: String,
    pub partition_id: i32,
    pub leader: i32,
    /// In-sync replica broker IDs.
    pub isr: Vec<i32>,
    /// Replica broker IDs (ISR ⊆ replicas).
    pub replicas: Vec<i32>,
    /// Leader epoch — bumps on every leadership change.
    pub leader_epoch: i32,
}

/// `BrokerRegistration` — KIP-595 §3.1. A broker registers
/// itself with the controller on start; the record carries the
/// broker's endpoints and an incarnation ID that distinguishes
/// restarts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerRegistration {
    pub broker_id: i32,
    pub host: String,
    pub port: u16,
    /// `IncarnationId` — fresh UUID per process start. Used to
    /// detect zombie brokers after a network partition.
    pub incarnation_id: uuid::Uuid,
    /// Whether the broker has finished its startup fence and
    /// can serve requests.
    pub fenced: bool,
}

/// `ConfigRecord` — upsert of one configuration key. `value =
/// None` represents a delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRecord {
    pub scope: String,
    pub target: String,
    pub key: String,
    pub value: Option<String>,
}

/// The discriminated union the metadata log stores. Each
/// variant carries its own payload plus the epoch at which it
/// was appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataRecord {
    Topic {
        epoch: ControllerEpoch,
        record: TopicRecord,
    },
    /// Topic removal — used by the compactor to expire all
    /// records keyed by this topic name.
    TopicRemoved {
        epoch: ControllerEpoch,
        name: String,
    },
    Partition {
        epoch: ControllerEpoch,
        record: PartitionRecord,
    },
    Broker {
        epoch: ControllerEpoch,
        record: BrokerRegistration,
    },
    BrokerUnregistered {
        epoch: ControllerEpoch,
        broker_id: i32,
    },
    Config {
        epoch: ControllerEpoch,
        record: ConfigRecord,
    },
}

impl MetadataRecord {
    pub fn key(&self) -> MetadataKey {
        match self {
            MetadataRecord::Topic { record, .. } => MetadataKey::Topic(record.name.clone()),
            MetadataRecord::TopicRemoved { name, .. } => MetadataKey::Topic(name.clone()),
            MetadataRecord::Partition { record, .. } => {
                MetadataKey::Partition(record.topic.clone(), record.partition_id)
            }
            MetadataRecord::Broker { record, .. } => MetadataKey::Broker(record.broker_id),
            MetadataRecord::BrokerUnregistered { broker_id, .. } => MetadataKey::Broker(*broker_id),
            MetadataRecord::Config { record, .. } => MetadataKey::Config(
                record.scope.clone(),
                record.target.clone(),
                record.key.clone(),
            ),
        }
    }

    pub fn epoch(&self) -> ControllerEpoch {
        match self {
            MetadataRecord::Topic { epoch, .. }
            | MetadataRecord::TopicRemoved { epoch, .. }
            | MetadataRecord::Partition { epoch, .. }
            | MetadataRecord::Broker { epoch, .. }
            | MetadataRecord::BrokerUnregistered { epoch, .. }
            | MetadataRecord::Config { epoch, .. } => *epoch,
        }
    }

    /// `true` if this record represents a removal — used by
    /// the compactor to drop predecessor records keyed the same
    /// way.
    pub fn is_tombstone(&self) -> bool {
        matches!(
            self,
            MetadataRecord::TopicRemoved { .. }
                | MetadataRecord::BrokerUnregistered { .. }
                | MetadataRecord::Config {
                    record: ConfigRecord { value: None, .. },
                    ..
                }
        )
    }
}

/// Snapshot of the materialised cluster state — the result of
/// folding the metadata log through the compactor. The
/// controller exposes this view to broker clients and to the
/// `kafka_wire` metadata-response builder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClusterMetadata {
    pub topics: BTreeMap<String, TopicRecord>,
    /// Keyed by `(topic, partition)`.
    pub partitions: BTreeMap<(String, i32), PartitionRecord>,
    pub brokers: BTreeMap<i32, BrokerRegistration>,
    /// Keyed by `(scope, target, key)`.
    pub configs: BTreeMap<(String, String, String), String>,
}

impl ClusterMetadata {
    /// Apply a single record to the snapshot.
    pub fn apply(&mut self, rec: &MetadataRecord) {
        match rec {
            MetadataRecord::Topic { record, .. } => {
                self.topics.insert(record.name.clone(), record.clone());
            }
            MetadataRecord::TopicRemoved { name, .. } => {
                self.topics.remove(name);
                self.partitions.retain(|(t, _), _| t != name);
            }
            MetadataRecord::Partition { record, .. } => {
                self.partitions
                    .insert((record.topic.clone(), record.partition_id), record.clone());
            }
            MetadataRecord::Broker { record, .. } => {
                self.brokers.insert(record.broker_id, record.clone());
            }
            MetadataRecord::BrokerUnregistered { broker_id, .. } => {
                self.brokers.remove(broker_id);
            }
            MetadataRecord::Config { record, .. } => {
                let k = (
                    record.scope.clone(),
                    record.target.clone(),
                    record.key.clone(),
                );
                match &record.value {
                    Some(v) => {
                        self.configs.insert(k, v.clone());
                    }
                    None => {
                        self.configs.remove(&k);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(name: &str) -> MetadataRecord {
        MetadataRecord::Topic {
            epoch: ControllerEpoch(1),
            record: TopicRecord {
                name: name.into(),
                topic_id: uuid::Uuid::new_v4(),
                partition_count: 3,
                replication_factor: 2,
            },
        }
    }

    fn partition(t: &str, p: i32, leader: i32) -> MetadataRecord {
        MetadataRecord::Partition {
            epoch: ControllerEpoch(1),
            record: PartitionRecord {
                topic: t.into(),
                partition_id: p,
                leader,
                isr: vec![leader],
                replicas: vec![leader],
                leader_epoch: 0,
            },
        }
    }

    #[test]
    fn record_key_distinguishes_variants() {
        let t = topic("orders");
        let p = partition("orders", 0, 1);
        let b = MetadataRecord::Broker {
            epoch: ControllerEpoch(1),
            record: BrokerRegistration {
                broker_id: 1,
                host: "h".into(),
                port: 9092,
                incarnation_id: uuid::Uuid::new_v4(),
                fenced: false,
            },
        };
        assert_eq!(t.key(), MetadataKey::Topic("orders".into()));
        assert_eq!(p.key(), MetadataKey::Partition("orders".into(), 0));
        assert_eq!(b.key(), MetadataKey::Broker(1));
    }

    #[test]
    fn topic_removed_is_tombstone() {
        let r = MetadataRecord::TopicRemoved {
            epoch: ControllerEpoch(1),
            name: "x".into(),
        };
        assert!(r.is_tombstone());
        let t = topic("y");
        assert!(!t.is_tombstone());
    }

    #[test]
    fn cluster_metadata_apply_adds_topic() {
        let mut m = ClusterMetadata::default();
        m.apply(&topic("orders"));
        assert!(m.topics.contains_key("orders"));
        assert_eq!(m.topics["orders"].partition_count, 3);
    }

    #[test]
    fn cluster_metadata_apply_partition_keyed_correctly() {
        let mut m = ClusterMetadata::default();
        m.apply(&partition("orders", 0, 1));
        m.apply(&partition("orders", 1, 2));
        m.apply(&partition("payments", 0, 1));
        assert_eq!(m.partitions.len(), 3);
        assert_eq!(
            m.partitions[&("orders".into(), 1)].leader,
            2
        );
    }

    #[test]
    fn topic_removed_cascades_to_partitions() {
        let mut m = ClusterMetadata::default();
        m.apply(&topic("orders"));
        m.apply(&partition("orders", 0, 1));
        m.apply(&partition("orders", 1, 2));
        m.apply(&topic("payments"));
        m.apply(&partition("payments", 0, 1));
        m.apply(&MetadataRecord::TopicRemoved {
            epoch: ControllerEpoch(1),
            name: "orders".into(),
        });
        assert!(!m.topics.contains_key("orders"));
        assert!(m.topics.contains_key("payments"));
        // payments/0 must survive; both orders/* must be gone.
        assert_eq!(m.partitions.len(), 1);
        assert!(m.partitions.contains_key(&("payments".into(), 0)));
    }

    #[test]
    fn config_value_none_deletes() {
        let mut m = ClusterMetadata::default();
        let set = MetadataRecord::Config {
            epoch: ControllerEpoch(1),
            record: ConfigRecord {
                scope: "broker".into(),
                target: "1".into(),
                key: "log.retention.ms".into(),
                value: Some("86400000".into()),
            },
        };
        let unset = MetadataRecord::Config {
            epoch: ControllerEpoch(1),
            record: ConfigRecord {
                scope: "broker".into(),
                target: "1".into(),
                key: "log.retention.ms".into(),
                value: None,
            },
        };
        m.apply(&set);
        assert_eq!(m.configs.len(), 1);
        m.apply(&unset);
        assert!(m.configs.is_empty());
    }
}
