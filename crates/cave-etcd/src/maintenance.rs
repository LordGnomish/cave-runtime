//! Maintenance-API extensions.  The base store already exposes
//! `defragment`, `alarm`, `status`, `hash`, and `snapshot`.  This module
//! adds the two endpoints etcd v3.6 ships that were missing: `HashKV`
//! (hash filtered by revision range) and `MoveLeader`.
//!
//! Mirrors etcd v3.6.10
//!   `api/etcdserverpb/rpc.proto` (`HashKVRequest`, `MoveLeaderRequest`)
//!   `server/etcdserver/api/v3rpc/maintenance.go` (`HashKV`, `MoveLeader`).

use crate::error::{EtcdError, EtcdResult};
use crate::models::{Member, RaftRole, ResponseHeader};
use crate::store::KvStore;
use serde::{Deserialize, Serialize};

// ── HashKV ────────────────────────────────────────────────────────────────

/// `HashKVRequest` — bound hash to history at a specific revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashKvRequest {
    /// `0` ⇒ hash the current revision; non-zero ⇒ hash up to and
    /// including this revision.
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashKvResponse {
    pub header: ResponseHeader,
    pub hash: u32,
    pub compact_revision: u64,
    /// Echoes back the requested revision (or the current revision when
    /// `revision == 0`).
    pub hash_revision: u64,
}

/// Compute a deterministic 32-bit hash of every `(key, value)` pair
/// whose `mod_revision <= revision`.  Mirrors
/// `server/storage/mvcc/hash.go#hashByRev`.
pub fn hash_kv(store: &KvStore, req: &HashKvRequest) -> EtcdResult<HashKvResponse> {
    let compact = store.compaction_revision();
    let target = if req.revision == 0 {
        store.current_revision()
    } else {
        req.revision
    };
    if compact > 0 && target < compact {
        return Err(EtcdError::RevisionCompacted {
            requested: target,
            compacted: compact,
        });
    }
    // Build a deterministic ordering: read all KVs at `target`, sort by
    // key, fold into a djb2-style hash.
    let resp = store.range(&crate::models::RangeRequest {
        key: "".into(),
        range_end: Some("\u{ffff}".into()), // anything below U+FFFF
        limit: None,
        revision: if req.revision == 0 { None } else { Some(target) },
        keys_only: false,
        count_only: false,
    })?;
    let mut kvs = resp.kvs;
    kvs.sort_by(|a, b| a.key.cmp(&b.key));
    let mut h: u32 = 5381;
    for kv in &kvs {
        for &b in kv.key.iter().chain(kv.value.iter()) {
            h = h.wrapping_mul(33).wrapping_add(b as u32);
        }
    }
    Ok(HashKvResponse {
        header: ResponseHeader {
            cluster_id: 1,
            member_id: 1,
            revision: store.current_revision(),
            raft_term: store.current_term(),
        },
        hash: h,
        compact_revision: compact,
        hash_revision: target,
    })
}

// ── MoveLeader ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveLeaderRequest {
    /// Member-id of the *target* (must currently be a non-learner voter
    /// other than the current leader).
    pub target_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveLeaderResponse {
    pub header: ResponseHeader,
    /// `from`/`to` member ids reflecting the transition.
    pub from: u64,
    pub to: u64,
}

/// Transfer leadership to `req.target_id`.  In the in-process store this
/// flips the local node to `Follower` and records the transition; in
/// production the caller would forward the equivalent
/// `LeaderTransferRequest` to the Raft module.
pub fn move_leader(
    store: &KvStore,
    req: &MoveLeaderRequest,
) -> EtcdResult<MoveLeaderResponse> {
    if !matches!(store.raft_role(), RaftRole::Leader) {
        return Err(EtcdError::NotLeader {
            term: store.current_term(),
            leader: None,
        });
    }
    let members = store.member_list().members;
    let target: &Member = members
        .iter()
        .find(|m| m.id == req.target_id)
        .ok_or(EtcdError::MemberNotFound(req.target_id))?;
    if target.is_learner {
        return Err(EtcdError::MemberNotLearner(req.target_id));
    }
    let from = store.local_member_id();
    if req.target_id == from {
        return Err(EtcdError::Internal(format!(
            "cannot transfer leadership to self (id={from})"
        )));
    }
    store.set_raft_role(RaftRole::Follower);
    Ok(MoveLeaderResponse {
        header: ResponseHeader {
            cluster_id: 1,
            member_id: from,
            revision: store.current_revision(),
            raft_term: store.current_term(),
        },
        from,
        to: req.target_id,
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Maintenance-API tests — feat/cave-etcd-deeper-003
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MemberAddRequest, PutRequest};

    fn dt(tenant_id: &str, suffix: &str) -> String {
        format!("/tenants/{}/{}", tenant_id, suffix)
    }

    fn pk_put(store: &KvStore, key: &str, value: &str) -> u64 {
        store.put(&PutRequest {
            key: key.into(),
            value: value.into(),
            lease: None,
            prev_kv: false,
        }).header.revision
    }

    // ── HashKV ──────────────────────────────────────────────────────

    #[test]
    fn test_hash_kv_zero_revision_uses_current() {
        // cite: etcd v3.6.10 HashKVRequest.revision=0 → current
        let tenant_id = "mt-001";
        let store = KvStore::new();
        pk_put(&store, &dt(tenant_id, "k"), "v");
        let r = hash_kv(&store, &HashKvRequest { revision: 0 }).unwrap();
        assert_eq!(r.hash_revision, store.current_revision());
        assert!(r.hash != 0);
    }

    #[test]
    fn test_hash_kv_specific_revision() {
        // cite: etcd v3.6.10 HashKVRequest.revision (bound at rev)
        let tenant_id = "mt-002";
        let store = KvStore::new();
        pk_put(&store, &dt(tenant_id, "k"), "v1");
        let rev_after_v1 = store.current_revision();
        pk_put(&store, &dt(tenant_id, "k"), "v2");
        let r = hash_kv(&store, &HashKvRequest { revision: rev_after_v1 }).unwrap();
        assert_eq!(r.hash_revision, rev_after_v1);
    }

    #[test]
    fn test_hash_kv_compacted_revision_errors() {
        // cite: etcd v3.6.10 HashKV returns ErrCompacted
        let tenant_id = "mt-003";
        let store = KvStore::new();
        pk_put(&store, &dt(tenant_id, "k"), "v1");
        let old = store.current_revision();
        pk_put(&store, &dt(tenant_id, "k"), "v2");
        store.compact_v2(store.current_revision()).unwrap();
        let err = hash_kv(&store, &HashKvRequest { revision: old });
        assert!(matches!(err, Err(EtcdError::RevisionCompacted { .. })));
    }

    #[test]
    fn test_hash_kv_deterministic() {
        // cite: etcd v3.6.10 hash is deterministic per (state, rev)
        let tenant_id = "mt-004";
        let store = KvStore::new();
        for i in 0..5 {
            pk_put(&store, &dt(tenant_id, &format!("k{i}")), &format!("v{i}"));
        }
        let h1 = hash_kv(&store, &HashKvRequest { revision: 0 }).unwrap().hash;
        let h2 = hash_kv(&store, &HashKvRequest { revision: 0 }).unwrap().hash;
        assert_eq!(h1, h2);
    }

    // ── MoveLeader ──────────────────────────────────────────────────

    #[test]
    fn test_move_leader_to_voter_succeeds() {
        // cite: etcd v3.6.10 server/.../v3rpc/maintenance.go MoveLeader
        let tenant_id = "mt-005";
        let store = KvStore::new();
        pk_put(&store, &dt(tenant_id, "k"), "v");
        store.set_raft_role(RaftRole::Leader);
        let target = store
            .member_add(&MemberAddRequest {
                peer_ur_ls: vec!["http://m2:2380".into()],
                is_learner: false,
            })
            .member
            .id;
        let resp = move_leader(&store, &MoveLeaderRequest { target_id: target }).unwrap();
        assert_eq!(resp.to, target);
        assert_eq!(store.raft_role(), RaftRole::Follower);
    }

    #[test]
    fn test_move_leader_rejects_when_not_leader() {
        // cite: etcd v3.6.10 ErrNotLeader
        let _tenant_id = "mt-006";
        let store = KvStore::new();
        store.set_raft_role(RaftRole::Follower);
        let err = move_leader(&store, &MoveLeaderRequest { target_id: 1 });
        assert!(matches!(err, Err(EtcdError::NotLeader { .. })));
    }

    #[test]
    fn test_move_leader_rejects_unknown_target() {
        // cite: etcd v3.6.10 ErrIDNotFound
        let _tenant_id = "mt-007";
        let store = KvStore::new();
        store.set_raft_role(RaftRole::Leader);
        let err = move_leader(&store, &MoveLeaderRequest { target_id: 9_999 });
        assert!(matches!(err, Err(EtcdError::MemberNotFound(_))));
    }

    #[test]
    fn test_move_leader_rejects_learner_target() {
        // cite: etcd v3.6.10 ErrLearnerNotPromoted
        let _tenant_id = "mt-008";
        let store = KvStore::new();
        store.set_raft_role(RaftRole::Leader);
        let learner = store
            .member_add(&MemberAddRequest {
                peer_ur_ls: vec!["http://l:2380".into()],
                is_learner: true,
            })
            .member
            .id;
        let err = move_leader(&store, &MoveLeaderRequest { target_id: learner });
        assert!(matches!(err, Err(EtcdError::MemberNotLearner(_))));
    }

    #[test]
    fn test_move_leader_rejects_self_target() {
        // cite: etcd v3.6.10 ErrSelfTransfer
        let _tenant_id = "mt-009";
        let store = KvStore::new();
        store.set_raft_role(RaftRole::Leader);
        let err = move_leader(&store, &MoveLeaderRequest {
            target_id: store.local_member_id(),
        });
        assert!(matches!(err, Err(EtcdError::Internal(_))));
    }
}
