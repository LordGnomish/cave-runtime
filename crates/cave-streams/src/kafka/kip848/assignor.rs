// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// server-common/src/main/java/org/apache/kafka/server/group/share/UniformAssignor.java
//
//! Server-side assignor used by the KIP-848 coordinator.

use std::collections::BTreeMap;

use super::records::TopicPartitions;

/// Uniform (round-robin within each topic) assignor.
///
/// For each topic in the cluster:
///   * sort members alphabetically (stable)
///   * sort partitions ascending
///   * walk partitions in order, handing them out round-robin to members
///
/// Result: every member gets `floor(P/M)` or `floor(P/M)+1` partitions
/// of each topic, balancing across topics.
pub struct UniformAssignor;

impl UniformAssignor {
    /// `members`     – ordered list of *eligible* member ids (subscribers
    ///                  to at least one of the topics in `topics`).
    /// `topics`      – `(topic, partition_count)` snapshot.
    pub fn assign(
        &self,
        members: &[String],
        topics: &[(String, i32)],
    ) -> BTreeMap<String, Vec<TopicPartitions>> {
        let mut out: BTreeMap<String, Vec<TopicPartitions>> = BTreeMap::new();
        if members.is_empty() {
            return out;
        }
        let mut sorted_members: Vec<&String> = members.iter().collect();
        sorted_members.sort();
        let mut cursor: usize = 0;
        for (topic, count) in topics {
            if *count <= 0 {
                continue;
            }
            // Distribute this topic's partitions starting from `cursor`
            // so that remainder is spread across topics — the classic
            // upstream `UniformAssignor`. We pre-create empty buckets so
            // every member appears in the output map.
            let mut per_member: BTreeMap<String, Vec<i32>> = BTreeMap::new();
            for m in &sorted_members {
                per_member.entry((*m).clone()).or_default();
            }
            for p in 0..*count {
                let m = sorted_members[(cursor + p as usize) % sorted_members.len()];
                per_member.get_mut(m).unwrap().push(p);
            }
            cursor = (cursor + *count as usize) % sorted_members.len();
            for (m, mut parts) in per_member {
                parts.sort();
                if parts.is_empty() {
                    continue;
                }
                out.entry(m)
                    .or_default()
                    .push(TopicPartitions {
                        topic: topic.clone(),
                        partitions: parts,
                    });
            }
        }
        out
    }
}

/// Builder for emitting per-member `TargetAssignmentRecord` payloads.
pub struct TargetAssignmentBuilder {
    group_id: String,
    group_epoch: i32,
    members: BTreeMap<String, Vec<TopicPartitions>>,
}

impl TargetAssignmentBuilder {
    pub fn new(group_id: impl Into<String>, group_epoch: i32) -> Self {
        Self {
            group_id: group_id.into(),
            group_epoch,
            members: BTreeMap::new(),
        }
    }
    pub fn add(&mut self, member_id: impl Into<String>, assigned: Vec<(String, Vec<i32>)>) {
        let v: Vec<TopicPartitions> = assigned
            .into_iter()
            .map(|(t, p)| TopicPartitions {
                topic: t,
                partitions: p,
            })
            .collect();
        self.members.insert(member_id.into(), v);
    }
    pub fn build(self) -> Vec<super::records::TargetAssignmentRecord> {
        self.members
            .into_iter()
            .map(|(member_id, assigned)| super::records::TargetAssignmentRecord {
                group_id: self.group_id.clone(),
                member_id,
                group_epoch: self.group_epoch,
                assigned,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_members_no_output() {
        let r = UniformAssignor.assign(&[], &[("t".into(), 4)]);
        assert!(r.is_empty());
    }

    #[test]
    fn even_split_2_members_4_partitions() {
        let m = vec!["a".to_string(), "b".to_string()];
        let r = UniformAssignor.assign(&m, &[("t".into(), 4)]);
        assert_eq!(r["a"][0].partitions, vec![0, 2]);
        assert_eq!(r["b"][0].partitions, vec![1, 3]);
    }

    #[test]
    fn uneven_split_3_members_7_partitions() {
        let m = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let r = UniformAssignor.assign(&m, &[("t".into(), 7)]);
        let mut sums: Vec<usize> = m.iter().map(|x| r[x][0].partitions.len()).collect();
        sums.sort();
        assert_eq!(sums, vec![2, 2, 3]);
    }
}
