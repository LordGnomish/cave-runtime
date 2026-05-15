// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gossip tab — recent PING / PONG / MEET / FAIL events.

use super::super::CacheViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GossipEventRow {
    pub timestamp_unix: i64,
    pub kind: &'static str,
    pub from_node_id: &'static str,
    pub to_node_id: &'static str,
    pub note: &'static str,
}

pub fn recent_events(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<GossipEventRow>, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(vec![
        GossipEventRow {
            timestamp_unix: 1_715_750_000,
            kind: "PING",
            from_node_id: "a1b2c3d4",
            to_node_id: "b2c3d4e5",
            note: "routine heartbeat",
        },
        GossipEventRow {
            timestamp_unix: 1_715_750_001,
            kind: "PONG",
            from_node_id: "b2c3d4e5",
            to_node_id: "a1b2c3d4",
            note: "reply",
        },
        GossipEventRow {
            timestamp_unix: 1_715_750_002,
            kind: "MEET",
            from_node_id: "a1b2c3d4",
            to_node_id: "d4e5f607",
            note: "new node intro",
        },
        GossipEventRow {
            timestamp_unix: 1_715_750_003,
            kind: "FAIL",
            from_node_id: "a1b2c3d4",
            to_node_id: "c3d4e5f6",
            note: "promoted from PFAIL",
        },
        GossipEventRow {
            timestamp_unix: 1_715_750_004,
            kind: "UPDATE",
            from_node_id: "b2c3d4e5",
            to_node_id: "*",
            note: "slot 1234 ownership",
        },
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GossipStats {
    pub messages_sent: u64,
    pub messages_received: u64,
    pub heartbeat_interval_ms: u64,
    pub node_timeout_ms: u64,
}

pub fn stats(_state: &AdminState, ctx: &RequestCtx) -> Result<GossipStats, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(GossipStats {
        messages_sent: 51_873,
        messages_received: 51_701,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 15_000,
    })
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CacheViewError> {
    let st = stats(state, ctx)?;
    let events = recent_events(state, ctx)?;
    let rows: Vec<Vec<String>> = events
        .iter()
        .map(|e| {
            vec![
                e.timestamp_unix.to_string(),
                e.kind.into(),
                e.from_node_id.into(),
                e.to_node_id.into(),
                e.note.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cache-cluster-gossip" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Gossip Bus</h2>
  <dl class="grid grid-cols-[14rem_1fr] gap-x-4 gap-y-1 text-sm">
    <dt class="text-gray-500">cluster_stats_messages_sent</dt><dd>{s}</dd>
    <dt class="text-gray-500">cluster_stats_messages_received</dt><dd>{r}</dd>
    <dt class="text-gray-500">heartbeat_interval</dt><dd>{hi} ms</dd>
    <dt class="text-gray-500">cluster_node_timeout</dt><dd>{nt} ms</dd>
  </dl>
  <h3 class="text-md font-semibold mt-3 mb-1">Recent events ({n})</h3>
  {tbl}
</section>"#,
        s = st.messages_sent,
        r = st.messages_received,
        hi = st.heartbeat_interval_ms,
        nt = st.node_timeout_ms,
        n = events.len(),
        tbl = table(&["ts", "kind", "from", "to", "note"], &rows),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn recent_events_returns_at_least_one_per_kind() {
        let evts = recent_events(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        let kinds: std::collections::HashSet<&str> = evts.iter().map(|e| e.kind).collect();
        assert!(kinds.contains("PING"));
        assert!(kinds.contains("MEET"));
        assert!(kinds.contains("FAIL"));
    }

    #[test]
    fn stats_reports_heartbeat_at_100ms() {
        let st = stats(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert_eq!(st.heartbeat_interval_ms, 100);
    }

    #[test]
    fn render_includes_messages_sent_and_received() {
        let h = render_section(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(h.contains("messages_sent"));
        assert!(h.contains("messages_received"));
    }

    #[test]
    fn refuses_without_cache_read_perm() {
        assert!(stats(&AdminState::seeded(), &ctx(&[])).is_err());
    }
}
