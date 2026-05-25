// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Maintenance API extensions — full `StatusResponse`, `AlarmList`, and
//! revision-bounded `Hash`.
//!
//! Mirrors etcd v3.6.10
//!   `api/etcdserverpb/rpc.proto` (StatusResponse — adds dbSizeInUse,
//!     isLearner, raftAppliedIndex, errors[]),
//!   `server/etcdserver/api/v3rpc/maintenance.go#Status`,
//!   `server/etcdserver/api/v3rpc/maintenance.go#Alarm` (AlarmList path).

use serde::{Deserialize, Serialize};
use std::sync::RwLock;

use crate::cluster_status::MemberStatus;

// ── Full StatusResponse ──────────────────────────────────────────────────

/// Full `StatusResponse` per etcd v3.6 rpc.proto.  Returned by
/// `Maintenance.Status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullStatus {
    pub version: String,
    pub db_size: u64,
    pub db_size_in_use: u64,
    pub leader: u64,
    pub raft_index: u64,
    pub raft_applied_index: u64,
    pub raft_term: u64,
    pub member_id: u64,
    pub cluster_id: u64,
    pub revision: u64,
    pub is_learner: bool,
    /// Free-form server-side errors / warnings (e.g. "alarm: NOSPACE").
    pub errors: Vec<String>,
    /// Storage-version reported during downgrade.
    pub storage_version: String,
}

impl FullStatus {
    pub fn from_member(
        m: &MemberStatus,
        cluster_id: u64,
        raft_applied_index: u64,
        errors: Vec<String>,
    ) -> Self {
        Self {
            version: m.version.clone(),
            db_size: m.db_size,
            db_size_in_use: m.db_size_in_use,
            leader: m.leader,
            raft_index: m.revision, // revision approximates raft index for in-process store
            raft_applied_index,
            raft_term: m.raft_term,
            member_id: m.member_id,
            cluster_id,
            revision: m.revision,
            is_learner: m.is_learner,
            errors,
            storage_version: m.version.clone(),
        }
    }

    /// True if any of the encoded `errors[]` entries is an active alarm.
    pub fn has_alarm(&self) -> bool {
        self.errors.iter().any(|e| e.starts_with("alarm:"))
    }

    /// True if the member is the cluster leader.
    pub fn is_leader(&self) -> bool {
        self.leader == self.member_id
    }
}

// ── AlarmList ─────────────────────────────────────────────────────────────

/// Alarm types — etcd v3 ships NoSpace and Corrupt; we keep both plus a
/// `None` variant for "no active alarms".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum AlarmType {
    None,
    NoSpace,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct AlarmEntry {
    pub member_id: u64,
    pub alarm: AlarmType,
}

/// Active-alarm registry.
pub struct AlarmRegistry {
    inner: RwLock<Vec<AlarmEntry>>,
}

impl AlarmRegistry {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Vec::new()),
        }
    }

    /// Activate an alarm.  Idempotent — re-activating an existing alarm
    /// is a no-op.  Returns `true` when the alarm is newly added.
    pub fn activate(&self, e: AlarmEntry) -> bool {
        let mut g = self.inner.write().unwrap();
        if g.iter().any(|x| x == &e) {
            return false;
        }
        g.push(e);
        true
    }

    /// Deactivate an alarm.  Returns `true` if it existed.
    pub fn deactivate(&self, e: &AlarmEntry) -> bool {
        let mut g = self.inner.write().unwrap();
        let len_before = g.len();
        g.retain(|x| x != e);
        g.len() != len_before
    }

    /// All currently-active alarms.
    pub fn list(&self) -> Vec<AlarmEntry> {
        self.inner.read().unwrap().clone()
    }

    /// Alarms scoped to a single member.
    pub fn list_for(&self, member_id: u64) -> Vec<AlarmEntry> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .filter(|e| e.member_id == member_id)
            .cloned()
            .collect()
    }

    /// Whether a specific alarm is active for a member.
    pub fn is_active(&self, member_id: u64, alarm: AlarmType) -> bool {
        self.inner
            .read()
            .unwrap()
            .iter()
            .any(|e| e.member_id == member_id && e.alarm == alarm)
    }

    /// Wipe everything — used by tests.
    pub fn clear(&self) {
        self.inner.write().unwrap().clear();
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for AlarmRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Revision-bounded hash ─────────────────────────────────────────────────

/// `HashKVRequest` extended with `[revision_lower, revision_upper]`.
/// Useful for "check that two members agreed on the data between rev X
/// and rev Y" without hashing the whole DB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashRangeRequest {
    pub revision_lower: u64,
    pub revision_upper: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashRangeResponse {
    pub revision_lower: u64,
    pub revision_upper: u64,
    pub hash: u64,
    /// Number of revisions folded into the hash.
    pub revisions_hashed: u64,
}

/// One revision entry — `(revision, key, value)`.
#[derive(Debug, Clone)]
pub struct RevisionRecord<'a> {
    pub revision: u64,
    pub key: &'a [u8],
    pub value: &'a [u8],
}

/// Compute a deterministic hash over a slice of revision records bounded
/// by `[lower, upper]`.  Sort-stable and order-independent up to revision
/// number.
pub fn hash_range(records: &[RevisionRecord<'_>], req: &HashRangeRequest) -> HashRangeResponse {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut count = 0u64;
    let mut filtered: Vec<&RevisionRecord<'_>> = records
        .iter()
        .filter(|r| r.revision >= req.revision_lower && r.revision <= req.revision_upper)
        .collect();
    filtered.sort_by_key(|r| (r.revision, r.key));
    for r in &filtered {
        h = h.wrapping_mul(0x100000001b3).wrapping_add(r.revision);
        for &b in r.key.iter().chain(r.value.iter()) {
            h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
        }
        count += 1;
    }
    HashRangeResponse {
        revision_lower: req.revision_lower,
        revision_upper: req.revision_upper,
        hash: h,
        revisions_hashed: count,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M15
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster_status::MemberHealth;

    fn member_status(id: u64, leader: u64, db: u64, rev: u64, learner: bool) -> MemberStatus {
        MemberStatus {
            member_id: id,
            name: format!("m{id}"),
            revision: rev,
            db_size: db,
            db_size_in_use: db.saturating_sub(10),
            leader,
            raft_term: 7,
            is_learner: learner,
            health: MemberHealth::Healthy,
            last_heartbeat_age_secs: Some(0),
            version: "3.6.10".into(),
        }
    }

    // ── FullStatus ────────────────────────────────────────────────────

    #[test]
    fn test_status_from_member_populates_fields() {
        // cite: rpc.proto StatusResponse
        let m = member_status(7, 7, 1024, 100, false);
        let s = FullStatus::from_member(&m, 0xCAFE, 99, vec![]);
        assert_eq!(s.member_id, 7);
        assert_eq!(s.cluster_id, 0xCAFE);
        assert_eq!(s.db_size, 1024);
        assert_eq!(s.db_size_in_use, 1014);
        assert_eq!(s.leader, 7);
        assert_eq!(s.raft_term, 7);
        assert_eq!(s.raft_index, 100);
        assert_eq!(s.raft_applied_index, 99);
        assert_eq!(s.revision, 100);
        assert_eq!(s.version, "3.6.10");
    }

    #[test]
    fn test_status_is_leader_true_when_self() {
        // cite: maintenance.go (leader id == self ⇒ leader)
        let m = member_status(7, 7, 0, 0, false);
        let s = FullStatus::from_member(&m, 1, 0, vec![]);
        assert!(s.is_leader());
    }

    #[test]
    fn test_status_is_leader_false_when_other() {
        let m = member_status(7, 99, 0, 0, false);
        let s = FullStatus::from_member(&m, 1, 0, vec![]);
        assert!(!s.is_leader());
    }

    #[test]
    fn test_status_has_alarm_with_alarm_prefix() {
        // cite: maintenance.go (errors[] carries 'alarm:' prefix)
        let m = member_status(1, 1, 0, 0, false);
        let s = FullStatus::from_member(&m, 1, 0, vec!["alarm: NOSPACE".into()]);
        assert!(s.has_alarm());
    }

    #[test]
    fn test_status_has_alarm_false_when_clean() {
        let m = member_status(1, 1, 0, 0, false);
        let s = FullStatus::from_member(&m, 1, 0, vec![]);
        assert!(!s.has_alarm());
    }

    #[test]
    fn test_status_records_learner_flag() {
        // cite: rpc.proto isLearner
        let m = member_status(7, 1, 0, 0, true);
        let s = FullStatus::from_member(&m, 1, 0, vec![]);
        assert!(s.is_learner);
    }

    #[test]
    fn test_status_storage_version_present() {
        // cite: rpc.proto storageVersion
        let m = member_status(1, 1, 0, 0, false);
        let s = FullStatus::from_member(&m, 1, 0, vec![]);
        assert_eq!(s.storage_version, "3.6.10");
    }

    // ── AlarmRegistry ─────────────────────────────────────────────────

    #[test]
    fn test_alarm_activate_returns_true_for_new() {
        // cite: maintenance.go AlarmActivate
        let r = AlarmRegistry::new();
        assert!(r.activate(AlarmEntry {
            member_id: 1,
            alarm: AlarmType::NoSpace
        }));
    }

    #[test]
    fn test_alarm_activate_idempotent() {
        // cite: maintenance.go (re-activate is no-op)
        let r = AlarmRegistry::new();
        r.activate(AlarmEntry {
            member_id: 1,
            alarm: AlarmType::NoSpace,
        });
        assert!(!r.activate(AlarmEntry {
            member_id: 1,
            alarm: AlarmType::NoSpace
        }));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn test_alarm_deactivate_returns_true_when_present() {
        // cite: maintenance.go AlarmDeactivate
        let r = AlarmRegistry::new();
        let e = AlarmEntry {
            member_id: 1,
            alarm: AlarmType::Corrupt,
        };
        r.activate(e.clone());
        assert!(r.deactivate(&e));
        assert!(r.is_empty());
    }

    #[test]
    fn test_alarm_deactivate_returns_false_when_absent() {
        let r = AlarmRegistry::new();
        let e = AlarmEntry {
            member_id: 1,
            alarm: AlarmType::Corrupt,
        };
        assert!(!r.deactivate(&e));
    }

    #[test]
    fn test_alarm_list_returns_all() {
        // cite: maintenance.go AlarmList
        let r = AlarmRegistry::new();
        r.activate(AlarmEntry {
            member_id: 1,
            alarm: AlarmType::NoSpace,
        });
        r.activate(AlarmEntry {
            member_id: 2,
            alarm: AlarmType::Corrupt,
        });
        assert_eq!(r.list().len(), 2);
    }

    #[test]
    fn test_alarm_list_for_member_filters() {
        // cite: maintenance.go (per-member filter)
        let r = AlarmRegistry::new();
        r.activate(AlarmEntry {
            member_id: 1,
            alarm: AlarmType::NoSpace,
        });
        r.activate(AlarmEntry {
            member_id: 2,
            alarm: AlarmType::Corrupt,
        });
        let m1 = r.list_for(1);
        assert_eq!(m1.len(), 1);
        assert_eq!(m1[0].alarm, AlarmType::NoSpace);
    }

    #[test]
    fn test_alarm_is_active_check() {
        // cite: maintenance.go (alarm-state lookup)
        let r = AlarmRegistry::new();
        r.activate(AlarmEntry {
            member_id: 1,
            alarm: AlarmType::NoSpace,
        });
        assert!(r.is_active(1, AlarmType::NoSpace));
        assert!(!r.is_active(1, AlarmType::Corrupt));
        assert!(!r.is_active(2, AlarmType::NoSpace));
    }

    #[test]
    fn test_alarm_clear_removes_all() {
        // cite: tests/maintenance — reset between cases
        let r = AlarmRegistry::new();
        r.activate(AlarmEntry {
            member_id: 1,
            alarm: AlarmType::NoSpace,
        });
        r.activate(AlarmEntry {
            member_id: 2,
            alarm: AlarmType::Corrupt,
        });
        r.clear();
        assert!(r.is_empty());
    }

    #[test]
    fn test_alarm_distinct_types_for_same_member_coexist() {
        // cite: maintenance.go (NOSPACE + CORRUPT can coexist)
        let r = AlarmRegistry::new();
        r.activate(AlarmEntry {
            member_id: 1,
            alarm: AlarmType::NoSpace,
        });
        r.activate(AlarmEntry {
            member_id: 1,
            alarm: AlarmType::Corrupt,
        });
        assert_eq!(r.list_for(1).len(), 2);
    }

    // ── Revision-bounded hash ─────────────────────────────────────────

    #[test]
    fn test_hash_range_filters_by_revision() {
        // cite: maintenance.go HashKV (revision bound)
        let recs = vec![
            RevisionRecord {
                revision: 1,
                key: b"a",
                value: b"1",
            },
            RevisionRecord {
                revision: 5,
                key: b"b",
                value: b"2",
            },
            RevisionRecord {
                revision: 10,
                key: b"c",
                value: b"3",
            },
        ];
        let r1 = hash_range(
            &recs,
            &HashRangeRequest {
                revision_lower: 1,
                revision_upper: 5,
            },
        );
        assert_eq!(r1.revisions_hashed, 2);
    }

    #[test]
    fn test_hash_range_inclusive_upper() {
        // cite: HashKV (inclusive upper bound)
        let recs = vec![RevisionRecord {
            revision: 5,
            key: b"a",
            value: b"1",
        }];
        let r = hash_range(
            &recs,
            &HashRangeRequest {
                revision_lower: 5,
                revision_upper: 5,
            },
        );
        assert_eq!(r.revisions_hashed, 1);
    }

    #[test]
    fn test_hash_range_deterministic() {
        // cite: HashKV stable ordering
        let recs = vec![
            RevisionRecord {
                revision: 1,
                key: b"a",
                value: b"1",
            },
            RevisionRecord {
                revision: 2,
                key: b"b",
                value: b"2",
            },
        ];
        let r1 = hash_range(
            &recs,
            &HashRangeRequest {
                revision_lower: 1,
                revision_upper: 100,
            },
        );
        let r2 = hash_range(
            &recs,
            &HashRangeRequest {
                revision_lower: 1,
                revision_upper: 100,
            },
        );
        assert_eq!(r1.hash, r2.hash);
    }

    #[test]
    fn test_hash_range_order_independent() {
        // cite: HashKV (sort by (rev,key) ⇒ order-independent)
        let recs = vec![
            RevisionRecord {
                revision: 2,
                key: b"b",
                value: b"2",
            },
            RevisionRecord {
                revision: 1,
                key: b"a",
                value: b"1",
            },
        ];
        let recs_swapped = vec![
            RevisionRecord {
                revision: 1,
                key: b"a",
                value: b"1",
            },
            RevisionRecord {
                revision: 2,
                key: b"b",
                value: b"2",
            },
        ];
        let h1 = hash_range(
            &recs,
            &HashRangeRequest {
                revision_lower: 0,
                revision_upper: 100,
            },
        )
        .hash;
        let h2 = hash_range(
            &recs_swapped,
            &HashRangeRequest {
                revision_lower: 0,
                revision_upper: 100,
            },
        )
        .hash;
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_range_empty_window() {
        // cite: HashKV (no records in window)
        let recs = vec![RevisionRecord {
            revision: 1,
            key: b"a",
            value: b"1",
        }];
        let r = hash_range(
            &recs,
            &HashRangeRequest {
                revision_lower: 5,
                revision_upper: 10,
            },
        );
        assert_eq!(r.revisions_hashed, 0);
    }

    #[test]
    fn test_hash_range_changes_on_value_diff() {
        // cite: HashKV (different values ⇒ different hashes)
        let r1 = hash_range(
            &[RevisionRecord {
                revision: 1,
                key: b"a",
                value: b"v1",
            }],
            &HashRangeRequest {
                revision_lower: 1,
                revision_upper: 1,
            },
        );
        let r2 = hash_range(
            &[RevisionRecord {
                revision: 1,
                key: b"a",
                value: b"v2",
            }],
            &HashRangeRequest {
                revision_lower: 1,
                revision_upper: 1,
            },
        );
        assert_ne!(r1.hash, r2.hash);
    }

    #[test]
    fn test_hash_range_records_window() {
        let recs = vec![RevisionRecord {
            revision: 7,
            key: b"a",
            value: b"v",
        }];
        let r = hash_range(
            &recs,
            &HashRangeRequest {
                revision_lower: 5,
                revision_upper: 9,
            },
        );
        assert_eq!(r.revision_lower, 5);
        assert_eq!(r.revision_upper, 9);
    }
}
