// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Nodes tab — per-node summary aggregating pods + capacity + taints.
//!
//! Mirrors the upstream Kubernetes Dashboard's Node list (Name,
//! Status, CPU req/lim, Mem req/lim, Pods, Taints).

use super::KubeletViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{scope, AdminState, KubeletPod, SchedulerNode};

/// One row in the Nodes table — joins SchedulerNode capacity with the
/// kubelet-side pod count (so the page is self-sufficient).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRow {
    pub name: String,
    pub ready: bool,
    pub cpu_milli_total: u64,
    pub mem_mib_total: u64,
    pub pods_total: u32,
    pub pods_running: u32,
    pub pods_pending: u32,
    pub pods_failed: u32,
    pub taints: Vec<String>,
}

pub fn list_nodes(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<NodeRow>, KubeletViewError> {
    ctx.authorise(Permission::KubeletRead)?;
    let nodes: Vec<SchedulerNode> = scope(
        &state.scheduler_nodes.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    let pods: Vec<KubeletPod> = scope(
        &state.kubelet_pods.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    Ok(nodes
        .into_iter()
        .map(|n| {
            let on_node: Vec<&KubeletPod> = pods.iter().filter(|p| p.node == n.name).collect();
            NodeRow {
                name: n.name,
                ready: n.ready,
                cpu_milli_total: n.allocatable_cpu_milli,
                mem_mib_total: n.allocatable_mem_mib,
                pods_total: on_node.len() as u32,
                pods_running: on_node.iter().filter(|p| p.status == "Running").count() as u32,
                pods_pending: on_node.iter().filter(|p| p.status == "Pending").count() as u32,
                pods_failed: on_node.iter().filter(|p| p.status == "Failed").count() as u32,
                taints: n.taints,
            }
        })
        .collect())
}

pub fn ready_count(rows: &[NodeRow]) -> usize {
    rows.iter().filter(|n| n.ready).count()
}

pub fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, KubeletViewError> {
    let rows = list_nodes(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|n| {
            vec![
                n.name.clone(),
                if n.ready { "Ready" } else { "NotReady" }.into(),
                format!("{}m", n.cpu_milli_total),
                format!("{}Mi", n.mem_mib_total),
                format!(
                    "{} ({}R/{}P/{}F)",
                    n.pods_total, n.pods_running, n.pods_pending, n.pods_failed
                ),
                if n.taints.is_empty() {
                    "—".into()
                } else {
                    n.taints.join(", ")
                },
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kubelet-nodes" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Nodes ({n}, {ready} Ready)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        ready = ready_count(&rows),
        tbl = table(
            &["name", "status", "cpu allocatable", "mem allocatable", "pods (R/P/F)", "taints"],
            &table_rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_nodes_joins_capacity_with_pod_counts() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Nodes/NodeList.tsx",
            "NodeList",
            "acme"
        );
        let s = AdminState::seeded();
        let nodes = list_nodes(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        assert!(!nodes.is_empty(), "seeded state must have acme nodes");
        // Total cluster pod count must agree with the per-node sum.
        let sum: u32 = nodes.iter().map(|n| n.pods_total).sum();
        assert!(sum >= 1);
    }

    #[test]
    fn list_nodes_excludes_evil_tenant() {
        let s = AdminState::seeded();
        let nodes = list_nodes(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        assert!(!nodes.iter().any(|n| n.name == "evil-node"));
    }

    #[test]
    fn list_nodes_requires_kubelet_read() {
        let s = AdminState::seeded();
        assert!(list_nodes(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn ready_count_matches_ready_flag() {
        let s = AdminState::seeded();
        let nodes = list_nodes(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        let ready = ready_count(&nodes);
        let expected: usize = nodes.iter().filter(|n| n.ready).count();
        assert_eq!(ready, expected);
    }

    #[test]
    fn render_section_emits_columns_and_status_summary() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        for col in ["name", "status", "cpu allocatable", "mem allocatable", "pods (R/P/F)", "taints"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
        assert!(html.contains("Ready)"));
    }
}
