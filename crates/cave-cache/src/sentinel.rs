// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sentinel HA.
//!
//! Ports the master-failover half of upstream Redis's `src/sentinel.c`.
//! A Sentinel is a process that monitors a designated master, gossips
//! its view of liveness to peer sentinels, and — once quorum agrees —
//! promotes a replica to master.
//!
//! This module owns the pure state machine: monitor entries, the
//! per-master subjective-down / objective-down ladder, peer
//! quorum voting, and the leader-election + failover sequence. It
//! deliberately does not own the TCP/SCRIPTING side — those are bound
//! in the server crate. Each transition is a state machine step, and
//! every test drives it with a deterministic clock.

use std::collections::{HashMap, HashSet};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SentinelError {
    #[error("master {0} not monitored")]
    UnknownMaster(String),
    #[error("master {0} already monitored")]
    AlreadyMonitored(String),
    #[error("no eligible replica for failover")]
    NoEligibleReplica,
    #[error("quorum {required} not reached (got {observed})")]
    QuorumUnreached { required: u32, observed: u32 },
}

/// Subjective / objective down ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterState {
    /// Master answers PING.
    Up,
    /// We lost contact past `down_after_ms` — *we* think it's down.
    SubjectiveDown,
    /// Quorum of peer sentinels agreed it's down.
    ObjectiveDown,
    /// Leader sentinel is mid-failover.
    FailingOver,
    /// New master promoted; old master entry removed.
    PromotedAway,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaInfo {
    /// `host:port` form.
    pub addr: String,
    /// Replication offset reported by INFO replication.
    pub offset: u64,
    /// Last-seen instant (logical clock).
    pub last_seen_ms: u64,
    /// Priority: lower wins (cf. `slave-priority` in upstream).
    pub priority: u8,
}

#[derive(Debug, Clone)]
pub struct MonitoredMaster {
    pub name: String,
    pub addr: String,
    pub quorum: u32,
    pub down_after_ms: u64,
    pub state: MasterState,
    pub last_ping_ok_ms: u64,
    pub replicas: Vec<ReplicaInfo>,
    /// `sentinel_id → last vote epoch` — peers who agreed this master
    /// is `SubjectiveDown` at the recorded logical time.
    pub odown_votes: HashSet<String>,
    /// Configuration epoch — bumped on every elected failover.
    pub config_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailoverPlan {
    /// Master being failed over.
    pub master_name: String,
    /// The chosen replica that will become the new master.
    pub promoted_addr: String,
    /// All known replicas after promotion (excluding the promoted one,
    /// which becomes the new master).
    pub new_replica_addrs: Vec<String>,
    pub new_config_epoch: u64,
}

#[derive(Debug, Default)]
pub struct Sentinel {
    pub id: String,
    masters: HashMap<String, MonitoredMaster>,
    /// Current logical clock — bumped by [`Sentinel::advance_clock`].
    now_ms: u64,
}

impl Sentinel {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            masters: HashMap::new(),
            now_ms: 0,
        }
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    pub fn advance_clock(&mut self, delta_ms: u64) {
        self.now_ms += delta_ms;
    }

    /// `SENTINEL MONITOR <name> <addr> <quorum>`.
    pub fn monitor(
        &mut self,
        name: impl Into<String>,
        addr: impl Into<String>,
        quorum: u32,
        down_after_ms: u64,
    ) -> Result<(), SentinelError> {
        let name = name.into();
        if self.masters.contains_key(&name) {
            return Err(SentinelError::AlreadyMonitored(name));
        }
        let now = self.now_ms;
        self.masters.insert(
            name.clone(),
            MonitoredMaster {
                name,
                addr: addr.into(),
                quorum,
                down_after_ms,
                state: MasterState::Up,
                last_ping_ok_ms: now,
                replicas: Vec::new(),
                odown_votes: HashSet::new(),
                config_epoch: 0,
            },
        );
        Ok(())
    }

    /// `SENTINEL REMOVE <name>`.
    pub fn remove(&mut self, name: &str) -> Result<(), SentinelError> {
        self.masters
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| SentinelError::UnknownMaster(name.into()))
    }

    pub fn get(&self, name: &str) -> Option<&MonitoredMaster> {
        self.masters.get(name)
    }

    pub fn list(&self) -> Vec<&MonitoredMaster> {
        self.masters.values().collect()
    }

    /// Note a successful PING reply from a master.
    pub fn record_ping_ok(&mut self, name: &str) -> Result<(), SentinelError> {
        let now = self.now_ms;
        let m = self
            .masters
            .get_mut(name)
            .ok_or_else(|| SentinelError::UnknownMaster(name.into()))?;
        m.last_ping_ok_ms = now;
        if matches!(m.state, MasterState::SubjectiveDown | MasterState::ObjectiveDown) {
            m.state = MasterState::Up;
            m.odown_votes.clear();
        }
        Ok(())
    }

    /// Re-evaluate the master's state given the current clock. Flips
    /// `Up → SubjectiveDown` when `now - last_ping_ok > down_after_ms`.
    pub fn tick(&mut self, name: &str) -> Result<MasterState, SentinelError> {
        let now = self.now_ms;
        let m = self
            .masters
            .get_mut(name)
            .ok_or_else(|| SentinelError::UnknownMaster(name.into()))?;
        if matches!(m.state, MasterState::Up)
            && now.saturating_sub(m.last_ping_ok_ms) >= m.down_after_ms
        {
            m.state = MasterState::SubjectiveDown;
        }
        Ok(m.state)
    }

    /// Register a peer sentinel's vote that the master is down.
    /// `peer_sentinel_id` is the gossipping sentinel's id.
    pub fn record_peer_down_vote(
        &mut self,
        name: &str,
        peer_sentinel_id: impl Into<String>,
    ) -> Result<MasterState, SentinelError> {
        let m = self
            .masters
            .get_mut(name)
            .ok_or_else(|| SentinelError::UnknownMaster(name.into()))?;
        m.odown_votes.insert(peer_sentinel_id.into());
        // Quorum reached → escalate to ObjectiveDown.
        if m.odown_votes.len() as u32 >= m.quorum
            && matches!(m.state, MasterState::SubjectiveDown)
        {
            m.state = MasterState::ObjectiveDown;
        }
        Ok(m.state)
    }

    pub fn register_replica(&mut self, name: &str, replica: ReplicaInfo) -> Result<(), SentinelError> {
        let m = self
            .masters
            .get_mut(name)
            .ok_or_else(|| SentinelError::UnknownMaster(name.into()))?;
        // De-dup by addr.
        m.replicas.retain(|r| r.addr != replica.addr);
        m.replicas.push(replica);
        Ok(())
    }

    pub fn forget_replica(&mut self, name: &str, addr: &str) -> Result<(), SentinelError> {
        let m = self
            .masters
            .get_mut(name)
            .ok_or_else(|| SentinelError::UnknownMaster(name.into()))?;
        m.replicas.retain(|r| r.addr != addr);
        Ok(())
    }

    /// Pick the best replica for promotion. Tie-breakers (in order):
    /// 1. Lowest `priority` value (0 is excluded — Redis convention
    ///    treats 0 as "never promote").
    /// 2. Highest replication `offset`.
    /// 3. Lexicographically smallest `addr` (deterministic).
    pub fn select_promotion_candidate(&self, name: &str) -> Result<&ReplicaInfo, SentinelError> {
        let m = self
            .masters
            .get(name)
            .ok_or_else(|| SentinelError::UnknownMaster(name.into()))?;
        m.replicas
            .iter()
            .filter(|r| r.priority > 0)
            .min_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then(b.offset.cmp(&a.offset))
                    .then(a.addr.cmp(&b.addr))
            })
            .ok_or(SentinelError::NoEligibleReplica)
    }

    /// Begin failover: transition state, bump epoch, return the plan.
    /// Errors if quorum not met yet.
    pub fn begin_failover(&mut self, name: &str) -> Result<FailoverPlan, SentinelError> {
        let candidate_addr = self.select_promotion_candidate(name)?.addr.clone();
        let m = self
            .masters
            .get_mut(name)
            .ok_or_else(|| SentinelError::UnknownMaster(name.into()))?;
        let observed = m.odown_votes.len() as u32;
        if !matches!(m.state, MasterState::ObjectiveDown) {
            return Err(SentinelError::QuorumUnreached {
                required: m.quorum,
                observed,
            });
        }
        m.state = MasterState::FailingOver;
        m.config_epoch += 1;
        let new_replicas: Vec<String> = m
            .replicas
            .iter()
            .filter(|r| r.addr != candidate_addr)
            .map(|r| r.addr.clone())
            // The deposed master becomes a replica of the new one.
            .chain(std::iter::once(m.addr.clone()))
            .collect();
        Ok(FailoverPlan {
            master_name: m.name.clone(),
            promoted_addr: candidate_addr,
            new_replica_addrs: new_replicas,
            new_config_epoch: m.config_epoch,
        })
    }

    /// Commit the failover: replace the master's addr, rotate replicas.
    pub fn commit_failover(&mut self, plan: &FailoverPlan) -> Result<(), SentinelError> {
        let m = self
            .masters
            .get_mut(&plan.master_name)
            .ok_or_else(|| SentinelError::UnknownMaster(plan.master_name.clone()))?;
        let now = self.now_ms;
        m.addr = plan.promoted_addr.clone();
        m.state = MasterState::Up;
        m.last_ping_ok_ms = now;
        m.odown_votes.clear();
        m.replicas = plan
            .new_replica_addrs
            .iter()
            .map(|a| ReplicaInfo {
                addr: a.clone(),
                offset: 0,
                last_seen_ms: now,
                priority: 100,
            })
            .collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Sentinel {
        Sentinel::new("sentinel-1")
    }

    fn replica(addr: &str, offset: u64, priority: u8) -> ReplicaInfo {
        ReplicaInfo {
            addr: addr.into(),
            offset,
            last_seen_ms: 0,
            priority,
        }
    }

    #[test]
    fn monitor_stores_master() {
        let mut s = s();
        s.monitor("mymaster", "10.0.0.1:6379", 2, 5000).unwrap();
        let m = s.get("mymaster").unwrap();
        assert_eq!(m.quorum, 2);
        assert_eq!(m.state, MasterState::Up);
    }

    #[test]
    fn monitor_duplicate_refused() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        assert!(matches!(s.monitor("m", "a", 1, 1000).unwrap_err(), SentinelError::AlreadyMonitored(_)));
    }

    #[test]
    fn tick_flags_subjective_down_after_timeout() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        s.advance_clock(999);
        assert_eq!(s.tick("m").unwrap(), MasterState::Up);
        s.advance_clock(1);
        assert_eq!(s.tick("m").unwrap(), MasterState::SubjectiveDown);
    }

    #[test]
    fn ping_ok_clears_subjective_down() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        s.advance_clock(2000);
        s.tick("m").unwrap();
        assert_eq!(s.get("m").unwrap().state, MasterState::SubjectiveDown);
        s.record_ping_ok("m").unwrap();
        assert_eq!(s.get("m").unwrap().state, MasterState::Up);
    }

    #[test]
    fn quorum_escalates_to_objective_down() {
        let mut s = s();
        s.monitor("m", "a", 3, 1000).unwrap();
        s.advance_clock(2000);
        s.tick("m").unwrap();
        // 2 of 3 votes — still subjective.
        s.record_peer_down_vote("m", "s2").unwrap();
        s.record_peer_down_vote("m", "s3").unwrap();
        assert_eq!(s.get("m").unwrap().state, MasterState::SubjectiveDown);
        // 3rd vote → quorum, escalate.
        let st = s.record_peer_down_vote("m", "s4").unwrap();
        assert_eq!(st, MasterState::ObjectiveDown);
    }

    #[test]
    fn duplicate_peer_votes_dedup() {
        let mut s = s();
        s.monitor("m", "a", 2, 1000).unwrap();
        s.advance_clock(2000);
        s.tick("m").unwrap();
        s.record_peer_down_vote("m", "s2").unwrap();
        s.record_peer_down_vote("m", "s2").unwrap(); // same peer
        assert_eq!(s.get("m").unwrap().state, MasterState::SubjectiveDown);
    }

    #[test]
    fn select_candidate_picks_lowest_priority() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        s.register_replica("m", replica("r1", 100, 50)).unwrap();
        s.register_replica("m", replica("r2", 50, 10)).unwrap();
        s.register_replica("m", replica("r3", 200, 100)).unwrap();
        let c = s.select_promotion_candidate("m").unwrap();
        assert_eq!(c.addr, "r2");
    }

    #[test]
    fn select_candidate_tiebreaks_on_offset_then_addr() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        s.register_replica("m", replica("r1", 100, 50)).unwrap();
        s.register_replica("m", replica("r2", 200, 50)).unwrap();
        s.register_replica("m", replica("r3", 200, 50)).unwrap();
        let c = s.select_promotion_candidate("m").unwrap();
        // r2 + r3 tied on offset 200, lex smallest addr wins.
        assert_eq!(c.addr, "r2");
    }

    #[test]
    fn priority_zero_is_excluded() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        s.register_replica("m", replica("r1", 100, 0)).unwrap();
        s.register_replica("m", replica("r2", 50, 100)).unwrap();
        let c = s.select_promotion_candidate("m").unwrap();
        assert_eq!(c.addr, "r2");
    }

    #[test]
    fn begin_failover_requires_objective_down() {
        let mut s = s();
        s.monitor("m", "old:6379", 1, 1000).unwrap();
        s.register_replica("m", replica("r1", 100, 50)).unwrap();
        // Not yet down → refused.
        let err = s.begin_failover("m").unwrap_err();
        assert!(matches!(err, SentinelError::QuorumUnreached { .. }));
    }

    #[test]
    fn full_failover_lifecycle() {
        let mut s = s();
        s.monitor("m", "old:6379", 2, 1000).unwrap();
        s.register_replica("m", replica("r1:6379", 100, 50)).unwrap();
        s.register_replica("m", replica("r2:6379", 200, 10)).unwrap();
        s.advance_clock(2000);
        s.tick("m").unwrap();
        s.record_peer_down_vote("m", "p1").unwrap();
        s.record_peer_down_vote("m", "p2").unwrap();
        let plan = s.begin_failover("m").unwrap();
        assert_eq!(plan.promoted_addr, "r2:6379");
        // Old master becomes a replica.
        assert!(plan.new_replica_addrs.contains(&"old:6379".to_string()));
        assert!(plan.new_replica_addrs.contains(&"r1:6379".to_string()));
        assert_eq!(plan.new_config_epoch, 1);
        s.commit_failover(&plan).unwrap();
        let m = s.get("m").unwrap();
        assert_eq!(m.addr, "r2:6379");
        assert_eq!(m.state, MasterState::Up);
        assert_eq!(m.replicas.len(), 2);
    }

    #[test]
    fn no_eligible_replica_errors() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        s.register_replica("m", replica("r1", 100, 0)).unwrap();
        assert!(matches!(s.select_promotion_candidate("m").unwrap_err(), SentinelError::NoEligibleReplica));
    }

    #[test]
    fn remove_drops_master() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        s.remove("m").unwrap();
        assert!(matches!(s.remove("m").unwrap_err(), SentinelError::UnknownMaster(_)));
    }

    #[test]
    fn forget_replica_removes_entry() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        s.register_replica("m", replica("r1", 100, 50)).unwrap();
        s.register_replica("m", replica("r2", 50, 50)).unwrap();
        s.forget_replica("m", "r1").unwrap();
        assert_eq!(s.get("m").unwrap().replicas.len(), 1);
    }

    #[test]
    fn register_replica_replaces_existing_addr() {
        let mut s = s();
        s.monitor("m", "a", 1, 1000).unwrap();
        s.register_replica("m", replica("r1", 100, 50)).unwrap();
        s.register_replica("m", replica("r1", 200, 50)).unwrap();
        let m = s.get("m").unwrap();
        assert_eq!(m.replicas.len(), 1);
        assert_eq!(m.replicas[0].offset, 200);
    }
}
