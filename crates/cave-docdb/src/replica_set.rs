//! Replica-set topology + monitoring.
//!
//! Tracks the canonical MongoDB replica set states documented at
//! <https://www.mongodb.com/docs/manual/reference/replica-states/>:
//!
//! | id | state    | description                              |
//! |----|----------|------------------------------------------|
//! | 0  | STARTUP  | Initial state, joining the set           |
//! | 1  | PRIMARY  | Elected primary, accepts writes          |
//! | 2  | SECONDARY| Replicating from primary                 |
//! | 3  | RECOVERING| Performing startup self-checks          |
//! | 5  | STARTUP2 | Performing initial sync                  |
//! | 6  | UNKNOWN  | Last-known state                         |
//! | 7  | ARBITER  | Vote-only, no data                       |
//! | 8  | DOWN     | Not reachable                            |
//! | 9  | ROLLBACK | Rolling back local writes                |
//! | 10 | REMOVED  | Removed from configuration               |

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberState {
    Startup = 0,
    Primary = 1,
    Secondary = 2,
    Recovering = 3,
    Startup2 = 5,
    Unknown = 6,
    Arbiter = 7,
    Down = 8,
    Rollback = 9,
    Removed = 10,
}

impl MemberState {
    pub fn from_i32(v: i32) -> Option<Self> {
        Some(match v {
            0 => Self::Startup,
            1 => Self::Primary,
            2 => Self::Secondary,
            3 => Self::Recovering,
            5 => Self::Startup2,
            6 => Self::Unknown,
            7 => Self::Arbiter,
            8 => Self::Down,
            9 => Self::Rollback,
            10 => Self::Removed,
            _ => return None,
        })
    }
    pub fn is_writable(self) -> bool {
        matches!(self, Self::Primary)
    }
    pub fn is_readable(self) -> bool {
        matches!(self, Self::Primary | Self::Secondary)
    }
}

#[derive(Debug, Clone)]
pub struct Member {
    pub id: i32,
    pub host: String,
    pub state: MemberState,
    pub priority: f64,
    pub votes: i32,
    pub hidden: bool,
    pub arbiter_only: bool,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub uptime_secs: u64,
    pub optime_ts: u64,
}

impl Member {
    pub fn new(id: i32, host: &str) -> Self {
        Self {
            id,
            host: host.into(),
            state: MemberState::Secondary,
            priority: 1.0,
            votes: 1,
            hidden: false,
            arbiter_only: false,
            last_heartbeat: None,
            uptime_secs: 0,
            optime_ts: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplicaSet {
    pub name: String,
    pub members: BTreeMap<i32, Member>,
    pub election_term: i64,
    /// Replica-set protocol version (always 1 for modern MongoDB).
    pub protocol_version: i32,
    /// Configured maximum lag tolerated for secondary reads.
    pub max_staleness_secs: i64,
}

impl ReplicaSet {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            members: BTreeMap::new(),
            election_term: 0,
            protocol_version: 1,
            max_staleness_secs: 90,
        }
    }

    pub fn add_member(&mut self, m: Member) {
        self.members.insert(m.id, m);
    }

    pub fn remove_member(&mut self, id: i32) -> bool {
        self.members.remove(&id).is_some()
    }

    pub fn primary(&self) -> Option<&Member> {
        self.members.values().find(|m| m.state == MemberState::Primary)
    }

    pub fn secondaries(&self) -> Vec<&Member> {
        self.members
            .values()
            .filter(|m| m.state == MemberState::Secondary && !m.hidden)
            .collect()
    }

    pub fn arbiters(&self) -> Vec<&Member> {
        self.members
            .values()
            .filter(|m| m.state == MemberState::Arbiter)
            .collect()
    }

    /// Run a single round of leader election. Highest priority among Secondary
    /// members (with vote count > 0) becomes Primary; ties broken by member id.
    pub fn elect_primary(&mut self) -> Option<i32> {
        let candidates: Vec<i32> = self
            .members
            .values()
            .filter(|m| {
                matches!(
                    m.state,
                    MemberState::Secondary | MemberState::Primary
                ) && !m.arbiter_only
                    && !m.hidden
                    && m.votes > 0
            })
            .map(|m| m.id)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        // Count votes available for the election: candidate votes + arbiter
        // votes (arbiters cast votes but never become primary themselves).
        let voting_arbiters = self
            .members
            .values()
            .filter(|m| m.state == MemberState::Arbiter && m.votes > 0)
            .count();
        let majority = (self.voting_members() / 2) + 1;
        if candidates.len() + voting_arbiters < majority {
            return None;
        }
        // Reset all to Secondary first.
        for m in self.members.values_mut() {
            if m.state == MemberState::Primary {
                m.state = MemberState::Secondary;
            }
        }
        let winner_id = *candidates
            .iter()
            .max_by(|a, b| {
                let pa = self.members[a].priority;
                let pb = self.members[b].priority;
                pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal).then_with(|| b.cmp(a))
            })?;
        let winner = self.members.get_mut(&winner_id)?;
        winner.state = MemberState::Primary;
        self.election_term += 1;
        Some(winner_id)
    }

    pub fn voting_members(&self) -> usize {
        self.members
            .values()
            .filter(|m| m.votes > 0 && !m.hidden)
            .count()
    }

    pub fn step_down(&mut self) -> bool {
        if let Some(p) = self.members.values_mut().find(|m| m.state == MemberState::Primary) {
            p.state = MemberState::Secondary;
            true
        } else {
            false
        }
    }

    /// Compute replication lag (seconds) for each secondary against the primary.
    pub fn replication_lag(&self) -> BTreeMap<i32, u64> {
        let primary_optime = match self.primary() {
            Some(p) => p.optime_ts,
            None => return BTreeMap::new(),
        };
        self.secondaries()
            .iter()
            .map(|m| (m.id, primary_optime.saturating_sub(m.optime_ts)))
            .collect()
    }

    pub fn is_quorum_alive(&self) -> bool {
        let alive = self
            .members
            .values()
            .filter(|m| m.state != MemberState::Down && m.votes > 0)
            .count();
        alive >= self.voting_members() / 2 + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: i32, priority: f64) -> Member {
        let mut m = Member::new(id, &format!("host{}", id));
        m.priority = priority;
        m
    }

    #[test]
    fn member_state_from_i32_valid() {
        assert_eq!(MemberState::from_i32(1), Some(MemberState::Primary));
        assert_eq!(MemberState::from_i32(2), Some(MemberState::Secondary));
        assert_eq!(MemberState::from_i32(7), Some(MemberState::Arbiter));
    }
    #[test]
    fn member_state_from_i32_invalid() {
        assert_eq!(MemberState::from_i32(99), None);
    }
    #[test]
    fn primary_is_writable() {
        assert!(MemberState::Primary.is_writable());
        assert!(!MemberState::Secondary.is_writable());
    }
    #[test]
    fn secondary_is_readable_but_not_writable() {
        assert!(MemberState::Secondary.is_readable());
        assert!(!MemberState::Secondary.is_writable());
    }
    #[test]
    fn arbiter_is_neither_readable_nor_writable() {
        assert!(!MemberState::Arbiter.is_readable());
        assert!(!MemberState::Arbiter.is_writable());
    }

    #[test]
    fn add_then_remove_member() {
        let mut rs = ReplicaSet::new("rs0");
        rs.add_member(member(0, 1.0));
        assert!(rs.remove_member(0));
        assert!(!rs.remove_member(0));
    }

    #[test]
    fn election_picks_highest_priority() {
        let mut rs = ReplicaSet::new("rs0");
        rs.add_member(member(0, 1.0));
        rs.add_member(member(1, 5.0));
        rs.add_member(member(2, 1.0));
        let winner = rs.elect_primary().unwrap();
        assert_eq!(winner, 1);
        assert_eq!(rs.primary().unwrap().id, 1);
    }

    #[test]
    fn election_requires_majority() {
        let mut rs = ReplicaSet::new("rs0");
        rs.add_member(member(0, 1.0));
        rs.add_member(member(1, 1.0));
        rs.add_member(member(2, 1.0));
        // Mark two as Down → only one alive vote, no majority.
        rs.members.get_mut(&1).unwrap().state = MemberState::Down;
        rs.members.get_mut(&2).unwrap().state = MemberState::Down;
        // voting_members counts m.votes > 0; Down members still have votes.
        // But candidates list filters by Secondary|Primary, so only id 0 is a candidate.
        // Majority of 3 = 2; only 1 candidate → no election.
        let winner = rs.elect_primary();
        assert_eq!(winner, None);
    }

    #[test]
    fn election_increments_term() {
        let mut rs = ReplicaSet::new("rs0");
        rs.add_member(member(0, 1.0));
        let initial_term = rs.election_term;
        rs.elect_primary().unwrap();
        assert_eq!(rs.election_term, initial_term + 1);
    }

    #[test]
    fn step_down_demotes_primary() {
        let mut rs = ReplicaSet::new("rs0");
        rs.add_member(member(0, 1.0));
        rs.elect_primary().unwrap();
        assert!(rs.step_down());
        assert!(rs.primary().is_none());
    }

    #[test]
    fn step_down_no_primary_returns_false() {
        let mut rs = ReplicaSet::new("rs0");
        rs.add_member(member(0, 1.0));
        assert!(!rs.step_down());
    }

    #[test]
    fn arbiter_does_not_become_primary() {
        let mut rs = ReplicaSet::new("rs0");
        rs.add_member(member(0, 1.0));
        let mut arb = member(1, 10.0);
        arb.arbiter_only = true;
        arb.state = MemberState::Arbiter;
        rs.add_member(arb);
        let winner = rs.elect_primary().unwrap();
        assert_eq!(winner, 0);
    }

    #[test]
    fn hidden_member_not_in_secondaries_list() {
        let mut rs = ReplicaSet::new("rs0");
        rs.add_member(member(0, 1.0));
        let mut hidden = member(1, 1.0);
        hidden.hidden = true;
        rs.add_member(hidden);
        assert_eq!(rs.secondaries().len(), 1);
    }

    #[test]
    fn replication_lag_zero_when_in_sync() {
        let mut rs = ReplicaSet::new("rs0");
        let mut p = member(0, 1.0);
        p.state = MemberState::Primary;
        p.optime_ts = 100;
        let mut s = member(1, 1.0);
        s.optime_ts = 100;
        rs.add_member(p);
        rs.add_member(s);
        let lag = rs.replication_lag();
        assert_eq!(lag.get(&1), Some(&0));
    }

    #[test]
    fn replication_lag_diff_seconds() {
        let mut rs = ReplicaSet::new("rs0");
        let mut p = member(0, 1.0);
        p.state = MemberState::Primary;
        p.optime_ts = 1000;
        let mut s = member(1, 1.0);
        s.optime_ts = 950;
        rs.add_member(p);
        rs.add_member(s);
        assert_eq!(rs.replication_lag().get(&1), Some(&50));
    }

    #[test]
    fn quorum_alive_with_majority_up() {
        let mut rs = ReplicaSet::new("rs0");
        for i in 0..3 {
            rs.add_member(member(i, 1.0));
        }
        assert!(rs.is_quorum_alive());
        rs.members.get_mut(&0).unwrap().state = MemberState::Down;
        assert!(rs.is_quorum_alive()); // 2/3 alive
        rs.members.get_mut(&1).unwrap().state = MemberState::Down;
        assert!(!rs.is_quorum_alive()); // only 1/3
    }

    #[test]
    fn voting_members_excludes_zero_vote() {
        let mut rs = ReplicaSet::new("rs0");
        let a = member(0, 1.0);
        let mut b = member(1, 1.0);
        b.votes = 0;
        rs.add_member(a);
        rs.add_member(b);
        assert_eq!(rs.voting_members(), 1);
    }
}

