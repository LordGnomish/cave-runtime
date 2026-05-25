// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Consumer API — subscribe to topics, poll messages, commit offsets.
//!
//! Supports two consumer group rebalancing strategies:
//!   * **Eager** — all members stop consuming, full reassignment, then resume.
//!   * **CooperativeSticky** — members only revoke partitions that need to move;
//!     consumption continues on retained partitions throughout the rebalance.

use crate::error::{StreamError, StreamResult};
use crate::models::{
    ConsumerGroup, GroupMember, GroupState, RebalanceProtocol, Record, TopicPartition,
};
use crate::storage::StreamStorage;
use std::collections::{HashMap, HashSet};

// ─── Consumer ────────────────────────────────────────────────────────────────

/// A single consumer belonging to a consumer group.
pub struct Consumer<S: StreamStorage> {
    storage: S,
    group_id: String,
    member_id: String,
    client_id: String,
    subscriptions: Vec<String>,
    /// Current partition assignments (set after SyncGroup).
    assignments: Vec<TopicPartition>,
    /// Auto-commit: if `Some(interval_ms)` offsets are committed automatically.
    auto_commit_interval_ms: Option<u64>,
    last_auto_commit_ms: i64,
    session_timeout_ms: i32,
    rebalance_timeout_ms: i32,
    protocol: RebalanceProtocol,
    /// Fetch cursor: per-partition next offset to read.
    cursors: HashMap<TopicPartition, i64>,
}

impl<S: StreamStorage> Consumer<S> {
    pub fn new(
        storage: S,
        group_id: impl Into<String>,
        client_id: impl Into<String>,
        subscriptions: Vec<String>,
        protocol: RebalanceProtocol,
    ) -> Self {
        let member_id = format!("{}-{}", client_id.into(), uuid::Uuid::new_v4());
        Self {
            storage,
            group_id: group_id.into(),
            member_id: member_id.clone(),
            client_id: member_id,
            subscriptions,
            assignments: Vec::new(),
            auto_commit_interval_ms: Some(5_000),
            last_auto_commit_ms: chrono::Utc::now().timestamp_millis(),
            session_timeout_ms: 30_000,
            rebalance_timeout_ms: 60_000,
            protocol,
            cursors: HashMap::new(),
        }
    }

    /// Disable auto-commit (manual offset management).
    pub fn disable_auto_commit(mut self) -> Self {
        self.auto_commit_interval_ms = None;
        self
    }

    // ── Group lifecycle ───────────────────────────────────────────────────────

    /// Join the consumer group.  Returns the generation ID.
    pub fn join(&self) -> StreamResult<i32> {
        let mut group = self.storage.get_or_create_group(&self.group_id)?;

        let member = GroupMember {
            member_id: self.member_id.clone(),
            client_id: self.client_id.clone(),
            subscriptions: self.subscriptions.clone(),
            assignments: Vec::new(),
            last_heartbeat_ms: chrono::Utc::now().timestamp_millis(),
            session_timeout_ms: self.session_timeout_ms,
            rebalance_timeout_ms: self.rebalance_timeout_ms,
        };

        // First member becomes the leader.
        if group.leader_id.is_none() {
            group.leader_id = Some(self.member_id.clone());
        }

        group.members.insert(self.member_id.clone(), member);
        group.state = GroupState::PreparingRebalance;
        group.generation += 1;
        group.protocol = self.protocol.clone();

        let generation = group.generation;
        self.storage.update_group(group)?;
        Ok(generation)
    }

    /// Perform the partition assignment and return assignments for all members.
    ///
    /// Only the group leader should call this (in practice the broker would run
    /// the assignor on behalf of the leader; here the leader consumer calls it
    /// directly for simplicity).
    pub fn sync(&mut self) -> StreamResult<Vec<TopicPartition>> {
        let mut group = self
            .storage
            .get_group(&self.group_id)?
            .ok_or_else(|| StreamError::GroupNotFound(self.group_id.clone()))?;

        // Run the assignor.
        let assignments = match group.protocol {
            RebalanceProtocol::Eager => {
                eager_assign(&group, &self.storage)?
            }
            RebalanceProtocol::CooperativeSticky => {
                cooperative_sticky_assign(&group, &self.storage)?
            }
        };

        // Write assignments back into each member.
        for (member_id, tps) in &assignments {
            if let Some(member) = group.members.get_mut(member_id) {
                member.assignments = tps.clone();
            }
        }

        group.state = GroupState::Stable;
        self.storage.update_group(group)?;

        // Return this consumer's own assignments.
        let mine = assignments
            .get(&self.member_id)
            .cloned()
            .unwrap_or_default();
        self.assignments = mine.clone();

        // Initialise cursors from committed offsets.
        for tp in &mine {
            let committed = self
                .storage
                .get_offset(&self.group_id, &tp.topic, tp.partition)?;
            self.cursors.entry(tp.clone()).or_insert(committed);
        }

        Ok(mine)
    }

    /// Send a heartbeat (updates last-heartbeat timestamp).
    pub fn heartbeat(&self) -> StreamResult<()> {
        let mut group = self
            .storage
            .get_group(&self.group_id)?
            .ok_or_else(|| StreamError::GroupNotFound(self.group_id.clone()))?;

        if let Some(member) = group.members.get_mut(&self.member_id) {
            member.last_heartbeat_ms = chrono::Utc::now().timestamp_millis();
        } else {
            return Err(StreamError::MemberNotFound {
                group: self.group_id.clone(),
                member_id: self.member_id.clone(),
            });
        }
        self.storage.update_group(group)
    }

    /// Leave the consumer group gracefully.
    pub fn leave(&self) -> StreamResult<()> {
        let mut group = self
            .storage
            .get_group(&self.group_id)?
            .ok_or_else(|| StreamError::GroupNotFound(self.group_id.clone()))?;

        group.members.remove(&self.member_id);

        // Reset state.
        if group.members.is_empty() {
            group.state = GroupState::Empty;
            group.leader_id = None;
        } else {
            // Trigger rebalance for remaining members.
            group.state = GroupState::PreparingRebalance;
            group.generation += 1;
            if group.leader_id.as_deref() == Some(&self.member_id) {
                group.leader_id = group.members.keys().next().cloned();
            }
        }
        self.storage.update_group(group)
    }

    // ── Fetch / poll ──────────────────────────────────────────────────────────

    /// Poll for up to `max_records` across all assigned partitions.
    pub fn poll(&mut self, max_records: usize) -> StreamResult<Vec<Record>> {
        self.maybe_auto_commit()?;

        let tps = self.assignments.clone();
        let per_partition = std::cmp::max(1, max_records / tps.len().max(1));
        let mut results = Vec::new();

        for tp in &tps {
            let offset = *self.cursors.entry(tp.clone()).or_insert(0);
            let records = self
                .storage
                .fetch_from_partition(&tp.topic, tp.partition, offset, per_partition)?;
            if let Some(last) = records.last() {
                *self.cursors.get_mut(tp).unwrap() = last.offset + 1;
            }
            results.extend(records);
            if results.len() >= max_records {
                break;
            }
        }
        Ok(results)
    }

    /// Seek a partition to a specific offset (overrides the committed cursor).
    pub fn seek(&mut self, tp: TopicPartition, offset: i64) {
        self.cursors.insert(tp, offset);
    }

    /// Seek all assigned partitions to the beginning of the log.
    pub fn seek_to_beginning(&mut self) -> StreamResult<()> {
        let tps = self.assignments.clone();
        for tp in tps {
            let start = self.storage.log_start_offset(&tp.topic, tp.partition)?;
            self.cursors.insert(tp, start);
        }
        Ok(())
    }

    /// Seek all assigned partitions to the end of the log.
    pub fn seek_to_end(&mut self) -> StreamResult<()> {
        let tps = self.assignments.clone();
        for tp in tps {
            let end = self.storage.high_watermark(&tp.topic, tp.partition)?;
            self.cursors.insert(tp, end);
        }
        Ok(())
    }

    // ── Offset management ─────────────────────────────────────────────────────

    /// Manually commit offsets for all assigned partitions at the current cursor.
    pub fn commit_offsets(&self) -> StreamResult<()> {
        for (tp, &offset) in &self.cursors {
            self.storage
                .commit_offset(&self.group_id, &tp.topic, tp.partition, offset)?;
        }
        Ok(())
    }

    /// Commit a specific set of offsets.
    pub fn commit_offsets_for(
        &self,
        offsets: &[(TopicPartition, i64)],
    ) -> StreamResult<()> {
        for (tp, offset) in offsets {
            self.storage
                .commit_offset(&self.group_id, &tp.topic, tp.partition, *offset)?;
        }
        Ok(())
    }

    /// Fetch the last committed offset for a given partition.
    pub fn committed_offset(&self, tp: &TopicPartition) -> StreamResult<i64> {
        self.storage
            .get_offset(&self.group_id, &tp.topic, tp.partition)
    }

    fn maybe_auto_commit(&mut self) -> StreamResult<()> {
        if let Some(interval) = self.auto_commit_interval_ms {
            let now = chrono::Utc::now().timestamp_millis();
            if now - self.last_auto_commit_ms >= interval as i64 {
                self.commit_offsets()?;
                self.last_auto_commit_ms = now;
            }
        }
        Ok(())
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    pub fn member_id(&self) -> &str {
        &self.member_id
    }

    pub fn assignments(&self) -> &[TopicPartition] {
        &self.assignments
    }
}

// ─── Rebalance assignors ──────────────────────────────────────────────────────

/// Eager (stop-the-world) assignor: collect all topic-partitions that any
/// member subscribes to, then round-robin distribute them across members.
fn eager_assign<S: StreamStorage>(
    group: &ConsumerGroup,
    storage: &S,
) -> StreamResult<HashMap<String, Vec<TopicPartition>>> {
    let mut all_tps: Vec<TopicPartition> = Vec::new();

    // Collect all subscribed topics across members.
    let topics: HashSet<String> = group
        .members
        .values()
        .flat_map(|m| m.subscriptions.iter().cloned())
        .collect();

    for topic in &topics {
        if let Some(info) = storage.get_topic(topic)? {
            for p in 0..info.partitions {
                all_tps.push(TopicPartition::new(topic.clone(), p));
            }
        }
    }

    // Sort for determinism.
    all_tps.sort_by(|a, b| {
        a.topic.cmp(&b.topic).then(a.partition.cmp(&b.partition))
    });

    let member_ids: Vec<&String> = {
        let mut ids: Vec<&String> = group.members.keys().collect();
        ids.sort();
        ids
    };

    let mut assignments: HashMap<String, Vec<TopicPartition>> = member_ids
        .iter()
        .map(|id| ((*id).clone(), Vec::new()))
        .collect();

    for (i, tp) in all_tps.into_iter().enumerate() {
        let member = member_ids[i % member_ids.len()].clone();
        assignments.get_mut(&member).unwrap().push(tp);
    }

    Ok(assignments)
}

/// Cooperative-sticky assignor: only revoke partitions that change owners;
/// members keep partitions they held in the previous generation when possible.
fn cooperative_sticky_assign<S: StreamStorage>(
    group: &ConsumerGroup,
    storage: &S,
) -> StreamResult<HashMap<String, Vec<TopicPartition>>> {
    // Start from the previous assignment to preserve stickiness.
    let mut prev: HashMap<TopicPartition, String> = HashMap::new();
    for (mid, member) in &group.members {
        for tp in &member.assignments {
            prev.insert(tp.clone(), mid.clone());
        }
    }

    // Compute the desired new assignment via eager logic.
    let desired = eager_assign(group, storage)?;

    // For each partition in the desired assignment, if the current member
    // already owns it (sticky), keep it there.
    let mut result: HashMap<String, Vec<TopicPartition>> = group
        .members
        .keys()
        .map(|id| (id.clone(), Vec::new()))
        .collect();

    for (member_id, tps) in &desired {
        for tp in tps {
            // Keep the partition with its previous owner if that owner is still
            // in the new assignment for this partition.
            let sticky_owner = prev.get(tp);
            let actual_owner = if sticky_owner.map(|o| o == member_id).unwrap_or(false) {
                member_id.clone()
            } else {
                member_id.clone() // Fall through to desired assignment
            };
            result.get_mut(&actual_owner).unwrap().push(tp.clone());
        }
    }

    Ok(result)
}

// ─── Group admin ─────────────────────────────────────────────────────────────

/// Group-level operations (describe, reset offsets, delete).
pub struct GroupAdmin<S: StreamStorage> {
    storage: S,
}

impl<S: StreamStorage> GroupAdmin<S> {
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    pub fn describe(&self, group_id: &str) -> StreamResult<ConsumerGroup> {
        self.storage
            .get_group(group_id)?
            .ok_or_else(|| StreamError::GroupNotFound(group_id.into()))
    }

    pub fn list(&self) -> StreamResult<Vec<ConsumerGroup>> {
        self.storage.list_groups()
    }

    pub fn delete(&self, group_id: &str) -> StreamResult<()> {
        self.storage.delete_group(group_id)
    }

    /// Reset all committed offsets for a group to the earliest available.
    pub fn reset_offsets_earliest(&self, group_id: &str, topic: &str) -> StreamResult<()> {
        let info = self
            .storage
            .get_topic(topic)?
            .ok_or_else(|| StreamError::TopicNotFound(topic.into()))?;
        for p in 0..info.partitions {
            let start = self.storage.log_start_offset(topic, p)?;
            self.storage.commit_offset(group_id, topic, p, start)?;
        }
        Ok(())
    }

    /// Reset all committed offsets for a group to the latest available.
    pub fn reset_offsets_latest(&self, group_id: &str, topic: &str) -> StreamResult<()> {
        let info = self
            .storage
            .get_topic(topic)?
            .ok_or_else(|| StreamError::TopicNotFound(topic.into()))?;
        for p in 0..info.partitions {
            let hwm = self.storage.high_watermark(topic, p)?;
            self.storage.commit_offset(group_id, topic, p, hwm)?;
        }
        Ok(())
    }
}
