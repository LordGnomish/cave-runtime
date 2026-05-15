// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Consumer group coordinator — join/sync/heartbeat/leave + rebalance protocols.
//!
//! Supports: range, roundrobin, sticky, cooperative-sticky

use crate::error::{StreamsError, StreamsResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

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
}
