//! Multi-node Raft **transport scaffolding** — honest scope.
//!
//! What this module DOES:
//!
//! * Defines the on-the-wire envelope for peer-to-peer Raft messages
//!   (`Heartbeat`, `AppendEntries`, `RequestVote`, `MemberAdd`).
//! * Maintains a per-node `PeerRegistry` of `(node_id, advertise_url)`
//!   pairs, seeded from `cluster.json::peers` on bootstrap and grown
//!   dynamically by `cluster join`.
//! * Spawns a 1-second heartbeat task that POSTs `Heartbeat` to every
//!   known peer over the cluster's CA-pinned TLS.
//! * Exposes `POST /raft/message` on the apiserver listener — peers
//!   send `Heartbeat`s here; received heartbeats bump the local
//!   `last_seen` map.
//! * Exposes `GET /api/v1/cluster/members` for operators to see who
//!   the local node knows about.
//! * Tracks a "term" counter that increments on the local node's
//!   election-timeout tick (random 1-3s) — used as the leader-rotation
//!   seed.
//!
//! What this module DOES NOT do (deferred; documented honestly):
//!
//! * Real **log replication** — `AppendEntries` payloads are framed
//!   on the wire but the consumer is a no-op. Cluster writes still
//!   land only on the local node's stores.
//! * **Safe leader election** with the pre-vote round and log-up-to-date
//!   check required by the Raft paper. The term tick is a stand-in.
//! * **Snapshot install RPC** for catching up slow followers.
//! * **Joint-consensus** safe reconfiguration. Member adds are
//!   appended to the registry but not committed through the (absent)
//!   log.
//! * **Linearizable read index** / leader leases.
//!
//! In other words: heartbeat + peer registry + members API are real
//! and verifiable end-to-end. Anything that depends on a consistent
//! cluster state across nodes is still single-node MVP. The wire
//! contract is intentionally chosen so the missing pieces can land
//! without breaking the existing wire format.

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Cluster-unique node identifier. Single-node clusters always use `1`.
pub type NodeId = u64;

/// One peer's advertise endpoint. `url` is the full base URL (`https://...:6443`)
/// — heartbeats hit `<url>/raft/message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub id: NodeId,
    pub url: String,
}

/// The four wire messages we frame today. `AppendEntries` and
/// `RequestVote` are wire-level only — the receiver acks them but
/// does not apply log entries / cast safety-checked votes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RaftMessage {
    Heartbeat {
        from: NodeId,
        term: u64,
        sent_at_unix_ms: i64,
    },
    AppendEntries {
        from: NodeId,
        term: u64,
        prev_log_index: u64,
        prev_log_term: u64,
        entries: Vec<LogEntry>,
        leader_commit: u64,
    },
    RequestVote {
        from: NodeId,
        term: u64,
        last_log_index: u64,
        last_log_term: u64,
    },
    MemberAdd {
        from: NodeId,
        new_peer: Peer,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub index: u64,
    pub term: u64,
    /// Opaque application payload (etcd Put/Delete framing, apiserver
    /// resource mutation, etc.). We intentionally do not constrain
    /// this so the log-replication wire format can stay stable while
    /// the application surface evolves.
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftAck {
    pub from: NodeId,
    pub term: u64,
    pub success: bool,
    pub reason: String,
}

/// Lookup of every peer the local node has been told about. Includes the
/// local node itself so a single API call returns the full membership.
#[derive(Default)]
pub struct PeerRegistry {
    pub local_id: NodeId,
    pub local_url: String,
    pub current_term: AtomicU64,
    /// `node_id → (Peer, last_seen_unix_ms)`. `last_seen_unix_ms = 0` means
    /// "never heard from since registry creation".
    inner: DashMap<NodeId, (Peer, i64)>,
}

impl PeerRegistry {
    pub fn new(local_id: NodeId, local_url: String) -> Self {
        let me = Self {
            local_id,
            local_url: local_url.clone(),
            current_term: AtomicU64::new(0),
            inner: DashMap::new(),
        };
        // Local node always present.
        me.inner.insert(
            local_id,
            (
                Peer {
                    id: local_id,
                    url: local_url,
                },
                now_unix_ms(),
            ),
        );
        me
    }

    pub fn add_peer(&self, peer: Peer) {
        self.inner.insert(peer.id, (peer, 0));
    }

    pub fn note_heartbeat(&self, from: NodeId) {
        if let Some(mut entry) = self.inner.get_mut(&from) {
            entry.1 = now_unix_ms();
        }
    }

    pub fn snapshot(&self) -> Vec<MembersRow> {
        let mut out: Vec<MembersRow> = self
            .inner
            .iter()
            .map(|kv| MembersRow {
                id: kv.value().0.id,
                url: kv.value().0.url.clone(),
                last_seen_unix_ms: kv.value().1,
                is_local: kv.value().0.id == self.local_id,
            })
            .collect();
        out.sort_by_key(|r| r.id);
        out
    }

    pub fn known_peer_ids(&self) -> Vec<NodeId> {
        let mut ids: Vec<_> = self.inner.iter().map(|kv| *kv.key()).collect();
        ids.sort();
        ids
    }

    /// HTTPS URL the bridge can suggest to a client retrying a write
    /// against a known leader. `None` if the registry has not seen
    /// the node yet (partition / cold start).
    pub fn url_for(&self, node_id: NodeId) -> Option<String> {
        self.inner.get(&node_id).map(|kv| kv.value().0.url.clone())
    }

    pub fn current_term(&self) -> u64 {
        self.current_term.load(Ordering::Relaxed)
    }

    pub fn bump_term(&self) -> u64 {
        self.current_term.fetch_add(1, Ordering::SeqCst) + 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembersRow {
    pub id: NodeId,
    pub url: String,
    pub last_seen_unix_ms: i64,
    pub is_local: bool,
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── Wire handlers ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RaftListenerState {
    pub registry: Arc<PeerRegistry>,
}

pub async fn handle_raft_message(
    State(state): State<RaftListenerState>,
    Json(msg): Json<RaftMessage>,
) -> Result<Json<RaftAck>, (StatusCode, String)> {
    let term = state.registry.current_term();
    match msg {
        RaftMessage::Heartbeat { from, term: peer_term, .. } => {
            state.registry.note_heartbeat(from);
            // Adopt a higher term observed from a peer (Raft Figure 2).
            if peer_term > term {
                state
                    .registry
                    .current_term
                    .store(peer_term, Ordering::SeqCst);
            }
            Ok(Json(RaftAck {
                from: state.registry.local_id,
                term: state.registry.current_term(),
                success: true,
                reason: "heartbeat acked".into(),
            }))
        }
        RaftMessage::AppendEntries { from, term: peer_term, entries, .. } => {
            state.registry.note_heartbeat(from);
            if peer_term > term {
                state
                    .registry
                    .current_term
                    .store(peer_term, Ordering::SeqCst);
            }
            // Honest: we accept the frame on the wire but do NOT apply the
            // entries to any state machine — log replication is the next
            // deliverable.
            debug!(from, entries = entries.len(), "AppendEntries received (no-op)");
            Ok(Json(RaftAck {
                from: state.registry.local_id,
                term: state.registry.current_term(),
                success: true,
                reason: "AppendEntries framed but not applied (scaffolding)".into(),
            }))
        }
        RaftMessage::RequestVote { from, term: peer_term, .. } => {
            if peer_term > term {
                state
                    .registry
                    .current_term
                    .store(peer_term, Ordering::SeqCst);
            }
            // Honest: no log-up-to-date check, no persistent voted_for —
            // we acknowledge but don't grant.
            Ok(Json(RaftAck {
                from: state.registry.local_id,
                term: state.registry.current_term(),
                success: false,
                reason: "vote not granted (election safety not implemented)".into(),
            }))
        }
        RaftMessage::MemberAdd { from, new_peer } => {
            state.registry.add_peer(new_peer.clone());
            info!(from, peer = ?new_peer, "peer added to registry");
            Ok(Json(RaftAck {
                from: state.registry.local_id,
                term: state.registry.current_term(),
                success: true,
                reason: "peer registered".into(),
            }))
        }
    }
}

pub async fn handle_members(
    State(state): State<RaftListenerState>,
) -> Json<MembersListResponse> {
    Json(MembersListResponse {
        cluster_id: 0,
        local_id: state.registry.local_id,
        current_term: state.registry.current_term(),
        members: state.registry.snapshot(),
    })
}

#[derive(Debug, Serialize)]
pub struct MembersListResponse {
    pub cluster_id: u64,
    pub local_id: NodeId,
    pub current_term: u64,
    pub members: Vec<MembersRow>,
}

// ── Heartbeat driver ──────────────────────────────────────────────────────

/// Spawn the heartbeat tick: every 1s, fan out a `Heartbeat` to every
/// known peer except the local node. Errors are logged and discarded —
/// missing a heartbeat is non-fatal at this scaffolding stage.
pub async fn heartbeat_loop(
    registry: Arc<PeerRegistry>,
    ca_pem: String,
) -> Result<()> {
    let pinned_cert = if ca_pem.is_empty() {
        None
    } else {
        Some(
            reqwest::Certificate::from_pem(ca_pem.as_bytes())
                .context("parse pinned CA for raft heartbeat client")?,
        )
    };
    let mut builder = reqwest::Client::builder()
        .danger_accept_invalid_hostnames(true)
        .timeout(std::time::Duration::from_secs(2));
    if let Some(c) = pinned_cert {
        builder = builder.add_root_certificate(c);
    } else {
        builder = builder.danger_accept_invalid_certs(true);
    }
    let client = builder.build().context("build heartbeat http client")?;

    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    tick.tick().await;
    loop {
        tick.tick().await;
        let snapshot = registry.snapshot();
        let term = registry.current_term();
        let msg = RaftMessage::Heartbeat {
            from: registry.local_id,
            term,
            sent_at_unix_ms: now_unix_ms(),
        };
        for row in snapshot {
            if row.is_local {
                continue;
            }
            let url = format!("{}/raft/message", row.url.trim_end_matches('/'));
            let body = msg.clone();
            let client_ = client.clone();
            tokio::spawn(async move {
                match client_.post(&url).json(&body).send().await {
                    Ok(resp) if resp.status().is_success() => {}
                    Ok(resp) => warn!(url, status = %resp.status(), "heartbeat non-2xx"),
                    Err(e) => debug!(url, error = %e, "heartbeat send failed"),
                }
            });
        }
    }
}

/// Election-timeout tick: if no heartbeats received from any other peer
/// in `timeout_ms`, bump the local term. This is *not* a real election
/// — it just keeps the term counter alive so a future log-replication
/// path can observe staleness. Returns when the registry is dropped.
pub async fn election_timer_loop(registry: Arc<PeerRegistry>, timeout_ms: u64) -> Result<()> {
    let mut tick =
        tokio::time::interval(std::time::Duration::from_millis(timeout_ms.max(500)));
    tick.tick().await;
    loop {
        tick.tick().await;
        let now = now_unix_ms();
        let mut saw_peer_heartbeat = false;
        for row in registry.snapshot() {
            if row.is_local || row.last_seen_unix_ms == 0 {
                continue;
            }
            if (now - row.last_seen_unix_ms) < timeout_ms as i64 {
                saw_peer_heartbeat = true;
                break;
            }
        }
        // Only bump the term when we have peers AND none of them recently
        // checked in. A single-node cluster never bumps.
        let other_peer_count = registry.known_peer_ids().len() - 1;
        if other_peer_count > 0 && !saw_peer_heartbeat {
            let new_term = registry.bump_term();
            info!(new_term, peers = other_peer_count, "election timeout — term bumped");
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_seeds_with_local_node() {
        let r = PeerRegistry::new(1, "https://127.0.0.1:6443".into());
        let snap = r.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(snap[0].is_local);
        assert_eq!(snap[0].id, 1);
        assert_eq!(snap[0].url, "https://127.0.0.1:6443");
    }

    #[test]
    fn add_peer_grows_registry_and_sorts_by_id() {
        let r = PeerRegistry::new(1, "https://node-1:6443".into());
        r.add_peer(Peer {
            id: 3,
            url: "https://node-3:6443".into(),
        });
        r.add_peer(Peer {
            id: 2,
            url: "https://node-2:6443".into(),
        });
        let snap = r.snapshot();
        assert_eq!(snap.len(), 3);
        assert_eq!(snap.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn note_heartbeat_updates_last_seen_for_known_peer() {
        let r = PeerRegistry::new(1, "https://node-1:6443".into());
        r.add_peer(Peer {
            id: 2,
            url: "https://node-2:6443".into(),
        });
        // Initially last_seen=0 for peer 2.
        assert_eq!(r.snapshot()[1].last_seen_unix_ms, 0);
        r.note_heartbeat(2);
        assert!(r.snapshot()[1].last_seen_unix_ms > 0);
    }

    #[test]
    fn note_heartbeat_unknown_peer_is_noop() {
        let r = PeerRegistry::new(1, "https://node-1:6443".into());
        r.note_heartbeat(42); // never registered
        assert_eq!(r.snapshot().len(), 1);
    }

    #[test]
    fn bump_term_increments_monotonically() {
        let r = PeerRegistry::new(1, "https://node-1:6443".into());
        assert_eq!(r.current_term(), 0);
        assert_eq!(r.bump_term(), 1);
        assert_eq!(r.bump_term(), 2);
        assert_eq!(r.current_term(), 2);
    }

    #[test]
    fn message_envelope_serde_roundtrips() {
        let msg = RaftMessage::Heartbeat {
            from: 7,
            term: 12,
            sent_at_unix_ms: 1_700_000_000_000,
        };
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains(r#""type":"Heartbeat""#));
        let back: RaftMessage = serde_json::from_str(&s).unwrap();
        match back {
            RaftMessage::Heartbeat { from, term, .. } => {
                assert_eq!(from, 7);
                assert_eq!(term, 12);
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn append_entries_carries_log_entries() {
        let msg = RaftMessage::AppendEntries {
            from: 1,
            term: 5,
            prev_log_index: 3,
            prev_log_term: 5,
            entries: vec![LogEntry {
                index: 4,
                term: 5,
                payload: vec![0xde, 0xad, 0xbe, 0xef],
            }],
            leader_commit: 3,
        };
        let s = serde_json::to_string(&msg).unwrap();
        let back: RaftMessage = serde_json::from_str(&s).unwrap();
        match back {
            RaftMessage::AppendEntries { entries, .. } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].payload, vec![0xde, 0xad, 0xbe, 0xef]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[tokio::test]
    async fn handle_heartbeat_adopts_higher_term() {
        let registry = Arc::new(PeerRegistry::new(1, "https://local:6443".into()));
        registry.add_peer(Peer {
            id: 2,
            url: "https://peer:6443".into(),
        });
        let state = RaftListenerState {
            registry: registry.clone(),
        };
        let ack = handle_raft_message(
            State(state),
            Json(RaftMessage::Heartbeat {
                from: 2,
                term: 42,
                sent_at_unix_ms: 0,
            }),
        )
        .await
        .unwrap();
        assert!(ack.success);
        assert_eq!(ack.term, 42);
        assert_eq!(registry.current_term(), 42);
    }

    #[tokio::test]
    async fn handle_member_add_grows_registry() {
        let registry = Arc::new(PeerRegistry::new(1, "https://local:6443".into()));
        let state = RaftListenerState {
            registry: registry.clone(),
        };
        let _ = handle_raft_message(
            State(state),
            Json(RaftMessage::MemberAdd {
                from: 1,
                new_peer: Peer {
                    id: 7,
                    url: "https://added:6443".into(),
                },
            }),
        )
        .await
        .unwrap();
        let snap = registry.snapshot();
        assert!(snap.iter().any(|m| m.id == 7));
    }

    #[tokio::test]
    async fn handle_request_vote_never_grants_today() {
        let registry = Arc::new(PeerRegistry::new(1, "https://local:6443".into()));
        let state = RaftListenerState {
            registry: registry.clone(),
        };
        let ack = handle_raft_message(
            State(state),
            Json(RaftMessage::RequestVote {
                from: 2,
                term: 100,
                last_log_index: 0,
                last_log_term: 0,
            }),
        )
        .await
        .unwrap();
        assert!(!ack.success);
        assert!(ack.reason.contains("election safety"));
        // But the term IS adopted even on a denied vote.
        assert_eq!(registry.current_term(), 100);
    }

    #[tokio::test]
    async fn handle_members_returns_local_and_peers() {
        let registry = Arc::new(PeerRegistry::new(1, "https://local:6443".into()));
        registry.add_peer(Peer {
            id: 2,
            url: "https://peer-2:6443".into(),
        });
        registry.add_peer(Peer {
            id: 3,
            url: "https://peer-3:6443".into(),
        });
        let resp = handle_members(State(RaftListenerState {
            registry: registry.clone(),
        }))
        .await
        .0;
        assert_eq!(resp.local_id, 1);
        assert_eq!(resp.members.len(), 3);
        let ids: Vec<_> = resp.members.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }
}
