// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// clients/src/main/java/org/apache/kafka/clients/consumer/CooperativeStickyAssignor.java
//
//! KIP-415 — `CooperativeStickyAssignor` (consumer-side).
//!
//! Computes a balanced and stable assignment that preserves owned
//! partitions whenever possible. The algorithm:
//!
//! 1. Collect candidate `(topic, partition)` pairs from the cluster
//!    snapshot (`topic_partitions`).
//! 2. Filter out partitions that no current member subscribes to.
//! 3. Compute the *capacity target* per member as
//!    `floor(N/M)` with the first `N mod M` (alphabetical) getting +1.
//! 4. Build the new plan: first re-assign each member's *owned*
//!    partitions that it still subscribes to (up to its capacity);
//!    second, pool the remaining unassigned partitions and hand them
//!    round-robin to members that still have headroom.
//! 5. Sort each member's list (topic-then-partition) for determinism.
//!
//! This mirrors upstream `CooperativeStickyAssignor.assign()` and
//! its parent `AbstractStickyAssignor.constrainedAssign()`. The
//! revoke-then-assign workflow is expressed by the *difference*
//! between each member's `owned_partitions` and the returned plan —
//! the caller's heartbeat loop drives the actual two-phase commit.

use std::collections::{BTreeMap, BTreeSet};

/// `(topic, partition)` pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TopicPartition {
    pub topic: String,
    pub partition: i32,
}

/// One member's view at rebalance time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemberSubscription {
    pub topics: Vec<String>,
    pub owned_partitions: Vec<TopicPartition>,
    /// Generation echoed from the coordinator; ignored by the
    /// algorithm but kept for parity with upstream serde.
    pub generation: i32,
}

#[derive(Default)]
pub struct CooperativeStickyAssignor;

impl CooperativeStickyAssignor {
    pub fn new() -> Self {
        Self
    }

    /// Compute the new assignment.
    ///
    /// `subs`              — member_id → subscription.
    /// `topic_partitions`  — topic → partition count snapshot.
    ///
    /// Returns: member_id → sorted `Vec<TopicPartition>` (every
    /// member that appears in `subs` shows up in the result; the
    /// list is empty if no subscribed partition is available).
    pub fn assign(
        &self,
        subs: &BTreeMap<String, MemberSubscription>,
        topic_partitions: &BTreeMap<String, i32>,
    ) -> BTreeMap<String, Vec<TopicPartition>> {
        let mut result: BTreeMap<String, Vec<TopicPartition>> = BTreeMap::new();
        if subs.is_empty() {
            return result;
        }

        // ── Step 1: collect candidate partitions ────────────────────────
        let mut all_partitions: Vec<TopicPartition> = Vec::new();
        for (topic, count) in topic_partitions {
            if *count <= 0 {
                continue;
            }
            for p in 0..*count {
                all_partitions.push(TopicPartition {
                    topic: topic.clone(),
                    partition: p,
                });
            }
        }
        all_partitions.sort();

        // ── Step 2: subscribers per topic ───────────────────────────────
        let mut topic_subscribers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (mid, s) in subs {
            for t in &s.topics {
                topic_subscribers
                    .entry(t.clone())
                    .or_default()
                    .insert(mid.clone());
            }
        }

        // Filter all_partitions to only those with subscribers.
        let assignable: Vec<TopicPartition> = all_partitions
            .iter()
            .filter(|tp| {
                topic_subscribers
                    .get(&tp.topic)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        // ── Step 3: per-member capacity ─────────────────────────────────
        // For each assignable partition, the member must be a subscriber.
        // We compute a uniform capacity over *eligible* assignments per
        // member. Since topics can be filtered, count each member's
        // upper bound as the number of assignable partitions in topics
        // it subscribes to.
        let member_ids: Vec<String> = subs.keys().cloned().collect();
        let mut member_topic_set: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (mid, s) in subs {
            member_topic_set.insert(mid.clone(), s.topics.iter().cloned().collect());
        }

        // Global capacity target: floor(N / M) and first `R = N mod M`
        // alphabetical members get +1.
        let total = assignable.len();
        let m = member_ids.len();
        let base = total / m;
        let rem = total % m;
        // `quota[member_id] = capacity`. Members sorted alphabetically.
        let mut quota: BTreeMap<String, usize> = BTreeMap::new();
        let mut sorted_members = member_ids.clone();
        sorted_members.sort();
        for (i, mid) in sorted_members.iter().enumerate() {
            let q = base + if i < rem { 1 } else { 0 };
            quota.insert(mid.clone(), q);
        }

        // Initialise empty buckets so every member is present in output.
        for mid in &member_ids {
            result.insert(mid.clone(), Vec::new());
        }

        // ── Step 4a: re-assign owned partitions, up to quota ────────────
        // Order members alphabetically for determinism.
        let mut taken: BTreeSet<TopicPartition> = BTreeSet::new();
        for mid in &sorted_members {
            let mut kept = 0usize;
            let s = &subs[mid];
            let topics = &member_topic_set[mid];
            for tp in &s.owned_partitions {
                // Owned partition must still exist (topic+partition
                // within current cluster range) and still be subscribed.
                let still_valid = topics.contains(&tp.topic)
                    && topic_partitions
                        .get(&tp.topic)
                        .map(|c| tp.partition < *c && tp.partition >= 0)
                        .unwrap_or(false);
                if !still_valid {
                    continue;
                }
                if taken.contains(tp) {
                    continue; // another member already claimed it (shouldn't
                              // happen in a sane input, but be defensive).
                }
                let q = *quota.get(mid).unwrap_or(&0);
                if kept < q {
                    result.get_mut(mid).unwrap().push(tp.clone());
                    taken.insert(tp.clone());
                    kept += 1;
                }
            }
        }

        // ── Step 4b: distribute remaining partitions ────────────────────
        let mut remaining: Vec<TopicPartition> = assignable
            .iter()
            .filter(|tp| !taken.contains(tp))
            .cloned()
            .collect();
        remaining.sort();
        // Hand them out to members that still have headroom, alphabetical.
        let mut idx = 0usize;
        for tp in remaining {
            // Find next member with headroom AND subscribed to this topic.
            let n = sorted_members.len();
            let mut placed = false;
            for offset in 0..n {
                let cand = &sorted_members[(idx + offset) % n];
                let q = *quota.get(cand).unwrap_or(&0);
                if result[cand].len() >= q {
                    continue;
                }
                if !member_topic_set[cand].contains(&tp.topic) {
                    continue;
                }
                result.get_mut(cand).unwrap().push(tp.clone());
                idx = (idx + offset + 1) % n;
                placed = true;
                break;
            }
            if !placed {
                // No member has both headroom + matching subscription —
                // partition goes unassigned. Upstream behaviour is the
                // same: orphan partitions are dropped from the plan.
            }
        }

        // ── Step 5: deterministic sort per member ───────────────────────
        for v in result.values_mut() {
            v.sort();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tp(t: &str, p: i32) -> TopicPartition {
        TopicPartition {
            topic: t.into(),
            partition: p,
        }
    }

    #[test]
    fn unit_one_member_owns_everything() {
        let mut subs = BTreeMap::new();
        subs.insert(
            "m".to_string(),
            MemberSubscription {
                topics: vec!["t".into()],
                owned_partitions: vec![],
                generation: 0,
            },
        );
        let mut topics = BTreeMap::new();
        topics.insert("t".to_string(), 4);
        let plan = CooperativeStickyAssignor::new().assign(&subs, &topics);
        assert_eq!(plan["m"].len(), 4);
    }

    #[test]
    fn unit_balanced_two_members_six_partitions() {
        let mut subs = BTreeMap::new();
        subs.insert(
            "a".to_string(),
            MemberSubscription {
                topics: vec!["t".into()],
                owned_partitions: vec![tp("t", 0), tp("t", 2), tp("t", 4)],
                generation: 0,
            },
        );
        subs.insert(
            "b".to_string(),
            MemberSubscription {
                topics: vec!["t".into()],
                owned_partitions: vec![tp("t", 1), tp("t", 3), tp("t", 5)],
                generation: 0,
            },
        );
        let mut topics = BTreeMap::new();
        topics.insert("t".to_string(), 6);
        let plan = CooperativeStickyAssignor::new().assign(&subs, &topics);
        // Sticky: each keeps its owned set.
        assert_eq!(plan["a"], vec![tp("t", 0), tp("t", 2), tp("t", 4)]);
        assert_eq!(plan["b"], vec![tp("t", 1), tp("t", 3), tp("t", 5)]);
    }
}
