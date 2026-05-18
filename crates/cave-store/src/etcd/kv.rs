// SPDX-License-Identifier: AGPL-3.0-or-later
//! etcd KV service — Put, Range, DeleteRange, Txn, Compact.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::engine::{
    mvcc::{Compare, CompareResult, CompareTarget, TxnOp},
    StorageEngine,
};

use super::proto::{
    etcdserverpb::{
        kv_server::Kv, CompactionRequest, CompactionResponse, DeleteRangeRequest,
        DeleteRangeResponse, PutRequest, PutResponse, RangeRequest, RangeResponse,
        ResponseHeader, TxnRequest, TxnResponse,
    },
    mvccpb,
};

pub struct KvServer {
    engine: Arc<StorageEngine>,
}

impl KvServer {
    pub fn new(engine: Arc<StorageEngine>) -> Self {
        Self { engine }
    }

    fn header(&self) -> ResponseHeader {
        let rev = self.engine.mvcc.read().current_revision();
        ResponseHeader {
            cluster_id: 1,
            member_id: 1,
            revision: rev,
            raft_term: 1,
        }
    }
}

fn kv_to_proto(kv: crate::engine::KeyValue) -> mvccpb::KeyValue {
    mvccpb::KeyValue {
        key: kv.key,
        value: kv.value,
        create_revision: kv.create_revision,
        mod_revision: kv.mod_revision,
        version: kv.version,
        lease: kv.lease,
    }
}

fn map_err(e: crate::error::StoreError) -> Status {
    use crate::error::StoreError;
    match e {
        StoreError::KeyNotFound => Status::not_found("key not found"),
        StoreError::RevisionCompacted(r) => Status::out_of_range(format!("revision {r} compacted")),
        StoreError::LeaseNotFound(id) => Status::not_found(format!("lease {id} not found")),
        StoreError::InvalidRequest(msg) => Status::invalid_argument(msg),
        StoreError::PermissionDenied => Status::permission_denied("permission denied"),
        e => Status::internal(e.to_string()),
    }
}

#[tonic::async_trait]
impl Kv for KvServer {
    async fn range(&self, req: Request<RangeRequest>) -> Result<Response<RangeResponse>, Status> {
        let r = req.into_inner();
        let revision = if r.revision == 0 { None } else { Some(r.revision) };

        let mvcc = self.engine.mvcc.read();

        {
            if let Some(rev) = revision {
                let compacted = mvcc.compacted_revision();
                if rev < compacted {
                    return Err(Status::out_of_range(format!("revision {rev} has been compacted")));
                }
            }
        }

        let (mut kvs, count) = mvcc.range(
            &r.key,
            &r.range_end,
            revision,
            r.limit,
            r.keys_only,
            r.count_only,
        );

        let more = count > kvs.len() as i64;
        let header = ResponseHeader {
            cluster_id: 1,
            member_id: 1,
            revision: mvcc.current_revision(),
            raft_term: 1,
        };
        drop(mvcc);

        // Sorting
        if r.sort_order != 0 {
            use super::proto::etcdserverpb::range_request::{SortOrder, SortTarget};
            let desc = r.sort_order == SortOrder::Descend as i32;
            kvs.sort_by(|a, b| {
                let ord = match r.sort_target {
                    t if t == SortTarget::Key as i32 => a.key.cmp(&b.key),
                    t if t == SortTarget::Version as i32 => a.version.cmp(&b.version),
                    t if t == SortTarget::Create as i32 => a.create_revision.cmp(&b.create_revision),
                    t if t == SortTarget::Mod as i32 => a.mod_revision.cmp(&b.mod_revision),
                    t if t == SortTarget::Value as i32 => a.value.cmp(&b.value),
                    _ => a.key.cmp(&b.key),
                };
                if desc { ord.reverse() } else { ord }
            });
        }

        let proto_kvs = kvs.into_iter().map(kv_to_proto).collect();
        Ok(Response::new(RangeResponse { header: Some(header), kvs: proto_kvs, more, count }))
    }

    async fn put(&self, req: Request<PutRequest>) -> Result<Response<PutResponse>, Status> {
        let r = req.into_inner();
        let (_, prev) = self.engine
            .put(r.key, r.value, r.lease, r.prev_kv)
            .map_err(map_err)?;

        let header = self.header();
        Ok(Response::new(PutResponse {
            header: Some(header),
            prev_kv: prev.map(kv_to_proto),
        }))
    }

    async fn delete_range(&self, req: Request<DeleteRangeRequest>) -> Result<Response<DeleteRangeResponse>, Status> {
        let r = req.into_inner();
        let (_, prev_kvs) = self.engine
            .delete_range(r.key, r.range_end, r.prev_kv)
            .map_err(map_err)?;

        let deleted = prev_kvs.len() as i64;
        let header = self.header();
        Ok(Response::new(DeleteRangeResponse {
            header: Some(header),
            deleted,
            prev_kvs: prev_kvs.into_iter().map(kv_to_proto).collect(),
        }))
    }

    async fn txn(&self, req: Request<TxnRequest>) -> Result<Response<TxnResponse>, Status> {
        use super::proto::etcdserverpb::{
            compare::{CompareResult as PResult, CompareTarget as PTarget, TargetUnion},
            request_op::Request as ReqOp,
            response_op::Response as RespOp,
            DeleteRangeResponse, PutResponse, RangeResponse, RequestOp, ResponseOp, TxnResponse,
        };

        let r = req.into_inner();

        let cmps: Vec<Compare> = r.compare.into_iter().map(|c| {
            let result = match c.result {
                x if x == PResult::Greater as i32 => CompareResult::Greater,
                x if x == PResult::Less as i32 => CompareResult::Less,
                x if x == PResult::NotEqual as i32 => CompareResult::NotEqual,
                _ => CompareResult::Equal,
            };
            let target = match c.target {
                x if x == PTarget::Version as i32 => {
                    CompareTarget::Version(match &c.target_union {
                        Some(TargetUnion::Version(v)) => *v,
                        _ => 0,
                    })
                }
                x if x == PTarget::Create as i32 => {
                    CompareTarget::CreateRevision(match &c.target_union {
                        Some(TargetUnion::CreateRevision(v)) => *v,
                        _ => 0,
                    })
                }
                x if x == PTarget::Mod as i32 => {
                    CompareTarget::ModRevision(match &c.target_union {
                        Some(TargetUnion::ModRevision(v)) => *v,
                        _ => 0,
                    })
                }
                x if x == PTarget::Value as i32 => {
                    CompareTarget::Value(match c.target_union {
                        Some(TargetUnion::Value(v)) => v,
                        _ => vec![],
                    })
                }
                x if x == PTarget::Lease as i32 => {
                    CompareTarget::Lease(match &c.target_union {
                        Some(TargetUnion::Lease(v)) => *v,
                        _ => 0,
                    })
                }
                _ => CompareTarget::Version(0),
            };
            Compare {
                key: c.key,
                range_end: c.range_end,
                result,
                target,
            }
        }).collect();

        fn decode_ops(ops: Vec<RequestOp>) -> Vec<TxnOp> {
            ops.into_iter().filter_map(|op| op.request.map(|r| match r {
                ReqOp::RequestRange(rr) => TxnOp::Range {
                    key: rr.key,
                    range_end: rr.range_end,
                    limit: rr.limit,
                    revision: if rr.revision == 0 { None } else { Some(rr.revision) },
                    keys_only: rr.keys_only,
                    count_only: rr.count_only,
                },
                ReqOp::RequestPut(pr) => TxnOp::Put {
                    key: pr.key,
                    value: pr.value,
                    lease_id: pr.lease,
                    prev_kv: pr.prev_kv,
                },
                ReqOp::RequestDeleteRange(dr) => TxnOp::Delete {
                    key: dr.key,
                    range_end: dr.range_end,
                    prev_kv: dr.prev_kv,
                },
                ReqOp::RequestTxn(_) => TxnOp::Range {
                    key: vec![],
                    range_end: vec![],
                    limit: 0,
                    revision: None,
                    keys_only: false,
                    count_only: false,
                },
            })).collect()
        }

        let success_ops = decode_ops(r.success);
        let failure_ops = decode_ops(r.failure);

        let (succeeded, results) = self.engine.mvcc.write().txn(cmps, success_ops, failure_ops);

        let header = self.header();
        let responses: Vec<ResponseOp> = results.into_iter().map(|res| {
            use crate::engine::mvcc::TxnResult;
            let response = match res {
                TxnResult::Range { kvs, count, more } => RespOp::ResponseRange(RangeResponse {
                    header: None,
                    kvs: kvs.into_iter().map(kv_to_proto).collect(),
                    more,
                    count,
                }),
                TxnResult::Put { prev_kv } => RespOp::ResponsePut(PutResponse {
                    header: None,
                    prev_kv: prev_kv.map(kv_to_proto),
                }),
                TxnResult::Delete { deleted, prev_kvs } => RespOp::ResponseDeleteRange(DeleteRangeResponse {
                    header: None,
                    deleted,
                    prev_kvs: prev_kvs.into_iter().map(kv_to_proto).collect(),
                }),
            };
            ResponseOp { response: Some(response) }
        }).collect();

        Ok(Response::new(TxnResponse { header: Some(header), succeeded, responses }))
    }

    async fn compact(&self, req: Request<CompactionRequest>) -> Result<Response<CompactionResponse>, Status> {
        let r = req.into_inner();
        self.engine.mvcc.write().compact(r.revision).map_err(map_err)?;
        let header = self.header();
        Ok(Response::new(CompactionResponse { header: Some(header) }))
    }
}
