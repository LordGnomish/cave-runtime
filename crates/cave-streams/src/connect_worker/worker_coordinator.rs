// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/runtime/src/main/java/org/apache/kafka/connect/runtime/distributed/WorkerCoordinator.java
//   connect/runtime/src/main/java/org/apache/kafka/connect/runtime/distributed/ConnectProtocol.java

//! Connect-specific group coordinator state machine — heartbeats,
//! JoinGroup/SyncGroup, eager vs. incremental rebalance modes.
//!
//! Mirrors upstream `WorkerCoordinator`. cave-streams' [`DistributedHerder`]
//! handles the herder-side state (members, generation, lowest-id
//! leader). `WorkerCoordinator` wraps the herder with the
//! request/response shape of the Kafka group-coordinator wire
//! protocol so a worker can drive the rebalance through Join/Sync
//! calls.
//!
//! ## Subprotocols
//!
//! Upstream advertises two:
//!
//! * `default` (eager, stop-the-world): every member revokes
//!   everything, then re-receives the new assignment.
//! * `sessioned` (incremental, KIP-415): members revoke only the
//!   minimal slice; sticky-retention is the norm. Wired here through
//!   the [`IncrementalConnectAssignor`].

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{StreamsError, StreamsResult};

use super::assignor_incremental::{
    AssignmentUnit, ConnectAssignmentDelta, IncrementalConnectAssignor, PreviousAssignment,
};
use super::distributed_herder::{DistributedHerder, HerderState, MemberId};

/// Which rebalance protocol the cluster runs. Upstream calls this
/// the "subprotocol" of the Connect ConsumerGroupProtocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebalanceMode {
    /// Stop-the-world eager rebalance (`default`).
    Eager,
    /// Incremental cooperative (`sessioned`, KIP-415).
    Incremental,
}

impl RebalanceMode {
    pub fn as_subprotocol(self) -> &'static str {
        match self {
            Self::Eager => "default",
            Self::Incremental => "sessioned",
        }
    }
    pub fn parse(s: &str) -> StreamsResult<Self> {
        Ok(match s {
            "default" | "eager" => Self::Eager,
            "sessioned" | "incremental" => Self::Incremental,
            other => {
                return Err(StreamsError::Internal(format!(
                    "WorkerCoordinator: unknown subprotocol '{other}'"
                )))
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGroupRequest {
    pub member_id: MemberId,
    /// Subprotocols the worker speaks, highest-pref first.
    pub supported_modes: Vec<RebalanceMode>,
    /// Worker-side declared connectors+tasks it should be running.
    /// The leader uses the union across all JoinGroup requests as
    /// the assignor input.
    pub desired_units: BTreeSet<AssignmentUnit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGroupResponse {
    /// The agreed-upon mode (intersection of every member's
    /// supported_modes; falls back to Eager).
    pub mode: RebalanceMode,
    pub generation: u64,
    pub leader: MemberId,
    /// True when *this* member is the leader.
    pub is_leader: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncGroupRequest {
    pub member_id: MemberId,
    pub generation: u64,
    /// If the requester is the leader, it ships the leader-computed
    /// assignment. Followers send `None` and receive the assignment
    /// the leader earlier published.
    pub leader_assignment: Option<ConnectAssignmentDelta>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncGroupResponse {
    pub mode: RebalanceMode,
    pub generation: u64,
    pub revoked: BTreeSet<AssignmentUnit>,
    pub assigned: BTreeSet<AssignmentUnit>,
    pub final_set: BTreeSet<AssignmentUnit>,
}

/// Coordinator-side events surfaced to the operator UI / metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorEvent {
    MemberJoined { member: MemberId, generation: u64 },
    MemberLeft { member: MemberId, generation: u64 },
    Rebalanced { generation: u64, deferred: usize },
    HeartbeatMissed { member: MemberId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorState {
    /// No members at all.
    Empty,
    /// At least one member but no assignment computed yet.
    PreparingRebalance,
    /// Leader has run the assignor; followers should SyncGroup.
    AwaitingSync,
    /// Steady state — heartbeats only.
    Stable,
}

/// State machine that fronts the herder and the incremental
/// assignor. Tracks per-member supported modes + last-heartbeat clock
/// to surface heartbeat-missed events. The clock is exposed as a
/// `u64` (the herder's tick counter), so test callers control time.
pub struct WorkerCoordinator {
    herder: DistributedHerder,
    assignor: IncrementalConnectAssignor,
    state: CoordinatorState,
    mode: RebalanceMode,
    supported_per_member: BTreeMap<MemberId, Vec<RebalanceMode>>,
    desired_per_member: BTreeMap<MemberId, BTreeSet<AssignmentUnit>>,
    /// Last heartbeat clock per member. Compared against
    /// `session_timeout` (in clock ticks) to detect dead members.
    last_heartbeat: BTreeMap<MemberId, u64>,
    /// Pending sync — leader-computed delta, awaiting follower
    /// SyncGroup calls.
    pending_assignment: Option<ConnectAssignmentDelta>,
    /// Number of ticks a member can skip before being evicted.
    /// Default 3 (consumer Kafka default).
    pub session_timeout_ticks: u64,
}

impl Default for WorkerCoordinator {
    fn default() -> Self {
        Self {
            herder: DistributedHerder::new(),
            assignor: IncrementalConnectAssignor::new(),
            state: CoordinatorState::Empty,
            mode: RebalanceMode::Eager,
            supported_per_member: BTreeMap::new(),
            desired_per_member: BTreeMap::new(),
            last_heartbeat: BTreeMap::new(),
            pending_assignment: None,
            session_timeout_ticks: 3,
        }
    }
}

impl WorkerCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> CoordinatorState {
        self.state
    }

    pub fn mode(&self) -> RebalanceMode {
        self.mode
    }

    pub fn members(&self) -> Vec<MemberId> {
        self.herder.members().to_vec()
    }

    pub fn generation(&self) -> u64 {
        self.herder.generation()
    }

    /// Process a JoinGroup request — the worker tells us its
    /// supported modes + the connectors+tasks it expects to run.
    pub fn join_group(
        &mut self,
        req: JoinGroupRequest,
    ) -> StreamsResult<JoinGroupResponse> {
        if req.supported_modes.is_empty() {
            return Err(StreamsError::Internal(
                "JoinGroup: supported_modes must be non-empty".into(),
            ));
        }
        self.herder.join(req.member_id.clone());
        self.supported_per_member
            .insert(req.member_id.clone(), req.supported_modes.clone());
        self.desired_per_member
            .insert(req.member_id.clone(), req.desired_units.clone());
        self.last_heartbeat
            .insert(req.member_id.clone(), self.herder.clock());

        // Negotiate the mode: pick the highest-pref mode every
        // member supports.
        self.mode = self.negotiate_mode();

        // Move state machine: at least one member → PreparingRebalance.
        self.state = CoordinatorState::PreparingRebalance;

        let leader = self
            .herder
            .leader()
            .cloned()
            .ok_or_else(|| StreamsError::Internal("JoinGroup: no leader after join".into()))?;
        Ok(JoinGroupResponse {
            mode: self.mode,
            generation: self.herder.generation(),
            is_leader: leader == req.member_id,
            leader,
        })
    }

    /// Process a LeaveGroup — drop the member, bump generation,
    /// move to PreparingRebalance.
    pub fn leave_group(&mut self, member: &MemberId) -> CoordinatorEvent {
        self.herder.leave(member.clone());
        self.supported_per_member.remove(member);
        self.desired_per_member.remove(member);
        self.last_heartbeat.remove(member);
        // After leave, if there are still members, we need a re-sync.
        self.state = match self.herder.state() {
            HerderState::Empty => CoordinatorState::Empty,
            _ => CoordinatorState::PreparingRebalance,
        };
        // Renegotiate mode against the remaining set.
        self.mode = self.negotiate_mode();
        CoordinatorEvent::MemberLeft {
            member: member.clone(),
            generation: self.herder.generation(),
        }
    }

    /// Leader: compute the assignment for *all* members. Caller (the
    /// leader) ships the result via `sync_group(_, Some(delta))`.
    ///
    /// `scheduled_delay_used_ms` lets the caller honour the
    /// incremental delay budget.
    pub fn compute_assignment(
        &mut self,
        scheduled_delay_used_ms: u64,
    ) -> ConnectAssignmentDelta {
        let members = self.herder.members().to_vec();
        let desired: BTreeSet<AssignmentUnit> = self
            .desired_per_member
            .values()
            .flat_map(|s| s.iter().cloned())
            .collect();
        let delta = match self.mode {
            RebalanceMode::Incremental => {
                self.assignor.assign(&members, &desired, scheduled_delay_used_ms)
            }
            RebalanceMode::Eager => {
                // Eager → revoke everything + reassign. Seed the
                // assignor with empty previous-assignments so every
                // unit is `assigned`.
                let mut empty = BTreeMap::new();
                for m in &members {
                    empty.insert(m.clone(), PreviousAssignment::default());
                }
                self.assignor.seed_previous(empty);
                self.assignor.assign(&members, &desired, scheduled_delay_used_ms)
            }
        };
        self.state = CoordinatorState::AwaitingSync;
        self.pending_assignment = Some(delta.clone());
        // Also push the herder into Stable for compat with existing
        // tests that read herder state.
        self.herder.register_tasks(
            &delta
                .per_worker
                .values()
                .flat_map(|r| r.final_set.iter().map(|u| u.key()))
                .collect::<Vec<_>>()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
        );
        delta
    }

    /// Process a SyncGroup. The leader's call includes the
    /// assignment; the leader's view is what the follower receives.
    /// Followers' calls receive whatever the leader earlier published.
    pub fn sync_group(
        &mut self,
        req: SyncGroupRequest,
    ) -> StreamsResult<SyncGroupResponse> {
        let leader = self
            .herder
            .leader()
            .cloned()
            .ok_or_else(|| StreamsError::Internal("SyncGroup: no leader".into()))?;
        if req.generation != self.herder.generation() {
            return Err(StreamsError::Internal(format!(
                "SyncGroup: stale generation {} vs {}",
                req.generation,
                self.herder.generation()
            )));
        }
        let delta = if req.member_id == leader {
            // Leader supplies the delta.
            let delta = req.leader_assignment.ok_or_else(|| {
                StreamsError::Internal("SyncGroup: leader must ship leader_assignment".into())
            })?;
            self.pending_assignment = Some(delta.clone());
            delta
        } else {
            self.pending_assignment.clone().ok_or_else(|| {
                StreamsError::Internal("SyncGroup: leader has not synced yet".into())
            })?
        };

        let report = delta
            .per_worker
            .get(&req.member_id)
            .cloned()
            .unwrap_or_default();
        // Transition to Stable once the leader has synced (we don't
        // strictly know when every follower has, but Kafka's model
        // is "stable as soon as the leader has published").
        if req.member_id == leader {
            self.state = CoordinatorState::Stable;
        }
        Ok(SyncGroupResponse {
            mode: self.mode,
            generation: self.herder.generation(),
            revoked: report.revoked,
            assigned: report.assigned,
            final_set: report.final_set,
        })
    }

    /// Heartbeat — record latest tick + return current generation.
    pub fn heartbeat(
        &mut self,
        member: &MemberId,
        generation: u64,
    ) -> StreamsResult<u64> {
        let cur = self
            .herder
            .heartbeat(member.as_str(), generation)
            .map_err(|e| StreamsError::Internal(format!("heartbeat: {e}")))?;
        self.last_heartbeat
            .insert(member.clone(), self.herder.clock());
        Ok(cur)
    }

    /// Move the coordinator clock forward. Evicts any member whose
    /// last heartbeat is older than `session_timeout_ticks`. Returns
    /// the events emitted.
    pub fn tick(&mut self) -> Vec<CoordinatorEvent> {
        self.herder.tick();
        let now = self.herder.clock();
        let mut events = Vec::new();
        // Snapshot members so we can mutate in the loop.
        let members: Vec<MemberId> = self.herder.members().to_vec();
        for m in members {
            let last = self.last_heartbeat.get(&m).copied().unwrap_or(0);
            if now.saturating_sub(last) > self.session_timeout_ticks {
                events.push(CoordinatorEvent::HeartbeatMissed { member: m.clone() });
                events.push(self.leave_group(&m));
            }
        }
        events
    }

    /// Negotiate the strongest mode every alive member supports.
    /// Preference order: Incremental > Eager.
    fn negotiate_mode(&self) -> RebalanceMode {
        if self.supported_per_member.is_empty() {
            return RebalanceMode::Eager;
        }
        let all_support_incremental = self
            .supported_per_member
            .values()
            .all(|modes| modes.contains(&RebalanceMode::Incremental));
        if all_support_incremental {
            RebalanceMode::Incremental
        } else {
            RebalanceMode::Eager
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(c: &str, t: u32) -> AssignmentUnit {
        AssignmentUnit::Task {
            connector: c.into(),
            task: t,
        }
    }

    fn join(
        coord: &mut WorkerCoordinator,
        member: &str,
        modes: &[RebalanceMode],
        desired: Vec<AssignmentUnit>,
    ) -> JoinGroupResponse {
        let req = JoinGroupRequest {
            member_id: member.into(),
            supported_modes: modes.to_vec(),
            desired_units: desired.into_iter().collect(),
        };
        coord.join_group(req).unwrap()
    }

    #[test]
    fn join_first_member_becomes_leader() {
        let mut c = WorkerCoordinator::new();
        let r = join(
            &mut c,
            "w1",
            &[RebalanceMode::Incremental, RebalanceMode::Eager],
            vec![unit("c", 0)],
        );
        assert!(r.is_leader);
        assert_eq!(r.leader, "w1".into());
        assert_eq!(r.generation, 1);
        assert_eq!(c.state(), CoordinatorState::PreparingRebalance);
    }

    #[test]
    fn join_negotiates_incremental_when_all_support_it() {
        let mut c = WorkerCoordinator::new();
        join(
            &mut c,
            "w1",
            &[RebalanceMode::Incremental, RebalanceMode::Eager],
            vec![],
        );
        join(
            &mut c,
            "w2",
            &[RebalanceMode::Incremental, RebalanceMode::Eager],
            vec![],
        );
        assert_eq!(c.mode(), RebalanceMode::Incremental);
    }

    #[test]
    fn join_falls_back_to_eager_when_member_lacks_incremental() {
        let mut c = WorkerCoordinator::new();
        join(&mut c, "w1", &[RebalanceMode::Incremental], vec![]);
        join(&mut c, "w2", &[RebalanceMode::Eager], vec![]);
        assert_eq!(c.mode(), RebalanceMode::Eager);
    }

    #[test]
    fn join_empty_supported_modes_errors() {
        let mut c = WorkerCoordinator::new();
        let r = c.join_group(JoinGroupRequest {
            member_id: "w1".into(),
            supported_modes: vec![],
            desired_units: BTreeSet::new(),
        });
        assert!(r.is_err());
    }

    #[test]
    fn compute_assignment_returns_per_worker_breakdown() {
        let mut c = WorkerCoordinator::new();
        join(
            &mut c,
            "w1",
            &[RebalanceMode::Incremental, RebalanceMode::Eager],
            vec![unit("c", 0), unit("c", 1)],
        );
        join(
            &mut c,
            "w2",
            &[RebalanceMode::Incremental, RebalanceMode::Eager],
            vec![unit("c", 2), unit("c", 3)],
        );
        let delta = c.compute_assignment(0);
        let w1 = delta.per_worker.get(&MemberId::from("w1")).unwrap();
        let w2 = delta.per_worker.get(&MemberId::from("w2")).unwrap();
        let total = w1.final_set.len() + w2.final_set.len();
        assert_eq!(total, 4);
        assert_eq!(c.state(), CoordinatorState::AwaitingSync);
    }

    #[test]
    fn leader_sync_group_publishes_assignment_to_followers() {
        let mut c = WorkerCoordinator::new();
        join(&mut c, "w1", &[RebalanceMode::Incremental], vec![unit("c", 0)]);
        join(&mut c, "w2", &[RebalanceMode::Incremental], vec![unit("c", 1)]);
        let delta = c.compute_assignment(0);
        let generation = c.generation();
        // Leader is w1 (lowest id).
        let leader_resp = c
            .sync_group(SyncGroupRequest {
                member_id: "w1".into(),
                generation,
                leader_assignment: Some(delta.clone()),
            })
            .unwrap();
        assert!(!leader_resp.final_set.is_empty());
        let follower_resp = c
            .sync_group(SyncGroupRequest {
                member_id: "w2".into(),
                generation,
                leader_assignment: None,
            })
            .unwrap();
        assert!(!follower_resp.final_set.is_empty());
        assert_eq!(c.state(), CoordinatorState::Stable);
    }

    #[test]
    fn follower_sync_before_leader_errors() {
        let mut c = WorkerCoordinator::new();
        join(&mut c, "w1", &[RebalanceMode::Incremental], vec![]);
        join(&mut c, "w2", &[RebalanceMode::Incremental], vec![]);
        // No compute_assignment yet → follower sync without delta errors.
        let res = c.sync_group(SyncGroupRequest {
            member_id: "w2".into(),
            generation: c.generation(),
            leader_assignment: None,
        });
        assert!(res.is_err());
    }

    #[test]
    fn sync_group_stale_generation_errors() {
        let mut c = WorkerCoordinator::new();
        join(&mut c, "w1", &[RebalanceMode::Incremental], vec![]);
        let res = c.sync_group(SyncGroupRequest {
            member_id: "w1".into(),
            generation: 999,
            leader_assignment: None,
        });
        assert!(res.is_err());
    }

    #[test]
    fn heartbeat_round_trips_for_known_member() {
        let mut c = WorkerCoordinator::new();
        join(&mut c, "w1", &[RebalanceMode::Incremental], vec![]);
        let generation = c.generation();
        let returned = c.heartbeat(&"w1".into(), generation).unwrap();
        assert_eq!(returned, generation);
    }

    #[test]
    fn heartbeat_unknown_member_errors() {
        let mut c = WorkerCoordinator::new();
        join(&mut c, "w1", &[RebalanceMode::Incremental], vec![]);
        assert!(c.heartbeat(&"w99".into(), 1).is_err());
    }

    #[test]
    fn tick_evicts_silent_members() {
        let mut c = WorkerCoordinator::new();
        c.session_timeout_ticks = 2;
        join(&mut c, "w1", &[RebalanceMode::Incremental], vec![]);
        join(&mut c, "w2", &[RebalanceMode::Incremental], vec![]);
        // Heartbeat from w1 only — w2 will fall behind. Aggregate
        // events across the full window because the exact tick on
        // which `now - last > session_timeout_ticks` flips depends
        // on whether the heartbeat lands before or after the tick
        // bump; the contract is "w2 is evicted within a few ticks".
        let mut all_events: Vec<CoordinatorEvent> = Vec::new();
        for _ in 0..6 {
            all_events.extend(c.tick());
            // Keep w1 alive at every iteration.
            if c.members().contains(&"w1".into()) {
                let _ = c.heartbeat(&"w1".into(), c.generation());
            }
        }
        assert!(
            all_events
                .iter()
                .any(|e| matches!(e, CoordinatorEvent::HeartbeatMissed { member } if member == &MemberId::from("w2"))),
            "expected heartbeat-missed for w2 in events stream: {all_events:?}"
        );
        assert!(!c.members().contains(&"w2".into()));
    }

    #[test]
    fn leave_group_drops_member_and_renegotiates() {
        let mut c = WorkerCoordinator::new();
        join(&mut c, "w1", &[RebalanceMode::Incremental], vec![]);
        join(&mut c, "w2", &[RebalanceMode::Eager], vec![]);
        assert_eq!(c.mode(), RebalanceMode::Eager);
        c.leave_group(&"w2".into());
        assert_eq!(c.mode(), RebalanceMode::Incremental);
        assert!(!c.members().contains(&"w2".into()));
    }

    #[test]
    fn eager_mode_revokes_everything_on_rebalance() {
        let mut c = WorkerCoordinator::new();
        join(&mut c, "w1", &[RebalanceMode::Eager], vec![unit("c", 0), unit("c", 1)]);
        let d1 = c.compute_assignment(0);
        let w1 = &d1.per_worker[&MemberId::from("w1")];
        // First gen: every unit is assigned (no previous), revoked empty.
        assert_eq!(w1.assigned.len(), 2);
        // Now join w2 in Eager mode.
        join(&mut c, "w2", &[RebalanceMode::Eager], vec![]);
        let d2 = c.compute_assignment(0);
        // In Eager, the assignor sees empty previous → every retained
        // unit appears as assigned again.
        let total_assigned: usize = d2.per_worker.values().map(|r| r.assigned.len()).sum();
        assert_eq!(total_assigned, 2);
    }

    #[test]
    fn subprotocol_strings_match_upstream() {
        assert_eq!(RebalanceMode::Eager.as_subprotocol(), "default");
        assert_eq!(
            RebalanceMode::Incremental.as_subprotocol(),
            "sessioned"
        );
        assert_eq!(
            RebalanceMode::parse("sessioned").unwrap(),
            RebalanceMode::Incremental
        );
        assert!(RebalanceMode::parse("bogus").is_err());
    }

    #[test]
    fn member_id_ordering_picks_lowest_as_leader() {
        let mut c = WorkerCoordinator::new();
        join(&mut c, "w2", &[RebalanceMode::Incremental], vec![]);
        let r = join(&mut c, "w1", &[RebalanceMode::Incremental], vec![]);
        assert_eq!(r.leader, "w1".into());
    }
}
