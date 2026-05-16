// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// clients/src/main/java/org/apache/kafka/clients/consumer/CooperativeStickyAssignor.java
//
//! KIP-415 — `CooperativeStickyAssignor` (consumer-side) RED stub.
//!
//! The cooperative-sticky assignor produces a balanced and stable
//! assignment by preserving the current owners' partitions whenever
//! possible, and revoking only the partitions that need to move to
//! restore balance.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TopicPartition {
    pub topic: String,
    pub partition: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemberSubscription {
    pub topics: Vec<String>,
    pub owned_partitions: Vec<TopicPartition>,
    pub generation: i32,
}

#[derive(Default)]
pub struct CooperativeStickyAssignor;

impl CooperativeStickyAssignor {
    pub fn new() -> Self {
        Self
    }
    /// RED stub — returns empty map. Real impl supersedes.
    pub fn assign(
        &self,
        _subs: &BTreeMap<String, MemberSubscription>,
        _topic_partitions: &BTreeMap<String, i32>,
    ) -> BTreeMap<String, Vec<TopicPartition>> {
        BTreeMap::new()
    }
}
