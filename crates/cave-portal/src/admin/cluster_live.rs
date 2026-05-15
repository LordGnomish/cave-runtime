//! `/admin/cluster` — live Raft state + node liveness.
//!
//! Reads (no mutate) the cluster state machine and renders the four
//! values an operator wants on a single dashboard: current term,
//! who's leader, WAL apply progress, and per-node last_seen.
//!
//! The SSE handler tails the event bus' `RaftStateChange` /
//! `ApplyEntry` / `NodeReady` events and emits one combined snapshot
//! every second. This module owns the snapshot shape + persona-scoped
//! formatter; the actual axum SSE wiring lives in `mod.rs`.

use crate::admin::events::Event;
use crate::admin::permission::{Permission, RequestCtx};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClusterLiveError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatus {
    pub node_id: String,
    pub addr: String,
    pub last_seen_ms_ago: u64,
    pub ready: bool,
    pub active_connections: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterSnapshot {
    pub term: u64,
    pub leader_id: Option<u64>,
    pub commit_index: u64,
    pub last_applied: u64,
    pub apply_lag: u64,
    pub nodes: Vec<NodeStatus>,
    pub event_count: u64,
}

impl ClusterSnapshot {
    pub fn empty() -> Self {
        Self {
            term: 0,
            leader_id: None,
            commit_index: 0,
            last_applied: 0,
            apply_lag: 0,
            nodes: Vec::new(),
            event_count: 0,
        }
    }

    pub fn apply_lag_percent(&self) -> u8 {
        if self.commit_index == 0 {
            return 100;
        }
        let applied = self.last_applied.min(self.commit_index);
        ((applied * 100) / self.commit_index) as u8
    }

    /// Render the snapshot as a minimal SVG progress bar + node table.
    /// Used by the SSE handler to push pre-rendered HTML chunks
    /// straight into the page (htmx swap target).
    pub fn render_html(&self) -> String {
        let pct = self.apply_lag_percent();
        let node_rows: String = self
            .nodes
            .iter()
            .map(|n| {
                let last_seen = if n.last_seen_ms_ago < 1000 {
                    format!("{} ms", n.last_seen_ms_ago)
                } else {
                    format!("{:.1}s", n.last_seen_ms_ago as f64 / 1000.0)
                };
                let badge = if n.ready { "bg-green-100" } else { "bg-red-100" };
                format!(
                    r#"<tr class="border-t"><td class="px-3 py-1">{id}</td><td class="px-3 py-1">{addr}</td><td class="px-3 py-1 {badge} text-center">{ready}</td><td class="px-3 py-1 text-right">{ls}</td><td class="px-3 py-1 text-right">{conn}</td></tr>"#,
                    id = n.node_id,
                    addr = n.addr,
                    badge = badge,
                    ready = if n.ready { "ready" } else { "not ready" },
                    ls = last_seen,
                    conn = n.active_connections,
                )
            })
            .collect();
        let leader = self
            .leader_id
            .map(|i| i.to_string())
            .unwrap_or_else(|| "—".into());
        format!(
            r#"<div class="grid grid-cols-4 gap-3 text-sm">
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">term</div><div class="text-2xl font-bold">{term}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">leader</div><div class="text-2xl font-bold">{leader}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">commit / applied</div><div class="text-2xl font-bold">{commit} / {applied}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">apply progress</div><div class="text-2xl font-bold">{pct}%</div></div>
</div>
<div class="mt-4 bg-white rounded shadow"><table class="w-full text-sm"><thead class="bg-gray-100"><tr><th class="px-3 py-2 text-left">node</th><th class="px-3 py-2 text-left">addr</th><th class="px-3 py-2 text-center">state</th><th class="px-3 py-2 text-right">last_seen</th><th class="px-3 py-2 text-right">conns</th></tr></thead><tbody>{rows}</tbody></table></div>"#,
            term = self.term,
            leader = leader,
            commit = self.commit_index,
            applied = self.last_applied,
            pct = pct,
            rows = node_rows,
        )
    }
}

/// In-memory snapshot store updated by the bus consumer + read by
/// the SSE handler.
#[derive(Debug)]
pub struct ClusterLiveState {
    snapshot: RwLock<ClusterSnapshot>,
    nodes: RwLock<BTreeMap<String, NodeStatus>>,
}

impl ClusterLiveState {
    pub fn new() -> Self {
        Self {
            snapshot: RwLock::new(ClusterSnapshot::empty()),
            nodes: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn read(&self, ctx: &RequestCtx) -> Result<ClusterSnapshot, ClusterLiveError> {
        ctx.authorise(Permission::ClusterLiveRead)?;
        let mut snap = self.snapshot.read().unwrap().clone();
        snap.nodes = self.nodes.read().unwrap().values().cloned().collect();
        snap.apply_lag = snap.commit_index.saturating_sub(snap.last_applied);
        Ok(snap)
    }

    pub fn upsert_node(&self, n: NodeStatus) {
        self.nodes.write().unwrap().insert(n.node_id.clone(), n);
    }

    pub fn forget_node(&self, node_id: &str) {
        self.nodes.write().unwrap().remove(node_id);
    }

    /// Fold an event into the snapshot. Cluster-wide events are
    /// authoritative; tenant-scope events bump `event_count` only.
    pub fn ingest(&self, ev: &Event) {
        let mut snap = self.snapshot.write().unwrap();
        snap.event_count += 1;
        match ev {
            Event::RaftStateChange {
                term,
                leader_id,
                commit_index,
                last_applied,
            } => {
                snap.term = *term;
                snap.leader_id = *leader_id;
                snap.commit_index = *commit_index;
                snap.last_applied = *last_applied;
            }
            Event::ApplyEntry { index, .. } => {
                if *index > snap.last_applied {
                    snap.last_applied = *index;
                }
            }
            Event::NodeReady { node, ready } => {
                drop(snap);
                let mut g = self.nodes.write().unwrap();
                let entry = g.entry(node.clone()).or_insert_with(|| NodeStatus {
                    node_id: node.clone(),
                    addr: String::new(),
                    last_seen_ms_ago: 0,
                    ready: false,
                    active_connections: 0,
                });
                entry.ready = *ready;
            }
            _ => {}
        }
    }
}

impl Default for ClusterLiveState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RequestCtx {
        RequestCtx::developer("acme", &[Permission::ClusterLiveRead])
    }

    fn node(id: &str, ready: bool) -> NodeStatus {
        NodeStatus {
            node_id: id.into(),
            addr: format!("10.0.0.{id}:6443"),
            last_seen_ms_ago: 50,
            ready,
            active_connections: 4,
        }
    }

    #[test]
    fn empty_snapshot_has_full_apply_pct() {
        let s = ClusterSnapshot::empty();
        assert_eq!(s.apply_lag_percent(), 100);
    }

    #[test]
    fn apply_pct_when_lagging() {
        let s = ClusterSnapshot {
            term: 1,
            leader_id: Some(1),
            commit_index: 100,
            last_applied: 75,
            apply_lag: 25,
            nodes: vec![],
            event_count: 0,
        };
        assert_eq!(s.apply_lag_percent(), 75);
    }

    #[test]
    fn read_requires_permission() {
        let s = ClusterLiveState::new();
        let ctx = RequestCtx::developer("acme", &[]);
        assert!(matches!(s.read(&ctx).unwrap_err(), ClusterLiveError::Auth(_)));
    }

    #[test]
    fn upsert_node_round_trips_in_snapshot() {
        let s = ClusterLiveState::new();
        s.upsert_node(node("1", true));
        s.upsert_node(node("2", false));
        let snap = s.read(&ctx()).unwrap();
        assert_eq!(snap.nodes.len(), 2);
    }

    #[test]
    fn forget_node_removes_entry() {
        let s = ClusterLiveState::new();
        s.upsert_node(node("1", true));
        s.forget_node("1");
        let snap = s.read(&ctx()).unwrap();
        assert!(snap.nodes.is_empty());
    }

    #[test]
    fn ingest_raft_state_change_updates_term_and_leader() {
        let s = ClusterLiveState::new();
        s.ingest(&Event::RaftStateChange {
            term: 7,
            leader_id: Some(2),
            commit_index: 50,
            last_applied: 45,
        });
        let snap = s.read(&ctx()).unwrap();
        assert_eq!(snap.term, 7);
        assert_eq!(snap.leader_id, Some(2));
        assert_eq!(snap.commit_index, 50);
        assert_eq!(snap.apply_lag, 5);
    }

    #[test]
    fn ingest_apply_entry_advances_last_applied() {
        let s = ClusterLiveState::new();
        s.ingest(&Event::RaftStateChange {
            term: 1,
            leader_id: Some(1),
            commit_index: 10,
            last_applied: 5,
        });
        s.ingest(&Event::ApplyEntry {
            index: 8,
            command_kind: "etcd.put".into(),
        });
        let snap = s.read(&ctx()).unwrap();
        assert_eq!(snap.last_applied, 8);
        assert_eq!(snap.apply_lag, 2);
    }

    #[test]
    fn ingest_apply_entry_does_not_regress() {
        let s = ClusterLiveState::new();
        s.ingest(&Event::ApplyEntry {
            index: 10,
            command_kind: "etcd.put".into(),
        });
        s.ingest(&Event::ApplyEntry {
            index: 5,
            command_kind: "etcd.put".into(),
        });
        let snap = s.read(&ctx()).unwrap();
        assert_eq!(snap.last_applied, 10);
    }

    #[test]
    fn ingest_node_ready_flips_state() {
        let s = ClusterLiveState::new();
        s.ingest(&Event::NodeReady {
            node: "n1".into(),
            ready: true,
        });
        s.ingest(&Event::NodeReady {
            node: "n1".into(),
            ready: false,
        });
        let snap = s.read(&ctx()).unwrap();
        assert_eq!(snap.nodes[0].ready, false);
    }

    #[test]
    fn render_html_contains_term_leader_and_pct() {
        let s = ClusterSnapshot {
            term: 9,
            leader_id: Some(3),
            commit_index: 100,
            last_applied: 80,
            apply_lag: 20,
            nodes: vec![node("1", true)],
            event_count: 0,
        };
        let html = s.render_html();
        assert!(html.contains(">9<"));
        assert!(html.contains(">3<"));
        assert!(html.contains("80%"));
        assert!(html.contains("not ready") == false);
        assert!(html.contains("ready"));
    }

    #[test]
    fn event_count_tracks_every_ingest() {
        let s = ClusterLiveState::new();
        for _ in 0..5 {
            s.ingest(&Event::ApplyEntry {
                index: 1,
                command_kind: "x".into(),
            });
        }
        let snap = s.read(&ctx()).unwrap();
        assert_eq!(snap.event_count, 5);
    }
}
