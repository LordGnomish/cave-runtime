//! Glue between [`RaftCore`] and the HTTP transport.
//!
//! The core ([`crate::raft_core::RaftCore`]) is a pure state machine: it
//! consumes RPC args and produces RPC replies + outbound side-effects.
//! This module:
//!
//! * Owns the shared `Arc<Mutex<RaftCore>>`.
//! * Mounts `POST /raft/rpc` (RequestVote + AppendEntries RPCs over JSON).
//! * Mounts `GET /api/v1/cluster/leader` (current role, term, leader id).
//! * Mounts `POST /api/v1/cluster/propose` (submit a command — leader only).
//! * Spawns a 50 ms driver task that ticks the core, fans outbound RPCs
//!   to peers via CA-pinned reqwest, and routes replies back.
//!
//! The transport choice is the same JSON-over-TLS reqwest client the
//! rest of cave-cluster uses; on a 3-node single-host smoke this means
//! https://127.0.0.1:6443/raft/rpc and friends.

use crate::raft_core::{
    AppendEntriesArgs, AppendEntriesReply, LogEntry, LogIndex, NodeId, OutboundCtx,
    OutboundMessage, ProposeError, RaftCore, RequestVoteArgs, RequestVoteReply, Role, Term,
};
use crate::raft_transport::PeerRegistry;
use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Shared handle the host passes around. Cheap to clone.
#[derive(Clone)]
pub struct RaftHandle {
    pub core: Arc<Mutex<RaftCore>>,
    pub registry: Arc<PeerRegistry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RaftRpc {
    RequestVote(RequestVoteArgs),
    AppendEntries(AppendEntriesArgs),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RaftRpcReply {
    RequestVote(RequestVoteReply),
    AppendEntries(AppendEntriesReply),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderInfo {
    pub local_id: NodeId,
    pub role: String,
    pub current_term: Term,
    pub leader_id: Option<NodeId>,
    /// HTTPS URL the client can issue writes to. Populated from the
    /// peer registry when `leader_id` is known. `None` during an
    /// election window or partition.
    #[serde(default)]
    pub leader_url: Option<String>,
    pub commit_index: LogIndex,
    pub last_applied: LogIndex,
    pub log_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposeRequest {
    /// Base64-encoded opaque command.
    pub command_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposeResponse {
    pub assigned_index: LogIndex,
    pub current_term: Term,
}

/// Returns a router that mounts `/raft/rpc` + `/api/v1/cluster/leader` +
/// `/api/v1/cluster/propose`. Use this when building the apiserver
/// listener.
pub fn router(handle: RaftHandle) -> Router {
    use axum::routing::{get, post};
    Router::new()
        .route("/raft/rpc", post(handle_raft_rpc))
        .route("/api/v1/cluster/leader", get(handle_leader_info))
        .route("/api/v1/cluster/propose", post(handle_propose))
        .with_state(handle)
}

async fn handle_raft_rpc(
    State(handle): State<RaftHandle>,
    Json(rpc): Json<RaftRpc>,
) -> Result<Json<RaftRpcReply>, (StatusCode, String)> {
    let now = Instant::now();
    let mut core = handle.core.lock().await;
    match rpc {
        RaftRpc::RequestVote(args) => {
            handle.registry.note_heartbeat(args.candidate_id);
            let reply = core
                .handle_request_vote(args, now)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("vote: {e}")))?;
            Ok(Json(RaftRpcReply::RequestVote(reply)))
        }
        RaftRpc::AppendEntries(args) => {
            handle.registry.note_heartbeat(args.leader_id);
            let reply = core
                .handle_append_entries(args, now)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("append: {e}")))?;
            Ok(Json(RaftRpcReply::AppendEntries(reply)))
        }
    }
}

async fn handle_leader_info(State(handle): State<RaftHandle>) -> Json<LeaderInfo> {
    let core = handle.core.lock().await;
    let leader_id = core.leader();
    let leader_url = leader_id.and_then(|id| handle.registry.url_for(id));
    Json(LeaderInfo {
        local_id: core.local_id,
        role: format!("{:?}", core.role()),
        current_term: core.current_term(),
        leader_id,
        leader_url,
        commit_index: core.commit_index(),
        last_applied: core.last_applied(),
        log_len: core.log_len(),
    })
}

async fn handle_propose(
    State(handle): State<RaftHandle>,
    Json(req): Json<ProposeRequest>,
) -> Result<Json<ProposeResponse>, (StatusCode, String)> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&req.command_b64)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("command_b64: {e}")))?;
    let mut core = handle.core.lock().await;
    match core.propose(bytes) {
        Ok(idx) => Ok(Json(ProposeResponse {
            assigned_index: idx,
            current_term: core.current_term(),
        })),
        Err(ProposeError::NotLeader(role, leader)) => Err((
            StatusCode::CONFLICT,
            format!(
                "not leader (role={:?}, leader={})",
                role,
                leader
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "<unknown>".into())
            ),
        )),
    }
}

/// Driver task: ticks the core, fans outbound RPCs to peers, routes
/// replies back. Runs forever (until the process exits). The host
/// spawns one per process.
pub async fn run_driver(handle: RaftHandle, ca_pem: String) -> Result<()> {
    use anyhow::Context;
    // Build an HTTP client pinned to the cluster CA (same shape as the
    // legacy heartbeat path in raft_transport).
    let pinned_cert = if ca_pem.is_empty() {
        None
    } else {
        Some(
            reqwest::Certificate::from_pem(ca_pem.as_bytes())
                .context("parse pinned CA for raft driver client")?,
        )
    };
    let mut builder = reqwest::Client::builder()
        .danger_accept_invalid_hostnames(true)
        .timeout(Duration::from_secs(2));
    if let Some(c) = pinned_cert {
        builder = builder.add_root_certificate(c);
    } else {
        builder = builder.danger_accept_invalid_certs(true);
    }
    let client = builder.build().context("build raft driver http client")?;

    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.tick().await; // skip immediate first tick
    loop {
        tick.tick().await;
        let now = Instant::now();
        let outbounds = {
            let mut core = handle.core.lock().await;
            core.tick(now).map_err(|e| anyhow::anyhow!("tick: {e}"))?
        };
        if outbounds.is_empty() {
            continue;
        }
        for (ob, ctx) in outbounds {
            let to = ob.to;
            let peer_url = match handle
                .registry
                .snapshot()
                .into_iter()
                .find(|m| m.id == to)
                .map(|m| m.url)
            {
                Some(u) => u,
                None => continue,
            };
            let endpoint = format!("{}/raft/rpc", peer_url.trim_end_matches('/'));
            let rpc = match ob.msg {
                OutboundMessage::RequestVote(a) => RaftRpc::RequestVote(a),
                OutboundMessage::AppendEntries(a) => RaftRpc::AppendEntries(a),
            };
            let client_ = client.clone();
            let handle_ = handle.clone();
            tokio::spawn(async move {
                match client_.post(&endpoint).json(&rpc).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<RaftRpcReply>().await {
                            Ok(reply) => {
                                let now = Instant::now();
                                let mut core = handle_.core.lock().await;
                                match (reply, ctx) {
                                    (RaftRpcReply::RequestVote(r), OutboundCtx::Vote) => {
                                        if let Err(e) =
                                            core.handle_request_vote_reply(to, r, now)
                                        {
                                            warn!(error = %e, "vote-reply handler");
                                        }
                                    }
                                    (
                                        RaftRpcReply::AppendEntries(r),
                                        OutboundCtx::Append {
                                            prev_log_index,
                                            entries_len,
                                        },
                                    ) => {
                                        if let Err(e) = core.handle_append_entries_reply(
                                            to,
                                            r,
                                            prev_log_index,
                                            entries_len,
                                            now,
                                        ) {
                                            warn!(error = %e, "append-reply handler");
                                        }
                                    }
                                    _ => warn!("reply variant mismatch"),
                                }
                            }
                            Err(e) => debug!(endpoint, error = %e, "decode raft reply"),
                        }
                    }
                    Ok(resp) => debug!(endpoint, status = %resp.status(), "raft rpc non-2xx"),
                    Err(e) => debug!(endpoint, error = %e, "raft rpc send failed"),
                }
            });
        }
    }
}

/// Convenience: take the local node's leader-info as plain text — used
/// by the 3-node smoke harness.
#[allow(dead_code)]
pub async fn snapshot_leader_info(handle: &RaftHandle) -> LeaderInfo {
    let core = handle.core.lock().await;
    let leader_id = core.leader();
    let leader_url = leader_id.and_then(|id| handle.registry.url_for(id));
    LeaderInfo {
        local_id: core.local_id,
        role: format!("{:?}", core.role()),
        current_term: core.current_term(),
        leader_id,
        leader_url,
        commit_index: core.commit_index(),
        last_applied: core.last_applied(),
        log_len: core.log_len(),
    }
}

// Silence dead-code warnings for items the host code reaches for but
// rustc doesn't see through dynamic dispatch.
#[allow(dead_code)]
fn _types_used(_e: LogEntry, _r: Role) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn make_handle(node: NodeId, peers: Vec<NodeId>, dir: &Path) -> RaftHandle {
        let core = RaftCore::load_or_init(node, peers, dir, Instant::now()).unwrap();
        let registry = Arc::new(PeerRegistry::new(node, format!("https://node-{node}")));
        RaftHandle {
            core: Arc::new(Mutex::new(core)),
            registry,
        }
    }

    #[tokio::test]
    async fn handle_raft_rpc_routes_request_vote_to_core() {
        let tmp = TempDir::new().unwrap();
        let handle = make_handle(1, vec![1, 2, 3], tmp.path());
        let resp = handle_raft_rpc(
            State(handle.clone()),
            Json(RaftRpc::RequestVote(RequestVoteArgs {
                term: 5,
                candidate_id: 2,
                last_log_index: 0,
                last_log_term: 0,
            })),
        )
        .await
        .unwrap();
        match resp.0 {
            RaftRpcReply::RequestVote(r) => {
                assert!(r.vote_granted);
                assert_eq!(r.term, 5);
            }
            _ => panic!("expected RequestVote reply"),
        }
        let core = handle.core.lock().await;
        assert_eq!(core.voted_for(), Some(2));
        assert_eq!(core.current_term(), 5);
    }

    #[tokio::test]
    async fn handle_leader_info_reports_role_and_term() {
        let tmp = TempDir::new().unwrap();
        let handle = make_handle(7, vec![7], tmp.path());
        let info = handle_leader_info(State(handle.clone())).await.0;
        assert_eq!(info.local_id, 7);
        assert_eq!(info.role, "Follower");
        assert_eq!(info.current_term, 0);
    }

    #[tokio::test]
    async fn handle_propose_rejects_when_not_leader() {
        use base64::Engine as _;
        let tmp = TempDir::new().unwrap();
        let handle = make_handle(1, vec![1, 2, 3], tmp.path());
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"x");
        let resp = handle_propose(
            State(handle),
            Json(ProposeRequest { command_b64: b64 }),
        )
        .await;
        match resp {
            Err((status, body)) => {
                assert_eq!(status, StatusCode::CONFLICT);
                assert!(body.contains("not leader"));
            }
            Ok(_) => panic!("must reject when not leader"),
        }
    }

    #[tokio::test]
    async fn handle_propose_accepts_after_manual_leader_promotion() {
        use base64::Engine as _;
        let tmp = TempDir::new().unwrap();
        let handle = make_handle(1, vec![1, 2, 3], tmp.path());
        // Manually promote to leader (skipping the election path).
        {
            let mut core = handle.core.lock().await;
            let now = Instant::now();
            core.become_candidate_for_test(now);
            core.become_leader_for_test(now);
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"hello");
        let resp = handle_propose(
            State(handle.clone()),
            Json(ProposeRequest { command_b64: b64 }),
        )
        .await
        .unwrap();
        assert_eq!(resp.0.assigned_index, 1);
        let core = handle.core.lock().await;
        assert_eq!(core.last_log_index(), 1);
    }
}
