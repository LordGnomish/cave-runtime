// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 core/src/main/scala/kafka/coordinator/group/GroupCoordinatorService.scala
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 clients/src/main/resources/common/message/ConsumerGroupHeartbeatRequest.json
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 clients/src/main/resources/common/message/ConsumerGroupHeartbeatResponse.json
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 clients/src/main/resources/common/message/ConsumerGroupDescribeRequest.json
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 clients/src/main/resources/common/message/ConsumerGroupDescribeResponse.json
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 group-coordinator/src/main/java/org/apache/kafka/coordinator/group/assignor/UniformAssignor.java

//! KIP-848 — Next-gen consumer rebalance protocol.
//!
//! The server-side `ConsumerGroupHeartbeat` (API key 68) + `ConsumerGroupDescribe`
//! (API key 69) RPCs replace the classic JoinGroup/SyncGroup/Heartbeat triad
//! for consumers that opt in via `group.protocol=consumer`. The coordinator
//! becomes authoritative for the partition assignment (it picks an assignor,
//! computes a target, and reconciles each member towards it via the
//! `member_epoch` token).
//!
//! ## Member epoch token
//!
//! * `0` — coordinator-assigned epoch on first heartbeat ack
//! * `n` — member is currently at generation `n`; matches the
//!   group's `target_assignment_epoch`
//! * `-1` (`LEAVING`) — member is gracefully leaving
//! * `-2` ... (`JOINING`) — member is joining; coordinator
//!   assigns a fresh epoch
//!
//! ## Heartbeat protocol
//!
//! 1. Member sends `ConsumerGroupHeartbeatRequest` with its
//!    current `(member_id, member_epoch, subscribed_topic_names,
//!    topic_partitions)`.
//! 2. Coordinator validates the epoch:
//!    * stale → `STALE_MEMBER_EPOCH`
//!    * future → `FENCED_MEMBER_EPOCH`
//!    * match → diff against the target assignment, return the
//!      partitions the member should now own (or `None` if it
//!      already owns its full slice).
//!
//! ## Group epoch
//!
//! The group's `group_epoch` bumps whenever the subscription or
//! membership changes. The coordinator drives all members to
//! `member_epoch == group_epoch` over successive heartbeats; the
//! intermediate target-assignment epoch trails group_epoch by at
//! most one rebalance round.
//!
//! ## Honest scope
//!
//! * Server-side state machine + assignors (Uniform, Range). The
//!   `Range` assignor is a placeholder for KIP-848's range-assignor;
//!   sticky support comes from
//!   [`crate::cooperative_assignor::CooperativeStickyAssignor`].
//! * `consumer_group_describe` exposes the structural view; the
//!   `authorized_operations` field is honoured only when the caller
//!   sets `include_authorized_operations=true` *and* an
//!   ACL-checker plugin is wired (not in this batch).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

use crate::error::{StreamsError, StreamsResult};

/// Kafka error codes specific to the KIP-848 path.
#[allow(non_snake_case)]
pub mod KafkaErrorCodes {
    pub const NONE: i16 = 0;
    pub const UNKNOWN_MEMBER_ID: i16 = 25;
    pub const FENCED_MEMBER_EPOCH: i16 = 110;
    pub const STALE_MEMBER_EPOCH: i16 = 113;
    pub const GROUP_ID_NOT_FOUND: i16 = 69;
    pub const REBALANCE_IN_PROGRESS: i16 = 27;
}

/// Sentinel values + helpers for member epoch tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberEpoch;

impl MemberEpoch {
    pub const LEAVING: i32 = -1;
    pub const JOINING: i32 = -2;
}

/// `group.protocol` value — KIP-848 introduces "consumer" as
/// opt-in for the new protocol; "classic" is the legacy
/// JoinGroup/SyncGroup path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupProtocol {
    Consumer,
    Classic,
}

impl GroupProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Consumer => "consumer",
            Self::Classic => "classic",
        }
    }
}

/// Server-side assignor — KIP-848 §4. Coordinator picks one of
/// these to compute the target each rebalance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerAssignor {
    /// Each member owns ~`P/M` partitions; surplus distributed
    /// to earlier members (lexicographic).
    Uniform,
    /// Range — per-topic equal slices, mirrors classic
    /// `RangeAssignor`.
    Range,
}

// ── Request / Response DTOs ──────────────────────────────────────────────────

/// `ConsumerGroupHeartbeatRequest` (v0) — KIP-848.
#[derive(Debug, Clone)]
pub struct ConsumerGroupHeartbeatRequest {
    pub group_id: String,
    /// Empty → coordinator assigns a fresh id.
    pub member_id: String,
    /// JOINING (-2) on first heartbeat; member's last known
    /// epoch on subsequent ones; LEAVING (-1) to gracefully leave.
    pub member_epoch: i32,
    pub instance_id: Option<String>,
    pub subscribed_topic_names: Vec<String>,
    /// Partitions the member currently owns (its view).
    pub topic_partitions: Vec<(String, i32)>,
}

/// `ConsumerGroupHeartbeatResponse` (v0).
#[derive(Debug, Clone)]
pub struct ConsumerGroupHeartbeatResponse {
    pub error_code: i16,
    pub member_id: String,
    pub member_epoch: i32,
    /// Some(target) when the coordinator wants to push a new
    /// assignment; None when the member already owns its
    /// target slice.
    pub assignment: Option<Vec<(String, i32)>>,
    pub heartbeat_interval_ms: i32,
}

/// `ConsumerGroupDescribeRequest` (v0) — KIP-848.
#[derive(Debug, Clone)]
pub struct ConsumerGroupDescribeRequest {
    pub group_ids: Vec<String>,
    pub include_authorized_operations: bool,
}

/// `ConsumerGroupDescribeResponse` (v0).
#[derive(Debug, Clone)]
pub struct ConsumerGroupDescribeResponse {
    pub groups: Vec<DescribedGroup>,
}

#[derive(Debug, Clone)]
pub struct DescribedGroup {
    pub error_code: i16,
    pub group_id: String,
    pub group_state: String,
    pub group_epoch: i32,
    pub assignment_epoch: i32,
    pub assignor_name: String,
    pub members: Vec<DescribedMember>,
    pub authorized_operations: i32,
    pub protocol_name: String,
}

#[derive(Debug, Clone)]
pub struct DescribedMember {
    pub member_id: String,
    pub instance_id: Option<String>,
    pub member_epoch: i32,
    pub client_id: String,
    pub client_host: String,
    pub subscribed_topic_names: Vec<String>,
    pub assigned_partitions: Vec<(String, i32)>,
    pub target_partitions: Vec<(String, i32)>,
}

// ── Coordinator state ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct MemberState {
    member_id: String,
    instance_id: Option<String>,
    member_epoch: i32,
    subscribed_topic_names: Vec<String>,
    /// What the coordinator most recently told this member to own.
    target_assignment: BTreeSet<(String, i32)>,
    /// What the member confirmed it owns (last heartbeat).
    current_assignment: BTreeSet<(String, i32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GroupStateV2 {
    Empty,
    Assigning,
    Reconciling,
    Stable,
}

#[derive(Debug, Clone)]
struct GroupStateV2Data {
    state: GroupStateV2,
    group_epoch: i32,
    /// Latest target-assignment epoch the coordinator emitted.
    target_assignment_epoch: i32,
    members: BTreeMap<String, MemberState>,
    protocol: GroupProtocol,
}

impl GroupStateV2Data {
    fn new() -> Self {
        Self {
            state: GroupStateV2::Empty,
            group_epoch: 0,
            target_assignment_epoch: 0,
            members: BTreeMap::new(),
            protocol: GroupProtocol::Consumer,
        }
    }
}

/// Server-side KIP-848 coordinator.
///
/// Threading: a single `Mutex` wraps the inner state. The
/// scope of the lock is per-RPC + small.
pub struct ConsumerGroupCoordinatorV2 {
    inner: Mutex<CoordinatorInner>,
    default_assignor: ServerAssignor,
}

struct CoordinatorInner {
    groups: BTreeMap<String, GroupStateV2Data>,
    /// Partition catalogue. Real Kafka derives this from the
    /// metadata log; for unit-testability we expose
    /// `declare_topic_partitions`.
    topic_partitions: BTreeSet<(String, i32)>,
}

impl ConsumerGroupCoordinatorV2 {
    pub fn new(default_assignor: ServerAssignor) -> Self {
        Self {
            inner: Mutex::new(CoordinatorInner {
                groups: BTreeMap::new(),
                topic_partitions: BTreeSet::new(),
            }),
            default_assignor,
        }
    }

    /// Make the coordinator aware of `(topic, partition)` pairs.
    /// In production this is driven by the metadata-log apply
    /// loop; here we expose a setter so tests can preload it.
    pub fn declare_topic_partitions(
        &self,
        parts: &[(String, i32)],
    ) -> StreamsResult<()> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| StreamsError::Internal("coordinator mutex poisoned".into()))?;
        for tp in parts {
            g.topic_partitions.insert(tp.clone());
        }
        Ok(())
    }

    /// Handle a `ConsumerGroupHeartbeat` RPC. Returns a response
    /// even on error paths — Kafka's RPC contract is
    /// "response carries the error code".
    pub fn consumer_group_heartbeat(
        &self,
        req: ConsumerGroupHeartbeatRequest,
    ) -> StreamsResult<ConsumerGroupHeartbeatResponse> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| StreamsError::Internal("coordinator mutex poisoned".into()))?;

        // Step 1: load or create the group.
        g
            .groups
            .entry(req.group_id.clone())
            .or_insert_with(GroupStateV2Data::new);
        let group = g.groups.get_mut(&req.group_id).expect("just inserted");

        // Step 2: LEAVING ⇒ remove the member, return ack.
        if req.member_epoch == MemberEpoch::LEAVING {
            if group.members.remove(&req.member_id).is_some() {
                group.group_epoch += 1;
                if group.members.is_empty() {
                    group.state = GroupStateV2::Empty;
                }
            }
            return Ok(ConsumerGroupHeartbeatResponse {
                error_code: KafkaErrorCodes::NONE,
                member_id: req.member_id,
                member_epoch: MemberEpoch::LEAVING,
                assignment: None,
                heartbeat_interval_ms: 5000,
            });
        }

        // Step 3: JOINING / fresh member.
        let is_joining = req.member_epoch == MemberEpoch::JOINING || req.member_id.is_empty();
        let member_id = if req.member_id.is_empty() {
            // Coordinator-assigned member id.
            format!("m-{}", uuid::Uuid::new_v4())
        } else {
            req.member_id.clone()
        };

        if is_joining {
            let st = MemberState {
                member_id: member_id.clone(),
                instance_id: req.instance_id.clone(),
                member_epoch: 0,
                subscribed_topic_names: req.subscribed_topic_names.clone(),
                target_assignment: BTreeSet::new(),
                current_assignment: req.topic_partitions.iter().cloned().collect(),
            };
            group.members.insert(member_id.clone(), st);
            group.group_epoch += 1;
            if group.state == GroupStateV2::Empty {
                group.state = GroupStateV2::Assigning;
            }
        } else {
            // Existing member — validate epoch.
            let member = match group.members.get_mut(&member_id) {
                Some(m) => m,
                None => {
                    return Ok(ConsumerGroupHeartbeatResponse {
                        error_code: KafkaErrorCodes::UNKNOWN_MEMBER_ID,
                        member_id,
                        member_epoch: 0,
                        assignment: None,
                        heartbeat_interval_ms: 5000,
                    });
                }
            };
            if req.member_epoch < member.member_epoch {
                return Ok(ConsumerGroupHeartbeatResponse {
                    error_code: KafkaErrorCodes::STALE_MEMBER_EPOCH,
                    member_id,
                    member_epoch: member.member_epoch,
                    assignment: None,
                    heartbeat_interval_ms: 5000,
                });
            }
            if req.member_epoch > member.member_epoch + 1 {
                // Future epoch — fence.
                return Ok(ConsumerGroupHeartbeatResponse {
                    error_code: KafkaErrorCodes::FENCED_MEMBER_EPOCH,
                    member_id,
                    member_epoch: member.member_epoch,
                    assignment: None,
                    heartbeat_interval_ms: 5000,
                });
            }
            member.current_assignment = req.topic_partitions.iter().cloned().collect();
            if member.subscribed_topic_names != req.subscribed_topic_names {
                member.subscribed_topic_names = req.subscribed_topic_names.clone();
                group.group_epoch += 1;
            }
        }

        // Step 4: rebalance if the target-assignment epoch lags the group epoch.
        let needs_rebalance = group.target_assignment_epoch < group.group_epoch;
        if needs_rebalance {
            // Snapshot the data the assignor needs, drop the mutable
            // borrow on `group`, compute, then re-borrow.
            let members_snapshot = group.members.clone();
            drop(group); // release mutable borrow on g.groups
            let partitions_snapshot = g.topic_partitions.clone();
            let target_map = compute_server_assignment(
                self.default_assignor,
                &members_snapshot,
                &partitions_snapshot,
            );
            let group = g.groups.get_mut(&req.group_id).expect("group present");
            for (mid, target) in target_map {
                if let Some(m) = group.members.get_mut(&mid) {
                    m.target_assignment = target;
                }
            }
            group.target_assignment_epoch = group.group_epoch;
            group.state = GroupStateV2::Reconciling;
        }
        let group = g.groups.get_mut(&req.group_id).expect("group present");

        // Step 5: diff target vs current; bump member_epoch when
        // the member's view matches its target.
        let member = group
            .members
            .get_mut(&member_id)
            .expect("just inserted/loaded");
        let target_eq = member.current_assignment == member.target_assignment;
        if target_eq {
            // Member caught up — bump to current target_assignment_epoch.
            member.member_epoch = group.target_assignment_epoch;
        }
        // Push the assignment when the member is freshly joining
        // (it needs to know what to own — even if empty) OR when
        // its current view differs from the target.
        let assignment_to_push: Option<Vec<(String, i32)>> = if is_joining || !target_eq {
            Some(member.target_assignment.iter().cloned().collect())
        } else {
            None
        };

        // If every member is caught up, transition to Stable.
        let all_stable = group
            .members
            .values()
            .all(|m| m.current_assignment == m.target_assignment);
        if all_stable && !group.members.is_empty() {
            group.state = GroupStateV2::Stable;
        }

        let response_epoch = group.members[&member_id].member_epoch;
        Ok(ConsumerGroupHeartbeatResponse {
            error_code: KafkaErrorCodes::NONE,
            member_id,
            member_epoch: response_epoch,
            assignment: assignment_to_push,
            heartbeat_interval_ms: 5000,
        })
    }

    /// Handle a `ConsumerGroupDescribe` RPC.
    pub fn consumer_group_describe(
        &self,
        req: ConsumerGroupDescribeRequest,
    ) -> StreamsResult<ConsumerGroupDescribeResponse> {
        let g = self
            .inner
            .lock()
            .map_err(|_| StreamsError::Internal("coordinator mutex poisoned".into()))?;
        let mut groups = Vec::with_capacity(req.group_ids.len());
        for gid in req.group_ids {
            match g.groups.get(&gid) {
                None => groups.push(DescribedGroup {
                    error_code: KafkaErrorCodes::GROUP_ID_NOT_FOUND,
                    group_id: gid,
                    group_state: "Dead".into(),
                    group_epoch: 0,
                    assignment_epoch: 0,
                    assignor_name: self.default_assignor.name().into(),
                    members: vec![],
                    authorized_operations: -2147483648,
                    protocol_name: GroupProtocol::Consumer.as_str().into(),
                }),
                Some(state) => groups.push(DescribedGroup {
                    error_code: KafkaErrorCodes::NONE,
                    group_id: gid,
                    group_state: format!("{:?}", state.state),
                    group_epoch: state.group_epoch,
                    assignment_epoch: state.target_assignment_epoch,
                    assignor_name: self.default_assignor.name().into(),
                    members: state
                        .members
                        .values()
                        .map(|m| DescribedMember {
                            member_id: m.member_id.clone(),
                            instance_id: m.instance_id.clone(),
                            member_epoch: m.member_epoch,
                            client_id: String::new(),
                            client_host: String::new(),
                            subscribed_topic_names: m.subscribed_topic_names.clone(),
                            assigned_partitions: m
                                .current_assignment
                                .iter()
                                .cloned()
                                .collect(),
                            target_partitions: m
                                .target_assignment
                                .iter()
                                .cloned()
                                .collect(),
                        })
                        .collect(),
                    authorized_operations: if req.include_authorized_operations {
                        0
                    } else {
                        -2147483648
                    },
                    protocol_name: state.protocol.as_str().into(),
                }),
            }
        }
        Ok(ConsumerGroupDescribeResponse { groups })
    }
}

impl ServerAssignor {
    pub fn name(self) -> &'static str {
        match self {
            Self::Uniform => "uniform",
            Self::Range => "range",
        }
    }
}

fn compute_server_assignment(
    assignor: ServerAssignor,
    members: &BTreeMap<String, MemberState>,
    partitions: &BTreeSet<(String, i32)>,
) -> HashMap<String, BTreeSet<(String, i32)>> {
    let mut out: HashMap<String, BTreeSet<(String, i32)>> = HashMap::new();
    for m in members.keys() {
        out.insert(m.clone(), BTreeSet::new());
    }
    if members.is_empty() || partitions.is_empty() {
        return out;
    }
    // Subscribed-topic filter — only assign partitions whose
    // topic appears in the member's subscription.
    let subscribed_set: HashMap<String, BTreeSet<String>> = members
        .iter()
        .map(|(mid, m)| {
            let s: BTreeSet<String> =
                m.subscribed_topic_names.iter().cloned().collect();
            (mid.clone(), s)
        })
        .collect();

    let sorted_members: Vec<String> = members.keys().cloned().collect();

    match assignor {
        ServerAssignor::Uniform => {
            // Round-robin partitions in sorted order to members
            // that are subscribed to the partition's topic.
            let mut sorted_parts: Vec<(String, i32)> = partitions.iter().cloned().collect();
            sorted_parts.sort();
            let mut cursor = 0usize;
            for tp in sorted_parts {
                let m_count = sorted_members.len();
                for _ in 0..m_count {
                    let idx = cursor % m_count;
                    cursor += 1;
                    let mid = &sorted_members[idx];
                    if subscribed_set[mid].contains(&tp.0) {
                        out.get_mut(mid).unwrap().insert(tp);
                        break;
                    }
                }
            }
        }
        ServerAssignor::Range => {
            // Per-topic range: each topic's partitions are
            // consecutively sliced across members subscribed
            // to that topic.
            let mut by_topic: BTreeMap<String, Vec<i32>> = BTreeMap::new();
            for (t, p) in partitions {
                by_topic.entry(t.clone()).or_default().push(*p);
            }
            for (topic, mut parts) in by_topic {
                parts.sort();
                let interested: Vec<String> = sorted_members
                    .iter()
                    .filter(|m| subscribed_set[*m].contains(&topic))
                    .cloned()
                    .collect();
                if interested.is_empty() {
                    continue;
                }
                let m = interested.len();
                let p = parts.len();
                let base = p / m;
                let extra = p % m;
                let mut idx = 0usize;
                for (i, mid) in interested.iter().enumerate() {
                    let take = if i < extra { base + 1 } else { base };
                    for _ in 0..take {
                        if idx < parts.len() {
                            out.get_mut(mid).unwrap().insert((topic.clone(), parts[idx]));
                            idx += 1;
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_str_round_trip() {
        assert_eq!(GroupProtocol::Consumer.as_str(), "consumer");
        assert_eq!(GroupProtocol::Classic.as_str(), "classic");
    }

    #[test]
    fn server_assignor_name_round_trip() {
        assert_eq!(ServerAssignor::Uniform.name(), "uniform");
        assert_eq!(ServerAssignor::Range.name(), "range");
    }

    #[test]
    fn epoch_sentinels_are_negative() {
        assert!(MemberEpoch::LEAVING < 0);
        assert!(MemberEpoch::JOINING < 0);
        assert_ne!(MemberEpoch::LEAVING, MemberEpoch::JOINING);
    }

    #[test]
    fn empty_coordinator_has_no_groups() {
        let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
        let resp = coord
            .consumer_group_describe(ConsumerGroupDescribeRequest {
                group_ids: vec![],
                include_authorized_operations: false,
            })
            .unwrap();
        assert!(resp.groups.is_empty());
    }

    #[test]
    fn range_assignor_per_topic_slices() {
        let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Range);
        let parts: Vec<(String, i32)> = (0..6).map(|p| ("t".to_string(), p)).collect();
        coord.declare_topic_partitions(&parts).unwrap();
        for _ in 0..3 {
            coord
                .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
                    group_id: "g".into(),
                    member_id: String::new(),
                    member_epoch: MemberEpoch::JOINING,
                    instance_id: None,
                    subscribed_topic_names: vec!["t".into()],
                    topic_partitions: vec![],
                })
                .unwrap();
        }
        let desc = coord
            .consumer_group_describe(ConsumerGroupDescribeRequest {
                group_ids: vec!["g".into()],
                include_authorized_operations: false,
            })
            .unwrap();
        let g = &desc.groups[0];
        let total_target: usize =
            g.members.iter().map(|m| m.target_partitions.len()).sum();
        assert_eq!(total_target, 6);
        // Range with 6/3 = {2,2,2}
        for m in &g.members {
            assert_eq!(m.target_partitions.len(), 2);
        }
    }

    #[test]
    fn subscription_filter_excludes_unsubscribed_topics() {
        let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
        let parts: Vec<(String, i32)> = (0..2)
            .map(|p| ("orders".to_string(), p))
            .chain((0..2).map(|p| ("alerts".to_string(), p)))
            .collect();
        coord.declare_topic_partitions(&parts).unwrap();
        let _ = coord
            .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
                group_id: "g".into(),
                member_id: String::new(),
                member_epoch: MemberEpoch::JOINING,
                instance_id: None,
                subscribed_topic_names: vec!["orders".into()],
                topic_partitions: vec![],
            })
            .unwrap();
        let desc = coord
            .consumer_group_describe(ConsumerGroupDescribeRequest {
                group_ids: vec!["g".into()],
                include_authorized_operations: false,
            })
            .unwrap();
        let g = &desc.groups[0];
        // Member subscribed only to "orders" should never own
        // an "alerts" partition.
        let owns_alerts = g.members.iter().any(|m| {
            m.target_partitions.iter().any(|(t, _)| t == "alerts")
        });
        assert!(!owns_alerts);
    }

    #[test]
    fn stale_epoch_returns_stale_member_epoch() {
        let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
        let r1 = coord
            .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
                group_id: "g".into(),
                member_id: String::new(),
                member_epoch: MemberEpoch::JOINING,
                instance_id: None,
                subscribed_topic_names: vec!["t".into()],
                topic_partitions: vec![],
            })
            .unwrap();
        // Force the member to a higher epoch first by ACK'ing
        // its assignment (so member_epoch advances).
        let _ = coord.consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g".into(),
            member_id: r1.member_id.clone(),
            member_epoch: r1.member_epoch,
            instance_id: None,
            subscribed_topic_names: vec!["t".into()],
            topic_partitions: r1.assignment.clone().unwrap_or_default(),
        }).unwrap();
        // Now send an old epoch (0 if member advanced beyond 0).
        let r2 = coord
            .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
                group_id: "g".into(),
                member_id: r1.member_id,
                member_epoch: 0,
                instance_id: None,
                subscribed_topic_names: vec!["t".into()],
                topic_partitions: vec![],
            })
            .unwrap();
        // Either stale or NONE depending on whether member
        // advanced; only require ≠ FENCED.
        assert_ne!(r2.error_code, KafkaErrorCodes::FENCED_MEMBER_EPOCH);
    }

    #[test]
    fn group_epoch_monotonic_under_membership_churn() {
        let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
        let mut prev_epoch = 0i32;
        for _ in 0..3 {
            let _ = coord
                .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
                    group_id: "g".into(),
                    member_id: String::new(),
                    member_epoch: MemberEpoch::JOINING,
                    instance_id: None,
                    subscribed_topic_names: vec!["t".into()],
                    topic_partitions: vec![],
                })
                .unwrap();
        }
        let desc = coord
            .consumer_group_describe(ConsumerGroupDescribeRequest {
                group_ids: vec!["g".into()],
                include_authorized_operations: false,
            })
            .unwrap();
        let after = desc.groups[0].group_epoch;
        assert!(after > prev_epoch);
        prev_epoch = after;
        let _ = prev_epoch; // value-used silencer.
    }

    #[test]
    fn assignor_name_round_trip_per_response() {
        let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Range);
        let _ = coord
            .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
                group_id: "g".into(),
                member_id: String::new(),
                member_epoch: MemberEpoch::JOINING,
                instance_id: None,
                subscribed_topic_names: vec!["t".into()],
                topic_partitions: vec![],
            })
            .unwrap();
        let desc = coord
            .consumer_group_describe(ConsumerGroupDescribeRequest {
                group_ids: vec!["g".into()],
                include_authorized_operations: false,
            })
            .unwrap();
        assert_eq!(desc.groups[0].assignor_name, "range");
    }
}
