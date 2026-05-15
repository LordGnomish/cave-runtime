// SPDX-License-Identifier: AGPL-3.0-or-later
//! Topology tab — CLUSTER NODES / CLUSTER SLOTS parity.

use super::super::CacheViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterNodeRow {
    pub node_id: &'static str,
    pub addr: &'static str,
    pub role: &'static str,
    pub state: &'static str,
    pub slot_ranges: &'static str,
    pub flags: &'static str,
}

pub fn list_nodes(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ClusterNodeRow>, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(vec![
        ClusterNodeRow {
            node_id: "a1b2c3d4e5f6071829304a5b6c7d8e9f0a1b2c3d",
            addr: "10.244.0.10:6379",
            role: "master",
            state: "ok",
            slot_ranges: "0-5460",
            flags: "MASTER,MYSELF",
        },
        ClusterNodeRow {
            node_id: "b2c3d4e5f6071829304a5b6c7d8e9f0a1b2c3d4e",
            addr: "10.244.1.10:6379",
            role: "master",
            state: "ok",
            slot_ranges: "5461-10922",
            flags: "MASTER",
        },
        ClusterNodeRow {
            node_id: "c3d4e5f6071829304a5b6c7d8e9f0a1b2c3d4e5f",
            addr: "10.244.2.10:6379",
            role: "master",
            state: "ok",
            slot_ranges: "10923-16383",
            flags: "MASTER",
        },
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterSummary {
    pub state: &'static str,
    pub slots_assigned: u16,
    pub slots_ok: u16,
    pub slots_pfail: u16,
    pub slots_fail: u16,
    pub known_nodes: u32,
    pub cluster_size: u32,
    pub current_epoch: u64,
}

pub fn summary(_state: &AdminState, ctx: &RequestCtx) -> Result<ClusterSummary, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(ClusterSummary {
        state: "ok",
        slots_assigned: 16_384,
        slots_ok: 16_384,
        slots_pfail: 0,
        slots_fail: 0,
        known_nodes: 3,
        cluster_size: 3,
        current_epoch: 12,
    })
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CacheViewError> {
    let s = summary(state, ctx)?;
    let nodes = list_nodes(state, ctx)?;
    let rows: Vec<Vec<String>> = nodes
        .iter()
        .map(|n| {
            vec![
                n.node_id.into(),
                n.addr.into(),
                n.role.into(),
                n.state.into(),
                n.slot_ranges.into(),
                n.flags.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cache-cluster-topology" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">Topology (CLUSTER INFO)</h2>
  <dl class="grid grid-cols-[14rem_1fr] gap-x-4 gap-y-1 text-sm">
    <dt class="text-gray-500">cluster_state</dt><dd><strong>{state}</strong></dd>
    <dt class="text-gray-500">cluster_slots_assigned</dt><dd>{a} / 16384</dd>
    <dt class="text-gray-500">cluster_slots_ok</dt><dd>{ok}</dd>
    <dt class="text-gray-500">cluster_slots_pfail</dt><dd>{pf}</dd>
    <dt class="text-gray-500">cluster_slots_fail</dt><dd>{f}</dd>
    <dt class="text-gray-500">cluster_known_nodes</dt><dd>{n}</dd>
    <dt class="text-gray-500">cluster_size</dt><dd>{sz}</dd>
    <dt class="text-gray-500">cluster_current_epoch</dt><dd>{ep}</dd>
  </dl>
  <h3 class="text-md font-semibold mt-3 mb-1">Nodes ({c})</h3>
  {tbl}
</section>"#,
        state = s.state,
        a = s.slots_assigned,
        ok = s.slots_ok,
        pf = s.slots_pfail,
        f = s.slots_fail,
        n = s.known_nodes,
        sz = s.cluster_size,
        ep = s.current_epoch,
        c = nodes.len(),
        tbl = table(
            &["node_id", "addr", "role", "state", "slots", "flags"],
            &rows,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_nodes_returns_three_masters_covering_all_slots() {
        let s = AdminState::seeded();
        let n = list_nodes(&s, &ctx(&[Permission::CacheRead])).unwrap();
        assert_eq!(n.len(), 3);
        assert!(n.iter().all(|x| x.role == "master"));
    }

    #[test]
    fn summary_reports_ok_state_with_full_slot_assignment() {
        let s = AdminState::seeded();
        let r = summary(&s, &ctx(&[Permission::CacheRead])).unwrap();
        assert_eq!(r.state, "ok");
        assert_eq!(r.slots_assigned, 16_384);
        assert_eq!(r.slots_pfail, 0);
    }

    #[test]
    fn render_section_includes_cluster_info_header() {
        let h = render_section(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(h.contains("CLUSTER INFO"));
        assert!(h.contains("cluster_state"));
        assert!(h.contains("cluster_slots_assigned"));
    }

    #[test]
    fn refuses_listing_without_cache_read() {
        assert!(list_nodes(&AdminState::seeded(), &ctx(&[])).is_err());
    }
}
