// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// core/src/main/scala/kafka/coordinator/group/ConsumerGroupCoordinator.scala
// (record schemas written to __consumer_offsets compacted topic)
//
//! Persistence records for the KIP-848 protocol. All records carry
//! `(group_id, member_id?)` as the natural compaction key so that
//! tombstones (encoded as empty payload, via [`PersistenceEntry::Tombstone`])
//! delete the latest value.

use serde::{Deserialize, Serialize};

/// `(topic, [partitions])` carrier — used in both assignment records
/// and the response wire frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicPartitions {
    pub topic: String,
    pub partitions: Vec<i32>,
}

/// Subscription clause for a member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MemberSubscription {
    pub topic_names: Vec<String>,
    pub topic_regex: Option<String>,
}

/// Group-level record. Compaction key: `("consumer_group", group_id)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerGroupRecord {
    pub group_id: String,
    pub group_epoch: i32,
    /// Snapshot of `(topic, partition_count)` known when this epoch
    /// was promoted; rebuilt on every group-epoch bump.
    pub topic_partition_metadata: Vec<(String, i32)>,
}

/// Per-member record. Compaction key: `("member", group_id, member_id)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRecord {
    pub group_id: String,
    pub member_id: String,
    pub instance_id: Option<String>,
    pub member_epoch: i32,
    pub subscription: MemberSubscription,
    pub rack_id: Option<String>,
}

/// Target-assignment record. Compaction key:
/// `("target_assignment", group_id, member_id)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetAssignmentRecord {
    pub group_id: String,
    pub member_id: String,
    pub group_epoch: i32,
    pub assigned: Vec<TopicPartitions>,
}

macro_rules! impl_record_serde {
    ($t:ident) => {
        impl $t {
            pub fn encode(&self) -> Vec<u8> {
                serde_json::to_vec(self).expect(stringify!($t encodes))
            }
            pub fn decode(bytes: &[u8]) -> Result<Self, String> {
                serde_json::from_slice(bytes).map_err(|e| e.to_string())
            }
        }
    };
}

impl_record_serde!(ConsumerGroupRecord);
impl_record_serde!(MemberRecord);
impl_record_serde!(TargetAssignmentRecord);

/// One entry written to the coordinator's persistence log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceEntry {
    ConsumerGroup(ConsumerGroupRecord),
    Member(MemberRecord),
    TargetAssignment(TargetAssignmentRecord),
    /// Tombstone — `(kind, key)` where `kind` is the record-type tag.
    Tombstone {
        kind: &'static str,
        group_id: String,
        member_id: Option<String>,
    },
}

impl PersistenceEntry {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ConsumerGroup(_) => "consumer_group",
            Self::Member(_) => "member",
            Self::TargetAssignment(_) => "target_assignment",
            Self::Tombstone { kind, .. } => kind,
        }
    }
    pub fn is_tombstone(&self) -> bool {
        matches!(self, Self::Tombstone { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_assignment_round_trip() {
        let r = TargetAssignmentRecord {
            group_id: "g".into(),
            member_id: "m".into(),
            group_epoch: 2,
            assigned: vec![TopicPartitions {
                topic: "t".into(),
                partitions: vec![0, 1],
            }],
        };
        assert_eq!(TargetAssignmentRecord::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn tombstone_is_tombstone() {
        let t = PersistenceEntry::Tombstone {
            kind: "member",
            group_id: "g".into(),
            member_id: Some("m".into()),
        };
        assert!(t.is_tombstone());
        assert_eq!(t.kind(), "member");
    }
}
