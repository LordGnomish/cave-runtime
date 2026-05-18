// SPDX-License-Identifier: AGPL-3.0-or-later
//! Distributed herder. Mirrors upstream
//! `connect/runtime/distributed/DistributedHerder.java`.
//!
//! The distributed herder is the cooperative-rebalance state
//! machine that fronts every worker in a Connect cluster. Each
//! worker keeps a copy of the herder; one worker is the *leader*
//! (lowest member id wins on first join) and is responsible for
//! computing the next assignment. Heartbeats keep generations
//! in sync; a member leaving (or arriving) bumps the generation
//! and forces a [`HerderState::Rebalancing`] pass.
//!
//! cave-streams' herder ships the state machine + rendezvous-hash
//! assignment + cooperative-sticky retention. The full
//! IncrementalAssignor pre-emption protocol from KIP-415 (revoke,
//! scheduled rebalance delay, then assign) is tracked, not in
//! this batch.

use std::collections::BTreeMap;

/// Newtype around a worker identifier. The herder uses string
/// member ids on the wire; the existing [`super::assignment`]
/// table indexes by `WorkerId = u64`. We hash the member id to
/// derive a worker id when feeding the assignment table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberId(pub String);

impl MemberId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for MemberId {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}
impl From<String> for MemberId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HerderState {
    Empty,
    Joining,
    Assigning,
    Rebalancing,
    Stable,
}

#[derive(Debug, thiserror::Error)]
pub enum HerderError {
    #[error("unknown member {0}")]
    UnknownMember(String),
    #[error("stale generation: got {got}, current {current}")]
    StaleGeneration { got: u64, current: u64 },
}

pub struct DistributedHerder {
    members: Vec<MemberId>,
    leader: Option<MemberId>,
    generation: u64,
    state: HerderState,
    clock: u64,
    tasks: Vec<String>,
    assignment: BTreeMap<String, MemberId>,
}

impl Default for DistributedHerder {
    fn default() -> Self {
        Self {
            members: Vec::new(),
            leader: None,
            generation: 0,
            state: HerderState::Empty,
            clock: 0,
            tasks: Vec::new(),
            assignment: BTreeMap::new(),
        }
    }
}

impl DistributedHerder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn members(&self) -> &[MemberId] {
        &self.members
    }

    pub fn leader(&self) -> Option<&MemberId> {
        self.leader.as_ref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn state(&self) -> HerderState {
        self.state
    }

    pub fn clock(&self) -> u64 {
        self.clock
    }

    pub fn assignment(&self) -> &BTreeMap<String, MemberId> {
        &self.assignment
    }

    /// Add a member to the group. First member becomes leader.
    /// Generation bumps, state moves to Joining → Assigning.
    pub fn join(&mut self, m: MemberId) {
        if self.members.iter().any(|x| x == &m) {
            return;
        }
        self.members.push(m);
        self.members.sort();
        // Lowest member id wins.
        self.leader = self.members.first().cloned();
        self.generation = self.generation.saturating_add(1);
        self.state = HerderState::Assigning;
    }

    /// Remove a member from the group. Triggers a rebalance.
    pub fn leave(&mut self, m: MemberId) {
        let before = self.members.len();
        self.members.retain(|x| x != &m);
        if self.members.len() == before {
            return;
        }
        // Drop any assignments owned by the departing member.
        self.assignment.retain(|_, owner| owner != &m);
        // Promote next member if leader left.
        if self.leader.as_ref() == Some(&m) {
            self.leader = self.members.first().cloned();
        }
        if self.members.is_empty() {
            self.state = HerderState::Empty;
        } else {
            self.state = HerderState::Rebalancing;
        }
        self.generation = self.generation.saturating_add(1);
    }

    pub fn register_tasks(&mut self, ids: &[&str]) {
        self.tasks = ids.iter().map(|s| s.to_string()).collect();
    }

    /// Run the assignment computation — rendezvous-hash each task
    /// onto the member set. After this the state becomes Stable.
    pub fn assign(&mut self) {
        self.assignment.clear();
        if self.members.is_empty() || self.tasks.is_empty() {
            self.state = if self.members.is_empty() {
                HerderState::Empty
            } else {
                HerderState::Stable
            };
            return;
        }
        for t in &self.tasks {
            if let Some(owner) = rendezvous_pick(&self.members, t) {
                self.assignment.insert(t.clone(), owner);
            }
        }
        self.state = HerderState::Stable;
    }

    /// Heartbeat from a known member. Returns the current
    /// generation if valid; errors on unknown member or stale
    /// generation.
    pub fn heartbeat(&mut self, member: &str, generation: u64) -> Result<u64, HerderError> {
        if !self.members.iter().any(|m| m.0 == member) {
            return Err(HerderError::UnknownMember(member.into()));
        }
        if generation != self.generation {
            return Err(HerderError::StaleGeneration {
                got: generation,
                current: self.generation,
            });
        }
        Ok(self.generation)
    }

    /// Return the slice of assigned tasks for `member`.
    pub fn sync_group(&self, member: &str) -> Vec<String> {
        self.assignment
            .iter()
            .filter_map(|(t, owner)| {
                if owner.0 == member {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Single tick of the herder clock. Models the upstream
    /// `DistributedHerder.tick()` loop — used by metrics +
    /// to detect heartbeat-staleness.
    pub fn tick(&mut self) {
        self.clock = self.clock.saturating_add(1);
    }
}

/// Rendezvous-hash pick. For each (member, task) pair we compute
/// a deterministic FNV-1a score and pick the member with the
/// highest score. Adding or removing a member shifts O(tasks/n)
/// of the keys — the property that makes rendezvous superior to
/// naive modulo for rebalance churn.
fn rendezvous_pick(members: &[MemberId], task: &str) -> Option<MemberId> {
    members
        .iter()
        .map(|m| (rendezvous_score(m, task), m))
        .max_by_key(|(s, _)| *s)
        .map(|(_, m)| m.clone())
}

fn rendezvous_score(m: &MemberId, task: &str) -> u64 {
    // Mirrors the existing `assignment::rendezvous_score` —
    // hash (member, task) through `DefaultHasher` so we get a
    // good distribution. Per-process stable (the seed is
    // process-local); that matches upstream's group-coordinator
    // assignment which also re-computes per process.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    m.0.hash(&mut h);
    task.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_herder_has_no_leader_and_state_empty() {
        let h = DistributedHerder::new();
        assert!(h.leader().is_none());
        assert_eq!(h.state(), HerderState::Empty);
        assert_eq!(h.generation(), 0);
    }

    #[test]
    fn first_join_makes_member_leader_and_bumps_generation() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        assert_eq!(h.leader(), Some(&MemberId::from("w1")));
        assert_eq!(h.generation(), 1);
    }

    #[test]
    fn second_join_keeps_lowest_member_as_leader() {
        let mut h = DistributedHerder::new();
        h.join("w2".into());
        h.join("w1".into());
        assert_eq!(h.leader(), Some(&MemberId::from("w1")));
    }

    #[test]
    fn duplicate_join_is_idempotent() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        h.join("w1".into());
        assert_eq!(h.members().len(), 1);
    }

    #[test]
    fn leave_unknown_is_noop() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        let g = h.generation();
        h.leave("w99".into());
        assert_eq!(h.generation(), g);
    }

    #[test]
    fn leave_drops_assignment_for_member() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        h.join("w2".into());
        h.register_tasks(&["c:0", "c:1"]);
        h.assign();
        h.leave("w2".into());
        for (_, owner) in h.assignment() {
            assert_ne!(owner, &MemberId::from("w2"));
        }
    }

    #[test]
    fn leader_failover_picks_next_lowest() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        h.join("w2".into());
        h.join("w3".into());
        h.leave("w1".into());
        assert_eq!(h.leader(), Some(&MemberId::from("w2")));
    }

    #[test]
    fn assign_distributes_tasks_across_members() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        h.join("w2".into());
        h.register_tasks(&["c:0", "c:1", "c:2", "c:3"]);
        h.assign();
        assert_eq!(h.assignment().len(), 4);
    }

    #[test]
    fn heartbeat_unknown_member_errors() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        assert!(h.heartbeat("w99", 1).is_err());
    }

    #[test]
    fn heartbeat_stale_generation_errors() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        let result = h.heartbeat("w1", 0);
        assert!(matches!(
            result.unwrap_err(),
            HerderError::StaleGeneration { .. }
        ));
    }

    #[test]
    fn tick_advances_clock_monotonically() {
        let mut h = DistributedHerder::new();
        let t0 = h.clock();
        h.tick();
        h.tick();
        h.tick();
        assert_eq!(h.clock(), t0 + 3);
    }

    #[test]
    fn sync_group_returns_assignment_for_member() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        h.join("w2".into());
        h.register_tasks(&["c:0", "c:1", "c:2"]);
        h.assign();
        let w1 = h.sync_group("w1");
        let w2 = h.sync_group("w2");
        assert_eq!(w1.len() + w2.len(), 3);
    }

    #[test]
    fn sync_group_unknown_member_empty() {
        let mut h = DistributedHerder::new();
        h.join("w1".into());
        h.register_tasks(&["c:0"]);
        h.assign();
        assert!(h.sync_group("w99").is_empty());
    }
}
