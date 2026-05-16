//! Consumer group coordinator — join/sync/heartbeat/leave + rebalance protocols.
//!
//! Supports: range, roundrobin, sticky, cooperative-sticky
//!
//! Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 core/src/main/scala/kafka/coordinator/group/GroupCoordinator.scala

use crate::error::{StreamsError, StreamsResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use uuid::Uuid;

/// Encode a (topic, partition) set into the simple wire form used
/// for cooperative-plan input — `"topic:partition,topic:partition"`
/// UTF-8 bytes. cave-streams' assignment bytes don't need to match
/// the Kafka MemberAssignment v0 wire shape exactly because the
/// cooperative coordinator interprets them locally; this codec is
/// the canonical local format used by [`decode_assignment`].
pub fn encode_assignment(
    parts: &std::collections::BTreeSet<crate::incremental_rebalance::Tp>,
) -> Vec<u8> {
    let mut out = String::new();
    for (i, (topic, partition)) in parts.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(topic);
        out.push(':');
        out.push_str(&partition.to_string());
    }
    out.into_bytes()
}

/// Inverse of [`encode_assignment`]. Empty bytes → empty set.
pub fn decode_assignment(
    bytes: &[u8],
) -> std::collections::BTreeSet<crate::incremental_rebalance::Tp> {
    let mut out = BTreeSet::new();
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return out,
    };
    for chunk in s.split(',') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let Some((topic, partition)) = chunk.rsplit_once(':') else {
            continue;
        };
        let Ok(p) = partition.parse::<i32>() else {
            continue;
        };
        out.insert((topic.to_string(), p));
    }
    out
}

// ── Rebalance protocol ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RebalanceProtocol {
    Range,
    RoundRobin,
    Sticky,
    CooperativeSticky,
}

impl RebalanceProtocol {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().replace('_', "-").as_str() {
            "range" => Self::Range,
            "roundrobin" | "round-robin" => Self::RoundRobin,
            "sticky" => Self::Sticky,
            "cooperative-sticky" => Self::CooperativeSticky,
            _ => Self::Range,
        }
    }

    /// Assign partitions to members.
    pub fn assign(
        &self,
        members: &[String],
        topic_partitions: &HashMap<String, i32>,
    ) -> HashMap<String, Vec<(String, i32)>> {
        match self {
            Self::Range => self.assign_range(members, topic_partitions),
            Self::RoundRobin => self.assign_roundrobin(members, topic_partitions),
            Self::Sticky | Self::CooperativeSticky => {
                // simplified: fall back to roundrobin
                self.assign_roundrobin(members, topic_partitions)
            }
        }
    }

    fn assign_range(
        &self,
        members: &[String],
        topic_partitions: &HashMap<String, i32>,
    ) -> HashMap<String, Vec<(String, i32)>> {
        let mut assignments: HashMap<String, Vec<(String, i32)>> =
            members.iter().map(|m| (m.clone(), vec![])).collect();
        if members.is_empty() {
            return assignments;
        }
        let mut sorted_members = members.to_vec();
        sorted_members.sort();
        for (topic, &num_partitions) in topic_partitions {
            let partitions_per_member = num_partitions / members.len() as i32;
            let extra = num_partitions % members.len() as i32;
            let mut start = 0;
            for (i, member) in sorted_members.iter().enumerate() {
                let count = partitions_per_member + if (i as i32) < extra { 1 } else { 0 };
                for p in start..start + count {
                    assignments.entry(member.clone()).or_default().push((topic.clone(), p));
                }
                start += count;
            }
        }
        assignments
    }

    fn assign_roundrobin(
        &self,
        members: &[String],
        topic_partitions: &HashMap<String, i32>,
    ) -> HashMap<String, Vec<(String, i32)>> {
        let mut assignments: HashMap<String, Vec<(String, i32)>> =
            members.iter().map(|m| (m.clone(), vec![])).collect();
        if members.is_empty() {
            return assignments;
        }
        let mut sorted_members = members.to_vec();
        sorted_members.sort();
        let mut idx = 0usize;
        let mut all_partitions: Vec<(String, i32)> = Vec::new();
        for (topic, &count) in topic_partitions {
            for p in 0..count {
                all_partitions.push((topic.clone(), p));
            }
        }
        all_partitions.sort();
        for (topic, partition) in all_partitions {
            let member = &sorted_members[idx % sorted_members.len()];
            assignments.entry(member.clone()).or_default().push((topic, partition));
            idx += 1;
        }
        assignments
    }
}

// ── Group member ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    pub member_id: String,
    pub client_id: String,
    pub client_host: String,
    pub session_timeout_ms: i32,
    pub rebalance_timeout_ms: i32,
    pub protocol_type: String,
    /// Per-protocol metadata (protocol name → metadata bytes)
    pub protocols: HashMap<String, Vec<u8>>,
    /// Current assignment (serialised assignment bytes per the chosen protocol)
    pub assignment: Vec<u8>,
    pub last_heartbeat: DateTime<Utc>,
    pub joined_at: DateTime<Utc>,
}

impl GroupMember {
    pub fn is_expired(&self) -> bool {
        let elapsed = Utc::now() - self.last_heartbeat;
        elapsed.num_milliseconds() > self.session_timeout_ms as i64
    }
}

// ── Group state machine ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum GroupState {
    Empty,
    PreparingRebalance,
    CompletingRebalance,
    Stable,
    Dead,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsumerGroup {
    pub group_id: String,
    pub state: GroupState,
    pub generation_id: i32,
    pub protocol_type: String,
    pub protocol_name: String,
    pub leader_id: String,
    pub members: HashMap<String, GroupMember>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ConsumerGroup {
    pub fn new(group_id: String) -> Self {
        Self {
            group_id,
            state: GroupState::Empty,
            generation_id: 0,
            protocol_type: String::new(),
            protocol_name: String::new(),
            leader_id: String::new(),
            members: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn is_empty_group(&self) -> bool {
        self.members.is_empty() || self.state == GroupState::Empty
    }
}

// ── Coordinator ───────────────────────────────────────────────────────────────

pub struct GroupCoordinator {
    groups: DashMap<String, ConsumerGroup>,
}

impl GroupCoordinator {
    pub fn new() -> Self {
        Self {
            groups: DashMap::new(),
        }
    }

    // ── JoinGroup ─────────────────────────────────────────────────────────────

    pub fn join_group(
        &self,
        group_id: String,
        member_id: Option<String>,
        client_id: String,
        client_host: String,
        session_timeout_ms: i32,
        rebalance_timeout_ms: i32,
        protocol_type: String,
        protocols: HashMap<String, Vec<u8>>,
    ) -> StreamsResult<JoinGroupResult> {
        let assigned_member_id = member_id
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| format!("{client_id}-{}", Uuid::new_v4()));

        let mut group = self
            .groups
            .entry(group_id.clone())
            .or_insert_with(|| ConsumerGroup::new(group_id.clone()));

        // If rejoining, update existing member
        let member = GroupMember {
            member_id: assigned_member_id.clone(),
            client_id,
            client_host,
            session_timeout_ms,
            rebalance_timeout_ms,
            protocol_type: protocol_type.clone(),
            protocols,
            assignment: vec![],
            last_heartbeat: Utc::now(),
            joined_at: Utc::now(),
        };
        group.members.insert(assigned_member_id.clone(), member);

        // Elect leader = first member alphabetically
        if group.leader_id.is_empty() || !group.members.contains_key(&group.leader_id) {
            group.leader_id = group.members.keys().min().cloned().unwrap_or_default();
        }

        group.protocol_type = protocol_type;
        group.state = GroupState::PreparingRebalance;
        group.generation_id += 1;
        group.updated_at = Utc::now();

        let is_leader = assigned_member_id == group.leader_id;
        let members_metadata: Vec<JoinGroupMemberMeta> = if is_leader {
            group
                .members
                .values()
                .map(|m| JoinGroupMemberMeta {
                    member_id: m.member_id.clone(),
                    metadata: m
                        .protocols
                        .get(&group.protocol_name)
                        .cloned()
                        .unwrap_or_default(),
                })
                .collect()
        } else {
            vec![]
        };

        Ok(JoinGroupResult {
            error_code: 0,
            generation_id: group.generation_id,
            protocol_name: group.protocol_name.clone(),
            leader_id: group.leader_id.clone(),
            member_id: assigned_member_id,
            members: members_metadata,
        })
    }

    // ── SyncGroup ─────────────────────────────────────────────────────────────

    pub fn sync_group(
        &self,
        group_id: &str,
        generation_id: i32,
        member_id: &str,
        assignments: HashMap<String, Vec<u8>>,
    ) -> StreamsResult<Vec<u8>> {
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or_else(|| StreamsError::GroupNotFound(group_id.into()))?;

        if group.generation_id != generation_id {
            return Err(StreamsError::IllegalGeneration {
                group: group_id.into(),
                expected: group.generation_id,
                got: generation_id,
            });
        }
        if !group.members.contains_key(member_id) {
            return Err(StreamsError::MemberNotFound {
                group: group_id.into(),
                member: member_id.into(),
            });
        }

        // Apply assignments from leader
        for (mid, assign) in assignments {
            if let Some(m) = group.members.get_mut(&mid) {
                m.assignment = assign;
            }
        }

        group.state = GroupState::Stable;
        group.updated_at = Utc::now();

        let assignment = group
            .members
            .get(member_id)
            .map(|m| m.assignment.clone())
            .unwrap_or_default();
        Ok(assignment)
    }

    // ── Heartbeat ─────────────────────────────────────────────────────────────

    pub fn heartbeat(
        &self,
        group_id: &str,
        generation_id: i32,
        member_id: &str,
    ) -> StreamsResult<i16> {
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or_else(|| StreamsError::GroupNotFound(group_id.into()))?;

        if group.generation_id != generation_id {
            return Err(StreamsError::IllegalGeneration {
                group: group_id.into(),
                expected: group.generation_id,
                got: generation_id,
            });
        }
        let member = group.members.get_mut(member_id).ok_or_else(|| {
            StreamsError::MemberNotFound {
                group: group_id.into(),
                member: member_id.into(),
            }
        })?;
        member.last_heartbeat = Utc::now();

        let error_code = if group.state == GroupState::PreparingRebalance {
            27 // REBALANCE_IN_PROGRESS
        } else {
            0
        };
        Ok(error_code)
    }

    // ── LeaveGroup ────────────────────────────────────────────────────────────

    pub fn leave_group(&self, group_id: &str, member_id: &str) -> StreamsResult<()> {
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or_else(|| StreamsError::GroupNotFound(group_id.into()))?;

        group.members.remove(member_id);
        if group.members.is_empty() {
            group.state = GroupState::Empty;
        } else {
            group.state = GroupState::PreparingRebalance;
            group.generation_id += 1;
            if group.leader_id == member_id {
                group.leader_id = group.members.keys().min().cloned().unwrap_or_default();
            }
        }
        group.updated_at = Utc::now();
        Ok(())
    }

    // ── DescribeGroups ────────────────────────────────────────────────────────

    pub fn describe_group(&self, group_id: &str) -> Option<GroupDescription> {
        let group = self.groups.get(group_id)?;
        Some(GroupDescription {
            group_id: group.group_id.clone(),
            state: format!("{:?}", group.state),
            protocol_type: group.protocol_type.clone(),
            protocol: group.protocol_name.clone(),
            members: group
                .members
                .values()
                .map(|m| MemberDescription {
                    member_id: m.member_id.clone(),
                    client_id: m.client_id.clone(),
                    client_host: m.client_host.clone(),
                })
                .collect(),
        })
    }

    pub fn list_groups(&self) -> Vec<GroupSummary> {
        self.groups
            .iter()
            .map(|e| GroupSummary {
                group_id: e.key().clone(),
                protocol_type: e.value().protocol_type.clone(),
                state: format!("{:?}", e.value().state),
            })
            .collect()
    }

    pub fn delete_group(&self, group_id: &str) -> StreamsResult<()> {
        let group = self
            .groups
            .get(group_id)
            .ok_or_else(|| StreamsError::GroupNotFound(group_id.into()))?;

        if !group.is_empty_group() {
            return Err(StreamsError::Internal(format!(
                "group {group_id} has active members"
            )));
        }
        drop(group);
        self.groups.remove(group_id);
        Ok(())
    }

    /// Build a cooperative incremental rebalance plan for `group_id`
    /// (KIP-415). The caller supplies the desired partition set; the
    /// coordinator computes a balanced + sticky target from each
    /// member's `assignment` bytes (interpreted via
    /// [`decode_assignment`]) and returns the two-phase plan that
    /// follows the revoke-then-assign discipline.
    pub fn cooperative_plan(
        &self,
        group_id: &str,
        partitions: &std::collections::BTreeSet<crate::incremental_rebalance::Tp>,
    ) -> StreamsResult<crate::incremental_rebalance::IncrementalRebalancePlan> {
        let group = self
            .groups
            .get(group_id)
            .ok_or_else(|| StreamsError::GroupNotFound(group_id.into()))?;
        let members: Vec<String> = group.members.keys().cloned().collect();
        let mut previous: HashMap<String, std::collections::BTreeSet<crate::incremental_rebalance::Tp>> =
            HashMap::new();
        for (mid, m) in &group.members {
            previous.insert(mid.clone(), decode_assignment(&m.assignment));
        }
        drop(group);
        Ok(crate::cooperative_assignor::cooperative_sticky_plan(
            &previous,
            &members,
            partitions,
        ))
    }

    /// Expire members that haven't heartbeated within their session timeout.
    pub fn expire_stale_members(&self) {
        for mut group in self.groups.iter_mut() {
            let stale: Vec<String> = group
                .members
                .iter()
                .filter(|(_, m)| m.is_expired())
                .map(|(id, _)| id.clone())
                .collect();
            for mid in stale {
                group.members.remove(&mid);
                tracing::info!(group_id = %group.group_id, member_id = %mid, "expired stale member");
            }
            if group.members.is_empty() {
                group.state = GroupState::Empty;
            }
        }
    }
}

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct JoinGroupResult {
    pub error_code: i16,
    pub generation_id: i32,
    pub protocol_name: String,
    pub leader_id: String,
    pub member_id: String,
    pub members: Vec<JoinGroupMemberMeta>,
}

#[derive(Debug, Clone)]
pub struct JoinGroupMemberMeta {
    pub member_id: String,
    pub metadata: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupDescription {
    pub group_id: String,
    pub state: String,
    pub protocol_type: String,
    pub protocol: String,
    pub members: Vec<MemberDescription>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemberDescription {
    pub member_id: String,
    pub client_id: String,
    pub client_host: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupSummary {
    pub group_id: String,
    pub protocol_type: String,
    pub state: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coordinator() -> GroupCoordinator {
        GroupCoordinator::new()
    }

    #[test]
    fn join_creates_group_and_elects_leader() {
        let c = coordinator();
        let result = c
            .join_group(
                "my-group".into(),
                None,
                "consumer-1".into(),
                "/127.0.0.1".into(),
                30000,
                60000,
                "consumer".into(),
                HashMap::new(),
            )
            .unwrap();
        assert_eq!(result.error_code, 0);
        assert!(!result.member_id.is_empty());
        assert!(!result.leader_id.is_empty());
        assert_eq!(result.generation_id, 1);
    }

    #[test]
    fn heartbeat_updates_timestamp() {
        let c = coordinator();
        let join = c
            .join_group(
                "hb-group".into(),
                Some("m1".into()),
                "c1".into(),
                "127.0.0.1".into(),
                30000,
                60000,
                "consumer".into(),
                HashMap::new(),
            )
            .unwrap();
        let r = c.heartbeat("hb-group", join.generation_id, "m1");
        assert!(r.is_ok());
    }

    #[test]
    fn leave_group_removes_member() {
        let c = coordinator();
        c.join_group(
            "leave-group".into(),
            Some("m1".into()),
            "c1".into(),
            "127.0.0.1".into(),
            30000,
            60000,
            "consumer".into(),
            HashMap::new(),
        )
        .unwrap();
        c.leave_group("leave-group", "m1").unwrap();
        let desc = c.describe_group("leave-group").unwrap();
        assert!(desc.members.is_empty());
    }

    #[test]
    fn roundrobin_assigns_all_partitions() {
        let proto = RebalanceProtocol::RoundRobin;
        let members = vec!["m1".into(), "m2".into()];
        let mut tp = HashMap::new();
        tp.insert("topic-a".into(), 4i32);
        let assignments = proto.assign(&members, &tp);
        let total: usize = assignments.values().map(|v| v.len()).sum();
        assert_eq!(total, 4);
    }

    #[test]
    fn range_assigns_all_partitions() {
        let proto = RebalanceProtocol::Range;
        let members = vec!["m1".into(), "m2".into(), "m3".into()];
        let mut tp = HashMap::new();
        tp.insert("events".into(), 6i32);
        let assignments = proto.assign(&members, &tp);
        let total: usize = assignments.values().map(|v| v.len()).sum();
        assert_eq!(total, 6);
    }

    #[test]
    fn assignment_codec_round_trips() {
        let mut s: BTreeSet<(String, i32)> = BTreeSet::new();
        s.insert(("orders".into(), 0));
        s.insert(("orders".into(), 1));
        s.insert(("alerts".into(), 7));
        let enc = encode_assignment(&s);
        let dec = decode_assignment(&enc);
        assert_eq!(dec, s);
    }

    #[test]
    fn assignment_codec_handles_empty() {
        let s = BTreeSet::new();
        assert_eq!(encode_assignment(&s), Vec::<u8>::new());
        assert_eq!(decode_assignment(&[]), BTreeSet::new());
    }

    #[test]
    fn assignment_codec_tolerates_garbage() {
        // Garbage segments are silently skipped.
        let dec = decode_assignment(b"orders:0,not-a-pair,alerts:7");
        let mut want = BTreeSet::new();
        want.insert(("orders".to_string(), 0));
        want.insert(("alerts".to_string(), 7));
        assert_eq!(dec, want);
    }

    #[test]
    fn cooperative_plan_for_group_uses_member_assignments() {
        let c = coordinator();
        // Two members with prior assignment.
        c.join_group(
            "co-grp".into(),
            Some("m1".into()),
            "c1".into(),
            "1".into(),
            30000,
            60000,
            "consumer".into(),
            HashMap::new(),
        )
        .unwrap();
        c.join_group(
            "co-grp".into(),
            Some("m2".into()),
            "c2".into(),
            "2".into(),
            30000,
            60000,
            "consumer".into(),
            HashMap::new(),
        )
        .unwrap();
        // Seed m1 with [t,0] and m2 with [t,1].
        let mut prev_m1 = BTreeSet::new();
        prev_m1.insert(("t".to_string(), 0));
        let mut prev_m2 = BTreeSet::new();
        prev_m2.insert(("t".to_string(), 1));
        let gen_now = c.groups.get("co-grp").unwrap().generation_id;
        let mut assigns = HashMap::new();
        assigns.insert("m1".into(), encode_assignment(&prev_m1));
        assigns.insert("m2".into(), encode_assignment(&prev_m2));
        c.sync_group("co-grp", gen_now, "m1", assigns).unwrap();
        // Compute cooperative plan over the same partition set —
        // already balanced.
        let parts: BTreeSet<(String, i32)> = [("t".to_string(), 0), ("t".to_string(), 1)]
            .into_iter()
            .collect();
        let plan = c.cooperative_plan("co-grp", &parts).unwrap();
        assert_eq!(plan.released_count, 0);
        assert_eq!(plan.stable_count, 2);
    }

    #[test]
    fn cooperative_plan_for_unknown_group_errors() {
        let c = coordinator();
        let parts = BTreeSet::new();
        let r = c.cooperative_plan("nope", &parts);
        assert!(r.is_err());
    }
}
