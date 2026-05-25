// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! v3election RPC service.
//!
//! Mirrors etcd v3.6.10 `server/etcdserver/api/v3election/` and the proto
//! `Election` service in `api/etcdserverpb/v3election.proto`.
//!
//! Underlying election primitives (Campaign/Resign/Leader/Observe) already
//! live in [`crate::concurrency::DistElection`]; this module wraps them in
//! the request/response shapes the v3election RPC service uses and
//! multiplexes one `DistElection` per `name` (prefix) the way the upstream
//! service does.

use crate::concurrency::{ConcurrencyError, DistElection};
use crate::models::ResponseHeader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderKey {
    pub name: Vec<u8>,
    pub key: Vec<u8>,
    pub rev: i64,
    pub lease: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignRequest {
    pub name: Vec<u8>,
    pub lease: i64,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignResponse {
    pub header: ResponseHeader,
    pub leader: LeaderKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProclaimRequest {
    pub leader: LeaderKey,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProclaimResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderRequest {
    pub name: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderResponse {
    pub header: ResponseHeader,
    pub kv: Option<ElectionKv>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElectionKv {
    pub key: Vec<u8>,
    pub create_revision: i64,
    pub mod_revision: i64,
    pub value: Vec<u8>,
    pub lease: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResignRequest {
    pub leader: LeaderKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResignResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserveResponse {
    pub header: ResponseHeader,
    pub kv: Option<ElectionKv>,
}

#[derive(thiserror::Error, Debug)]
pub enum ElectionRpcError {
    #[error("election not found for name {0:?}")]
    NotFound(Vec<u8>),
    #[error("concurrency: {0}")]
    Concurrency(#[from] ConcurrencyError),
    #[error("name must be non-empty")]
    EmptyName,
    #[error("lease must be non-zero for campaign")]
    ZeroLease,
}

pub type ElectionRpcResult<T> = Result<T, ElectionRpcError>;

/// Multiplexed election service: one [`DistElection`] per `name`.
/// Mirrors `server/etcdserver/api/v3election/election.go` which keys
/// elections by prefix.
pub struct ElectionService {
    elections: Mutex<HashMap<Vec<u8>, Arc<DistElection>>>,
    cluster_id: u64,
    member_id: u64,
}

impl Default for ElectionService {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl ElectionService {
    pub fn new(cluster_id: u64, member_id: u64) -> Self {
        Self {
            elections: Mutex::new(HashMap::new()),
            cluster_id,
            member_id,
        }
    }

    fn header(&self) -> ResponseHeader {
        ResponseHeader {
            cluster_id: self.cluster_id,
            member_id: self.member_id,
            revision: 0,
            raft_term: 0,
        }
    }

    fn for_name(&self, name: &[u8]) -> Arc<DistElection> {
        let mut g = self.elections.lock().unwrap();
        g.entry(name.to_vec())
            .or_insert_with(|| {
                let prefix = String::from_utf8_lossy(name).into_owned();
                Arc::new(DistElection::new(prefix))
            })
            .clone()
    }

    /// Number of distinct election names currently tracked.
    pub fn election_count(&self) -> usize {
        self.elections.lock().unwrap().len()
    }

    pub fn campaign(&self, req: CampaignRequest) -> ElectionRpcResult<CampaignResponse> {
        if req.name.is_empty() {
            return Err(ElectionRpcError::EmptyName);
        }
        if req.lease == 0 {
            return Err(ElectionRpcError::ZeroLease);
        }
        let el = self.for_name(&req.name);
        let (rev, _became) = el.campaign(req.lease, req.value);
        // Composite key mirrors upstream: <name>/<lease-hex>
        let mut key = req.name.clone();
        key.push(b'/');
        key.extend_from_slice(format!("{:x}", req.lease).as_bytes());
        Ok(CampaignResponse {
            header: self.header(),
            leader: LeaderKey {
                name: req.name,
                key,
                rev: rev as i64,
                lease: req.lease,
            },
        })
    }

    pub fn proclaim(&self, req: ProclaimRequest) -> ElectionRpcResult<ProclaimResponse> {
        let el = self.for_name(&req.leader.name);
        el.proclaim(req.leader.lease, req.value)?;
        Ok(ProclaimResponse {
            header: self.header(),
        })
    }

    pub fn leader(&self, req: LeaderRequest) -> ElectionRpcResult<LeaderResponse> {
        let el = self.for_name(&req.name);
        let kv = el.leader().map(|c| {
            let mut key = req.name.clone();
            key.push(b'/');
            key.extend_from_slice(format!("{:x}", c.lease_id).as_bytes());
            ElectionKv {
                key,
                create_revision: c.create_revision as i64,
                mod_revision: c.create_revision as i64,
                value: c.value,
                lease: c.lease_id,
            }
        });
        Ok(LeaderResponse {
            header: self.header(),
            kv,
        })
    }

    pub fn resign(&self, req: ResignRequest) -> ElectionRpcResult<ResignResponse> {
        let el = self.for_name(&req.leader.name);
        el.resign(req.leader.lease)?;
        Ok(ResignResponse {
            header: self.header(),
        })
    }

    /// Snapshot the current leader as a single Observe message; the
    /// streaming form requires a watch-style channel and is therefore
    /// expressed as a series of `observe_once` polls in cave-etcd's
    /// single-process model (the upstream RPC streams new leaders as
    /// they are elected).
    pub fn observe_once(&self, req: LeaderRequest) -> ElectionRpcResult<ObserveResponse> {
        let r = self.leader(req)?;
        Ok(ObserveResponse {
            header: r.header,
            kv: r.kv,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> ElectionService {
        ElectionService::new(1, 1)
    }

    #[test]
    fn campaign_returns_leader_key_with_composite_key_and_rev() {
        let s = svc();
        let r = s
            .campaign(CampaignRequest {
                name: b"/svc".to_vec(),
                lease: 0x10,
                value: b"node-a".to_vec(),
            })
            .unwrap();
        assert_eq!(r.leader.name, b"/svc");
        assert_eq!(r.leader.lease, 0x10);
        assert_eq!(r.leader.key, b"/svc/10");
        assert_eq!(r.leader.rev, 1);
    }

    #[test]
    fn campaign_rejects_empty_name() {
        let s = svc();
        let err = s
            .campaign(CampaignRequest {
                name: vec![],
                lease: 1,
                value: vec![],
            })
            .unwrap_err();
        assert!(matches!(err, ElectionRpcError::EmptyName));
    }

    #[test]
    fn campaign_rejects_zero_lease() {
        let s = svc();
        let err = s
            .campaign(CampaignRequest {
                name: b"/x".to_vec(),
                lease: 0,
                value: vec![],
            })
            .unwrap_err();
        assert!(matches!(err, ElectionRpcError::ZeroLease));
    }

    #[test]
    fn leader_after_campaign_is_first_in() {
        let s = svc();
        s.campaign(CampaignRequest {
            name: b"/svc".to_vec(),
            lease: 1,
            value: b"A".to_vec(),
        })
        .unwrap();
        s.campaign(CampaignRequest {
            name: b"/svc".to_vec(),
            lease: 2,
            value: b"B".to_vec(),
        })
        .unwrap();
        let l = s.leader(LeaderRequest {
            name: b"/svc".to_vec(),
        })
        .unwrap();
        let kv = l.kv.expect("leader");
        assert_eq!(kv.lease, 1);
        assert_eq!(kv.value, b"A");
    }

    #[test]
    fn leader_with_no_candidates_returns_none() {
        let s = svc();
        let l = s.leader(LeaderRequest {
            name: b"/svc".to_vec(),
        })
        .unwrap();
        assert!(l.kv.is_none());
    }

    #[test]
    fn proclaim_updates_leader_value() {
        let s = svc();
        let c = s
            .campaign(CampaignRequest {
                name: b"/svc".to_vec(),
                lease: 7,
                value: b"v1".to_vec(),
            })
            .unwrap();
        s.proclaim(ProclaimRequest {
            leader: c.leader.clone(),
            value: b"v2".to_vec(),
        })
        .unwrap();
        let l = s.leader(LeaderRequest {
            name: b"/svc".to_vec(),
        })
        .unwrap();
        assert_eq!(l.kv.unwrap().value, b"v2");
    }

    #[test]
    fn proclaim_fails_when_not_leader() {
        let s = svc();
        s.campaign(CampaignRequest {
            name: b"/svc".to_vec(),
            lease: 1,
            value: b"A".to_vec(),
        })
        .unwrap();
        let c2 = s
            .campaign(CampaignRequest {
                name: b"/svc".to_vec(),
                lease: 2,
                value: b"B".to_vec(),
            })
            .unwrap();
        let err = s
            .proclaim(ProclaimRequest {
                leader: c2.leader,
                value: b"v2".to_vec(),
            })
            .unwrap_err();
        assert!(matches!(err, ElectionRpcError::Concurrency(_)));
    }

    #[test]
    fn resign_promotes_next_candidate() {
        let s = svc();
        let c1 = s
            .campaign(CampaignRequest {
                name: b"/svc".to_vec(),
                lease: 1,
                value: b"A".to_vec(),
            })
            .unwrap();
        s.campaign(CampaignRequest {
            name: b"/svc".to_vec(),
            lease: 2,
            value: b"B".to_vec(),
        })
        .unwrap();
        s.resign(ResignRequest { leader: c1.leader }).unwrap();
        let l = s.leader(LeaderRequest {
            name: b"/svc".to_vec(),
        })
        .unwrap();
        assert_eq!(l.kv.unwrap().lease, 2);
    }

    #[test]
    fn observe_once_returns_current_leader() {
        let s = svc();
        s.campaign(CampaignRequest {
            name: b"/svc".to_vec(),
            lease: 9,
            value: b"X".to_vec(),
        })
        .unwrap();
        let o = s
            .observe_once(LeaderRequest {
                name: b"/svc".to_vec(),
            })
            .unwrap();
        assert_eq!(o.kv.unwrap().lease, 9);
    }

    #[test]
    fn distinct_names_are_independent() {
        let s = svc();
        s.campaign(CampaignRequest {
            name: b"/a".to_vec(),
            lease: 1,
            value: b"x".to_vec(),
        })
        .unwrap();
        s.campaign(CampaignRequest {
            name: b"/b".to_vec(),
            lease: 2,
            value: b"y".to_vec(),
        })
        .unwrap();
        assert_eq!(s.election_count(), 2);
        let la = s.leader(LeaderRequest {
            name: b"/a".to_vec(),
        })
        .unwrap();
        let lb = s.leader(LeaderRequest {
            name: b"/b".to_vec(),
        })
        .unwrap();
        assert_eq!(la.kv.unwrap().lease, 1);
        assert_eq!(lb.kv.unwrap().lease, 2);
    }
}
