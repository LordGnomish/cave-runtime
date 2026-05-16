// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// core/src/main/scala/kafka/server/group/GroupCoordinator.scala
// core/src/main/scala/kafka/coordinator/group/ConsumerGroupCoordinator.scala
//
//! Server-side state machine for KIP-848 consumer groups.

use std::collections::BTreeMap;

use crate::error::StreamsResult;

use super::assignor::{TargetAssignmentBuilder, UniformAssignor};
use super::records::{
    ConsumerGroupRecord, MemberRecord, MemberSubscription, PersistenceEntry, TopicPartitions,
};
use super::wire::{
    ConsumerGroupHeartbeatRequest, ConsumerGroupHeartbeatResponse, HeartbeatErrorCode,
};

/// Live state for one member.
#[derive(Debug, Clone)]
struct MemberState {
    member_id: String,
    instance_id: Option<String>,
    member_epoch: i32,
    subscription: MemberSubscription,
    rack_id: Option<String>,
    /// Coordinator-computed assignment at `group_epoch` last bump.
    target: Vec<TopicPartitions>,
}

#[derive(Debug, Clone, Default)]
struct GroupState {
    group_epoch: i32,
    members: BTreeMap<String, MemberState>,
    /// `instance_id → member_id` static-membership index.
    static_index: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerGroupSummary {
    pub group_id: String,
    pub group_epoch: i32,
    pub members: Vec<MemberSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberSummary {
    pub member_id: String,
    pub member_epoch: i32,
    pub subscribed: Vec<String>,
}

/// In-memory implementation of the KIP-848 server-side coordinator.
///
/// `__consumer_offsets` is modelled as the drain-able log returned from
/// [`Self::drain_persistence_log`] (broker integration writes those
/// entries to the real compacted topic).
pub struct ConsumerGroupCoordinator {
    groups: BTreeMap<String, GroupState>,
    /// Cluster metadata: topic → partition count.
    topic_metadata: BTreeMap<String, i32>,
    /// Monotonic counter for synthesising fresh `member_id`s.
    next_member_seq: u64,
    /// Pending records — broker drains this and writes them to the
    /// compacted persistence topic.
    pending: Vec<PersistenceEntry>,
    /// Heartbeat interval announced to clients (ms).
    heartbeat_interval_ms: i32,
}

impl Default for ConsumerGroupCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsumerGroupCoordinator {
    pub fn new() -> Self {
        Self {
            groups: BTreeMap::new(),
            topic_metadata: BTreeMap::new(),
            next_member_seq: 0,
            pending: Vec::new(),
            heartbeat_interval_ms: 5_000,
        }
    }

    /// Inject cluster metadata — the broker wires this from the
    /// controller's topic-partition view. Without it, freshly
    /// referenced topics resolve to `0` partitions and members get
    /// empty assignments (matches upstream behaviour).
    pub fn set_topic_partition_count(&mut self, topic: impl Into<String>, count: i32) {
        self.topic_metadata.insert(topic.into(), count);
    }

    pub fn describe_group(&self, group_id: &str) -> Option<ConsumerGroupSummary> {
        let g = self.groups.get(group_id)?;
        let mut members: Vec<MemberSummary> = g
            .members
            .values()
            .map(|m| MemberSummary {
                member_id: m.member_id.clone(),
                member_epoch: m.member_epoch,
                subscribed: m.subscription.topic_names.clone(),
            })
            .collect();
        members.sort_by(|a, b| a.member_id.cmp(&b.member_id));
        Some(ConsumerGroupSummary {
            group_id: group_id.to_string(),
            group_epoch: g.group_epoch,
            members,
        })
    }

    pub fn list_groups(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }

    pub fn drain_persistence_log(&mut self) -> Vec<PersistenceEntry> {
        std::mem::take(&mut self.pending)
    }

    /// Single heartbeat RPC. Validates inputs, mutates group state,
    /// recomputes assignment if necessary, and returns the response.
    pub fn heartbeat(
        &mut self,
        req: ConsumerGroupHeartbeatRequest,
    ) -> StreamsResult<ConsumerGroupHeartbeatResponse> {
        // ── input validation ────────────────────────────────────────────
        if req.protocol_version < 1 {
            return Ok(err_resp(
                &req.member_id,
                req.member_epoch,
                HeartbeatErrorCode::UnsupportedVersion,
            ));
        }
        if req.group_id.is_empty() {
            return Ok(err_resp(
                &req.member_id,
                req.member_epoch,
                HeartbeatErrorCode::InvalidGroupId,
            ));
        }
        if req.rebalance_timeout_ms < 0 {
            return Ok(err_resp(
                &req.member_id,
                req.member_epoch,
                HeartbeatErrorCode::InvalidRequest,
            ));
        }

        // ── leave path ──────────────────────────────────────────────────
        if req.member_epoch < 0 {
            return Ok(self.handle_leave(&req));
        }

        // ── join / static-rejoin / continued-heartbeat ──────────────────
        let member_id = self.resolve_member_id(&req);

        // Unknown member with non-empty id and non-zero epoch is fenced.
        let group_exists = self.groups.contains_key(&req.group_id);
        let known_member = group_exists
            && self
                .groups
                .get(&req.group_id)
                .map(|g| g.members.contains_key(&member_id))
                .unwrap_or(false);
        if !known_member && !req.member_id.is_empty() && req.instance_id.is_none() {
            return Ok(err_resp(
                &req.member_id,
                req.member_epoch,
                HeartbeatErrorCode::UnknownMemberId,
            ));
        }

        // ── apply join / refresh subscription ───────────────────────────
        let subscription_changed = self.apply_member(&req, &member_id);

        // ── epoch fencing ───────────────────────────────────────────────
        // After applying, the per-member state holds the correct epoch;
        // any request whose epoch is *greater* than what we know is fenced.
        let member_epoch_now = self
            .groups
            .get(&req.group_id)
            .and_then(|g| g.members.get(&member_id))
            .map(|m| m.member_epoch)
            .unwrap_or(0);
        // Allow the first join (epoch 0 in → epoch 1 out) and the
        // confirm/ack on the next call (epoch_in == epoch_now). Anything
        // higher than `member_epoch_now` is fenced.
        if req.member_epoch > member_epoch_now {
            return Ok(err_resp(
                &member_id,
                member_epoch_now,
                HeartbeatErrorCode::FencedMemberEpoch,
            ));
        }

        // ── recompute target assignment if subscription changed ─────────
        if subscription_changed {
            self.recompute_target(&req.group_id);
        }

        // ── build response ──────────────────────────────────────────────
        let g = self.groups.get(&req.group_id).expect("group present");
        let m = g.members.get(&member_id).expect("member present");
        Ok(ConsumerGroupHeartbeatResponse {
            error_code: HeartbeatErrorCode::None as i16,
            member_id: m.member_id.clone(),
            member_epoch: m.member_epoch,
            heartbeat_interval_ms: self.heartbeat_interval_ms,
            assignment: m.target.clone(),
        })
    }

    // ── internals ───────────────────────────────────────────────────────

    fn resolve_member_id(&mut self, req: &ConsumerGroupHeartbeatRequest) -> String {
        // KIP-345 static membership: reuse a prior member_id for the same instance.
        if let Some(inst) = &req.instance_id {
            if let Some(g) = self.groups.get(&req.group_id) {
                if let Some(existing) = g.static_index.get(inst) {
                    return existing.clone();
                }
            }
        }
        if !req.member_id.is_empty() {
            return req.member_id.clone();
        }
        // Mint fresh id.
        self.next_member_seq += 1;
        format!("member-{:016x}", self.next_member_seq)
    }

    /// Returns `true` if anything that influences the target assignment
    /// changed (subscription set or membership).
    fn apply_member(
        &mut self,
        req: &ConsumerGroupHeartbeatRequest,
        member_id: &str,
    ) -> bool {
        let g = self.groups.entry(req.group_id.clone()).or_default();
        let mut changed = false;

        // Static-index registration (new instance).
        if let Some(inst) = &req.instance_id {
            if !g.static_index.contains_key(inst) {
                g.static_index.insert(inst.clone(), member_id.to_string());
                changed = true;
            }
        }

        match g.members.get_mut(member_id) {
            None => {
                // Fresh join.
                g.group_epoch += 1;
                let m = MemberState {
                    member_id: member_id.to_string(),
                    instance_id: req.instance_id.clone(),
                    member_epoch: g.group_epoch,
                    subscription: MemberSubscription {
                        topic_names: req.subscribed_topic_names.clone(),
                        topic_regex: req.subscribed_topic_regex.clone(),
                    },
                    rack_id: req.rack_id.clone(),
                    target: vec![],
                };
                g.members.insert(member_id.to_string(), m);
                changed = true;
            }
            Some(m) => {
                let new_sub = MemberSubscription {
                    topic_names: req.subscribed_topic_names.clone(),
                    topic_regex: req.subscribed_topic_regex.clone(),
                };
                if m.subscription != new_sub {
                    m.subscription = new_sub;
                    // Subscription change → bump group epoch + member epoch.
                    g.group_epoch += 1;
                    m.member_epoch = g.group_epoch;
                    changed = true;
                }
            }
        }

        if changed {
            let snap_record = ConsumerGroupRecord {
                group_id: req.group_id.clone(),
                group_epoch: g.group_epoch,
                topic_partition_metadata: self
                    .topic_metadata
                    .iter()
                    .map(|(t, c)| (t.clone(), *c))
                    .collect(),
            };
            self.pending
                .push(PersistenceEntry::ConsumerGroup(snap_record));

            if let Some(m) = self.groups.get(&req.group_id).unwrap().members.get(member_id) {
                let mr = MemberRecord {
                    group_id: req.group_id.clone(),
                    member_id: member_id.to_string(),
                    instance_id: m.instance_id.clone(),
                    member_epoch: m.member_epoch,
                    subscription: m.subscription.clone(),
                    rack_id: m.rack_id.clone(),
                };
                self.pending.push(PersistenceEntry::Member(mr));
            }
        }

        changed
    }

    fn recompute_target(&mut self, group_id: &str) {
        // Collect snapshot — borrow checker.
        let (members, sub_topics, group_epoch) = {
            let g = self.groups.get(group_id).expect("group");
            let members: Vec<String> = g.members.keys().cloned().collect();
            let mut topics: std::collections::BTreeSet<String> = Default::default();
            for m in g.members.values() {
                for t in &m.subscription.topic_names {
                    topics.insert(t.clone());
                }
            }
            (members, topics, g.group_epoch)
        };

        let topic_counts: Vec<(String, i32)> = sub_topics
            .into_iter()
            .map(|t| {
                let c = *self.topic_metadata.get(&t).unwrap_or(&0);
                (t, c)
            })
            .collect();

        let plan = UniformAssignor.assign(&members, &topic_counts);

        // Write target into per-member state + emit records.
        let mut builder = TargetAssignmentBuilder::new(group_id, group_epoch);
        let g = self.groups.get_mut(group_id).expect("group");
        for member_id in members {
            let assigned = plan.get(&member_id).cloned().unwrap_or_default();
            if let Some(m) = g.members.get_mut(&member_id) {
                m.target = assigned.clone();
            }
            let pairs: Vec<(String, Vec<i32>)> = assigned
                .into_iter()
                .map(|tp| (tp.topic, tp.partitions))
                .collect();
            builder.add(member_id, pairs);
        }
        for rec in builder.build() {
            self.pending.push(PersistenceEntry::TargetAssignment(rec));
        }
    }

    fn handle_leave(
        &mut self,
        req: &ConsumerGroupHeartbeatRequest,
    ) -> ConsumerGroupHeartbeatResponse {
        let group_id = req.group_id.clone();
        let mut effective_member_id = req.member_id.clone();
        let mut leave_instance: Option<String> = None;
        if let Some(g) = self.groups.get(&group_id) {
            if effective_member_id.is_empty() {
                if let Some(inst) = &req.instance_id {
                    if let Some(mid) = g.static_index.get(inst) {
                        effective_member_id = mid.clone();
                    }
                }
            }
        }
        if let Some(g) = self.groups.get_mut(&group_id) {
            if let Some(m) = g.members.remove(&effective_member_id) {
                leave_instance = m.instance_id.clone();
                g.group_epoch += 1;
                // Emit tombstones for member + target_assignment.
                self.pending.push(PersistenceEntry::Tombstone {
                    kind: "member",
                    group_id: group_id.clone(),
                    member_id: Some(effective_member_id.clone()),
                });
                self.pending.push(PersistenceEntry::Tombstone {
                    kind: "target_assignment",
                    group_id: group_id.clone(),
                    member_id: Some(effective_member_id.clone()),
                });
            }
            if let Some(inst) = &leave_instance {
                g.static_index.remove(inst);
            }
        }
        // Recompute target after removal.
        self.recompute_target(&group_id);
        ConsumerGroupHeartbeatResponse {
            error_code: HeartbeatErrorCode::None as i16,
            member_id: effective_member_id,
            member_epoch: -1,
            heartbeat_interval_ms: self.heartbeat_interval_ms,
            assignment: vec![],
        }
    }
}

fn err_resp(
    member_id: &str,
    member_epoch: i32,
    code: HeartbeatErrorCode,
) -> ConsumerGroupHeartbeatResponse {
    ConsumerGroupHeartbeatResponse {
        error_code: code as i16,
        member_id: member_id.to_string(),
        member_epoch,
        heartbeat_interval_ms: 5_000,
        assignment: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(g: &str, m: &str, e: i32, sub: &[&str]) -> ConsumerGroupHeartbeatRequest {
        ConsumerGroupHeartbeatRequest {
            group_id: g.into(),
            member_id: m.into(),
            member_epoch: e,
            instance_id: None,
            rack_id: None,
            rebalance_timeout_ms: 30_000,
            subscribed_topic_names: sub.iter().map(|s| (*s).into()).collect(),
            subscribed_topic_regex: None,
            server_assignor: None,
            topic_partitions: vec![],
            protocol_version: 1,
        }
    }

    #[test]
    fn first_heartbeat_assigns_member_id() {
        let mut c = ConsumerGroupCoordinator::new();
        let resp = c.heartbeat(r("g", "", 0, &["t"])).unwrap();
        assert!(!resp.member_id.is_empty());
        assert_eq!(resp.member_epoch, 1);
    }

    #[test]
    fn confirm_returns_same_epoch() {
        let mut c = ConsumerGroupCoordinator::new();
        let r0 = c.heartbeat(r("g", "", 0, &["t"])).unwrap();
        let r1 = c
            .heartbeat(r("g", &r0.member_id, r0.member_epoch, &["t"]))
            .unwrap();
        assert_eq!(r0.member_epoch, r1.member_epoch);
    }
}
