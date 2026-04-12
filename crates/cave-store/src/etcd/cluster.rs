//! etcd Cluster + Maintenance services — single-node embedded implementation.

use std::sync::Arc;

use parking_lot::RwLock;
use tonic::{Request, Response, Status};

use crate::engine::StorageEngine;

use super::proto::etcdserverpb::{
    cluster_server::Cluster,
    maintenance_server::Maintenance,
    AlarmRequest, AlarmResponse,
    DefragmentRequest, DefragmentResponse,
    HashKvRequest, HashKvResponse,
    HashRequest, HashResponse,
    Member,
    MemberAddRequest, MemberAddResponse,
    MemberListRequest, MemberListResponse,
    MemberPromoteRequest, MemberPromoteResponse,
    MemberRemoveRequest, MemberRemoveResponse,
    MemberUpdateRequest, MemberUpdateResponse,
    MoveLeaderRequest, MoveLeaderResponse,
    ResponseHeader,
    StatusRequest, StatusResponse,
};

fn header(engine: &StorageEngine) -> ResponseHeader {
    ResponseHeader {
        cluster_id: 1,
        member_id: 1,
        revision: engine.mvcc.read().current_revision(),
        raft_term: 1,
    }
}

// ─── ClusterServer ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct ClusterMember {
    id: u64,
    name: String,
    peer_urls: Vec<String>,
    client_urls: Vec<String>,
    is_learner: bool,
}

pub struct ClusterServer {
    engine: Arc<StorageEngine>,
    members: Arc<RwLock<Vec<ClusterMember>>>,
}

impl ClusterServer {
    pub fn new(engine: Arc<StorageEngine>) -> Self {
        let self_member_inner = ClusterMember {
            id: 1,
            name: "cave-store".to_string(),
            peer_urls: vec!["http://localhost:2380".to_string()],
            client_urls: vec!["http://localhost:2379".to_string()],
            is_learner: false,
        };
        Self {
            engine,
            members: Arc::new(RwLock::new(vec![self_member_inner])),
        }
    }
}

fn member_to_proto(m: &ClusterMember) -> Member {
    Member {
        id: m.id,
        name: m.name.clone(),
        peer_urls: m.peer_urls.clone(),
        client_urls: m.client_urls.clone(),
        is_learner: m.is_learner,
    }
}

#[tonic::async_trait]
impl Cluster for ClusterServer {
    async fn member_add(&self, req: Request<MemberAddRequest>) -> Result<Response<MemberAddResponse>, Status> {
        let r = req.into_inner();
        let mut members = self.members.write();
        let id = (members.len() as u64) + 2;
        members.push(ClusterMember {
            id,
            name: format!("member-{id}"),
            peer_urls: r.peer_urls.clone(),
            client_urls: vec![],
            is_learner: r.is_learner,
        });
        let proto_members: Vec<Member> = members.iter().map(member_to_proto).collect();
        let new_member = proto_members.last().cloned().unwrap_or_else(|| Member {
            id: 1,
            name: "cave-store".to_string(),
            peer_urls: vec![],
            client_urls: vec![],
            is_learner: false,
        });
        Ok(Response::new(MemberAddResponse {
            header: Some(header(&self.engine)),
            member: Some(new_member),
            members: proto_members,
        }))
    }

    async fn member_remove(&self, req: Request<MemberRemoveRequest>) -> Result<Response<MemberRemoveResponse>, Status> {
        let r = req.into_inner();
        if r.id == 1 {
            return Err(Status::failed_precondition("cannot remove the only member"));
        }
        let mut members = self.members.write();
        members.retain(|m| m.id != r.id);
        let proto_members: Vec<Member> = members.iter().map(member_to_proto).collect();
        Ok(Response::new(MemberRemoveResponse {
            header: Some(header(&self.engine)),
            members: proto_members,
        }))
    }

    async fn member_update(&self, req: Request<MemberUpdateRequest>) -> Result<Response<MemberUpdateResponse>, Status> {
        let r = req.into_inner();
        let mut members = self.members.write();
        if let Some(m) = members.iter_mut().find(|m| m.id == r.id) {
            m.peer_urls = r.peer_urls;
        }
        let proto_members: Vec<Member> = members.iter().map(member_to_proto).collect();
        Ok(Response::new(MemberUpdateResponse {
            header: Some(header(&self.engine)),
            members: proto_members,
        }))
    }

    async fn member_list(&self, _: Request<MemberListRequest>) -> Result<Response<MemberListResponse>, Status> {
        let members = self.members.read();
        Ok(Response::new(MemberListResponse {
            header: Some(header(&self.engine)),
            members: members.iter().map(member_to_proto).collect(),
        }))
    }

    async fn member_promote(&self, req: Request<MemberPromoteRequest>) -> Result<Response<MemberPromoteResponse>, Status> {
        let r = req.into_inner();
        let mut members = self.members.write();
        if let Some(m) = members.iter_mut().find(|m| m.id == r.id) {
            m.is_learner = false;
        }
        let proto_members: Vec<Member> = members.iter().map(member_to_proto).collect();
        Ok(Response::new(MemberPromoteResponse {
            header: Some(header(&self.engine)),
            members: proto_members,
        }))
    }
}

// ─── MaintenanceServer ────────────────────────────────────────────────────────

pub struct MaintenanceServer {
    engine: Arc<StorageEngine>,
}

impl MaintenanceServer {
    pub fn new(engine: Arc<StorageEngine>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl Maintenance for MaintenanceServer {
    async fn alarm(&self, _: Request<AlarmRequest>) -> Result<Response<AlarmResponse>, Status> {
        Ok(Response::new(AlarmResponse {
            header: Some(header(&self.engine)),
            alarms: vec![],
        }))
    }

    async fn status(&self, _: Request<StatusRequest>) -> Result<Response<StatusResponse>, Status> {
        Ok(Response::new(StatusResponse {
            header: Some(header(&self.engine)),
            version: "cave-store/0.1.0".to_string(),
            db_size: 0,
            leader: 1,
            raft_index: 1,
            raft_term: 1,
            raft_applied_index: 1,
            errors: vec![],
            db_size_in_use: 0,
            is_learner: false,
        }))
    }

    async fn defragment(&self, _: Request<DefragmentRequest>) -> Result<Response<DefragmentResponse>, Status> {
        Ok(Response::new(DefragmentResponse { header: Some(header(&self.engine)) }))
    }

    async fn hash(&self, _: Request<HashRequest>) -> Result<Response<HashResponse>, Status> {
        Ok(Response::new(HashResponse {
            header: Some(header(&self.engine)),
            hash: 0,
        }))
    }

    async fn hash_kv(&self, _: Request<HashKvRequest>) -> Result<Response<HashKvResponse>, Status> {
        Ok(Response::new(HashKvResponse {
            header: Some(header(&self.engine)),
            hash: 0,
            compact_revision: self.engine.mvcc.read().compacted_revision(),
        }))
    }

    async fn move_leader(&self, _: Request<MoveLeaderRequest>) -> Result<Response<MoveLeaderResponse>, Status> {
        Err(Status::unimplemented("single-node cluster; no leader to move"))
    }
}
