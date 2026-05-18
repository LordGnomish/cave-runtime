// SPDX-License-Identifier: AGPL-3.0-or-later
//! Workloads tab — joined scheduler nodes ⨝ kubelet pods view.

use super::K8sDashboardViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadRow {
    pub node: String,
    pub node_ready: bool,
    pub pod_name: String,
    pub status: &'static str,
    pub restart_count: u32,
}

pub fn list_workloads(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<WorkloadRow>, K8sDashboardViewError> {
    ctx.authorise(Permission::K8sDashboardRead)?;
    let nodes = state.scheduler_nodes.read().unwrap();
    let pods = state.kubelet_pods.read().unwrap();
    let mut rows: Vec<WorkloadRow> = Vec::new();
    for node in nodes
        .iter()
        .filter(|n| n.tenant.as_str() == ctx.tenant.as_str())
    {
        let mut matched = false;
        for pod in pods
            .iter()
            .filter(|p| p.tenant.as_str() == ctx.tenant.as_str() && p.node == node.name)
        {
            matched = true;
            rows.push(WorkloadRow {
                node: node.name.clone(),
                node_ready: node.ready,
                pod_name: pod.pod_name.clone(),
                status: pod.status,
                restart_count: pod.restart_count,
            });
        }
        if !matched {
            rows.push(WorkloadRow {
                node: node.name.clone(),
                node_ready: node.ready,
                pod_name: String::new(),
                status: "Idle",
                restart_count: 0,
            });
        }
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadSummary {
    pub total_nodes: u32,
    pub ready_nodes: u32,
    pub total_pods: u32,
    pub running_pods: u32,
    pub failing_pods: u32,
}

pub fn workload_summary(rows: &[WorkloadRow]) -> WorkloadSummary {
    use std::collections::BTreeSet;
    let mut nodes_seen: BTreeSet<&str> = BTreeSet::new();
    let mut ready_nodes_set: BTreeSet<&str> = BTreeSet::new();
    let mut total_pods = 0u32;
    let mut running_pods = 0u32;
    let mut failing_pods = 0u32;
    for r in rows {
        nodes_seen.insert(r.node.as_str());
        if r.node_ready {
            ready_nodes_set.insert(r.node.as_str());
        }
        if !r.pod_name.is_empty() {
            total_pods += 1;
            match r.status {
                "Running" => running_pods += 1,
                "Failed" => failing_pods += 1,
                _ => {}
            }
        }
    }
    WorkloadSummary {
        total_nodes: nodes_seen.len() as u32,
        ready_nodes: ready_nodes_set.len() as u32,
        total_pods,
        running_pods,
        failing_pods,
    }
}

pub fn rows_for_node<'a>(rows: &'a [WorkloadRow], node: &str) -> Vec<&'a WorkloadRow> {
    rows.iter().filter(|r| r.node == node).collect()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, K8sDashboardViewError> {
    let rows = list_workloads(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.node),
                if r.node_ready { "Ready".into() } else { "NotReady".into() },
                escape(&r.pod_name),
                r.status.into(),
                r.restart_count.to_string(),
            ]
        })
        .collect();
    let summary = workload_summary(&rows);
    Ok(format!(
        r#"<section id="k8s-dashboard-workloads" class="mt-2">
  <div class="mb-4 grid grid-cols-5 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">NODES</div><div class="text-2xl font-bold">{tn}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">READY</div><div class="text-2xl font-bold">{rn}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">PODS</div><div class="text-2xl font-bold">{tp}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">RUNNING</div><div class="text-2xl font-bold text-green-700">{rp}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">FAILING</div><div class="text-2xl font-bold text-red-700">{fp}</div></div>
  </div>
  <h2 class="text-lg font-semibold mb-2">Workloads ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tn = summary.total_nodes,
        rn = summary.ready_nodes,
        tp = summary.total_pods,
        rp = summary.running_pods,
        fp = summary.failing_pods,
        tbl = table(&["node", "node_state", "pod", "status", "restarts"], &table_rows),
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
    fn list_workloads_joins_nodes_and_pods() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Workloads.tsx",
            "JoinNodesAndPods",
            "acme"
        );
        let rows = list_workloads(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead])).unwrap();
        let nodes_seen: std::collections::HashSet<_> = rows.iter().map(|r| &r.node).collect();
        assert!(nodes_seen.contains(&"node-a".to_string()));
        assert!(nodes_seen.contains(&"node-b".to_string()));
    }

    #[test]
    fn list_workloads_refuses_without_permission() {
        assert!(list_workloads(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_workloads_excludes_other_tenant() {
        let rows = list_workloads(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead])).unwrap();
        assert!(rows.iter().all(|r| r.node != "evil-node"));
    }

    #[test]
    fn summary_counts_pods_only_when_present() {
        let rows = list_workloads(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead])).unwrap();
        let s = workload_summary(&rows);
        let manual_pods = rows.iter().filter(|r| !r.pod_name.is_empty()).count() as u32;
        assert_eq!(s.total_pods, manual_pods);
    }

    #[test]
    fn rows_for_node_filters_correctly() {
        let rows = list_workloads(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead])).unwrap();
        let on_a = rows_for_node(&rows, "node-a");
        assert!(on_a.iter().all(|r| r.node == "node-a"));
    }

    #[test]
    fn render_section_shows_summary_cards() {
        let html = render_section(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead])).unwrap();
        for label in ["NODES", "READY", "PODS", "RUNNING", "FAILING"] {
            assert!(html.contains(label));
        }
    }
}
