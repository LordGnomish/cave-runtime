// SPDX-License-Identifier: AGPL-3.0-or-later
//! etcd Lease service — LeaseGrant, LeaseRevoke, LeaseKeepAlive, LeaseTimeToLive, LeaseLeases.

use std::sync::Arc;

use async_stream::try_stream;
use futures_core::Stream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};

use crate::engine::StorageEngine;

use super::proto::etcdserverpb::{
    lease_server::Lease, LeaseGrantRequest, LeaseGrantResponse, LeaseKeepAliveRequest,
    LeaseKeepAliveResponse, LeaseLeasesRequest, LeaseLeasesResponse, LeaseRevokeRequest,
    LeaseRevokeResponse, LeaseStatus, LeaseTimeToLiveRequest, LeaseTimeToLiveResponse,
    ResponseHeader,
};

pub struct LeaseServer {
    engine: Arc<StorageEngine>,
}

impl LeaseServer {
    pub fn new(engine: Arc<StorageEngine>) -> Self {
        Self { engine }
    }

    fn header(&self) -> ResponseHeader {
        ResponseHeader {
            cluster_id: 1,
            member_id: 1,
            revision: self.engine.mvcc.read().current_revision(),
            raft_term: 1,
        }
    }
}

fn map_err(e: crate::error::StoreError) -> Status {
    use crate::error::StoreError;
    match e {
        StoreError::LeaseNotFound(id) => Status::not_found(format!("lease {id} not found")),
        e => Status::internal(e.to_string()),
    }
}

#[tonic::async_trait]
impl Lease for LeaseServer {
    async fn lease_grant(&self, req: Request<LeaseGrantRequest>) -> Result<Response<LeaseGrantResponse>, Status> {
        let r = req.into_inner();
        if r.ttl <= 0 {
            return Err(Status::invalid_argument("TTL must be positive"));
        }
        let id = self.engine.lease_grant(r.id, r.ttl).map_err(map_err)?;
        Ok(Response::new(LeaseGrantResponse {
            header: Some(self.header()),
            id,
            ttl: r.ttl,
            error: String::new(),
        }))
    }

    async fn lease_revoke(&self, req: Request<LeaseRevokeRequest>) -> Result<Response<LeaseRevokeResponse>, Status> {
        let r = req.into_inner();
        self.engine.lease_revoke(r.id).map_err(map_err)?;
        Ok(Response::new(LeaseRevokeResponse { header: Some(self.header()) }))
    }

    type LeaseKeepAliveStream = std::pin::Pin<Box<dyn Stream<Item = Result<LeaseKeepAliveResponse, Status>> + Send>>;

    async fn lease_keep_alive(
        &self,
        req: Request<Streaming<LeaseKeepAliveRequest>>,
    ) -> Result<Response<Self::LeaseKeepAliveStream>, Status> {
        let engine = Arc::clone(&self.engine);
        let mut inbound = req.into_inner();

        let stream = try_stream! {
            while let Some(msg) = inbound.next().await {
                let msg = msg?;
                let result = engine.mvcc.write().lease_keep_alive(msg.id);
                match result {
                    Ok(ttl) => {
                        let header = ResponseHeader {
                            cluster_id: 1,
                            member_id: 1,
                            revision: engine.mvcc.read().current_revision(),
                            raft_term: 1,
                        };
                        yield LeaseKeepAliveResponse { header: Some(header), id: msg.id, ttl };
                    }
                    Err(e) => {
                        // Lease not found — yield TTL=0 as etcd does
                        tracing::warn!("lease keep-alive {}: {e}", msg.id);
                        let header = ResponseHeader {
                            cluster_id: 1,
                            member_id: 1,
                            revision: engine.mvcc.read().current_revision(),
                            raft_term: 1,
                        };
                        yield LeaseKeepAliveResponse { header: Some(header), id: msg.id, ttl: 0 };
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn lease_time_to_live(&self, req: Request<LeaseTimeToLiveRequest>) -> Result<Response<LeaseTimeToLiveResponse>, Status> {
        let r = req.into_inner();
        let (granted_ttl, remaining, keys) = self.engine.mvcc.read().lease_ttl(r.id).map_err(map_err)?;
        let keys_out = if r.keys { keys } else { vec![] };
        Ok(Response::new(LeaseTimeToLiveResponse {
            header: Some(self.header()),
            id: r.id,
            ttl: remaining,
            granted_ttl,
            keys: keys_out,
        }))
    }

    async fn lease_leases(&self, _req: Request<LeaseLeasesRequest>) -> Result<Response<LeaseLeasesResponse>, Status> {
        let ids = self.engine.mvcc.read().lease_list();
        let leases = ids.into_iter().map(|id| LeaseStatus { id }).collect();
        Ok(Response::new(LeaseLeasesResponse { header: Some(self.header()), leases }))
    }
}
