//! `/admin/k8s-dashboard` — Kubernetes Dashboard upstream-UI parity
//! scaffold.
//!
//! The cave-side equivalents (`apiserver.rs`, `kubelet.rs`,
//! `scheduler.rs`, `controller_manager.rs`) each expose one slice of
//! the control plane. This page mirrors the **upstream-UI** shape of
//! the Kubernetes Dashboard add-on — a tenant-scoped workload table
//! that joins scheduler nodes + kubelet pods so the operator has a
//! single landing page.
//!
//! Upstream UI: <https://github.com/kubernetes/dashboard>
//!
//! Status: scaffold. The 5 tests pin the join + render contracts.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum K8sDashboardViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadRow {
    pub node: String,
    pub node_ready: bool,
    pub pod_name: String,
    pub status: &'static str,
    pub restart_count: u32,
}

/// Join scheduler nodes ⨝ kubelet pods on `node` for the caller's
/// tenant. Nodes without pods still appear (single row with an empty
/// pod name) so an idle node is visible in the workload landing page;
/// pods whose node is unknown are dropped (data error, surfaced
/// elsewhere).
pub fn list_workloads(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<WorkloadRow>, K8sDashboardViewError> {
    ctx.authorise(Permission::K8sDashboardRead)?;
    let nodes = state.scheduler_nodes.read().unwrap();
    let pods = state.kubelet_pods.read().unwrap();
    let mut rows: Vec<WorkloadRow> = Vec::new();
    for node in nodes.iter().filter(|n| n.tenant.as_str() == ctx.tenant.as_str()) {
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

/// Summary counts for the upstream dashboard's header card:
/// total nodes, ready nodes, running pods, restarting pods.
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

/// Filter rows to a single node — used by the node-detail drill-down
/// the upstream Dashboard exposes.
pub fn rows_for_node<'a>(rows: &'a [WorkloadRow], node: &str) -> Vec<&'a WorkloadRow> {
    rows.iter().filter(|r| r.node == node).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, K8sDashboardViewError> {
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
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Kubernetes Dashboard workload view (cave-apiserver +
    cave-kubelet + cave-scheduler + cave-controller-manager).
    Upstream: <a class="text-blue-700 underline" href="https://github.com/kubernetes/dashboard">github.com/kubernetes/dashboard</a>.
  </p>
  <div class="mb-4 grid grid-cols-5 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">NODES</div><div class="text-2xl font-bold">{total_nodes}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">READY</div><div class="text-2xl font-bold">{ready_nodes}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">PODS</div><div class="text-2xl font-bold">{total_pods}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">RUNNING</div><div class="text-2xl font-bold text-green-700">{running_pods}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">FAILING</div><div class="text-2xl font-bold text-red-700">{failing_pods}</div></div>
  </div>
  <h2 class="text-lg font-semibold mb-2">Workloads ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        total_nodes = summary.total_nodes,
        ready_nodes = summary.ready_nodes,
        total_pods = summary.total_pods,
        running_pods = summary.running_pods,
        failing_pods = summary.failing_pods,
        tbl = table(
            &["node", "node_state", "pod", "status", "restarts"],
            &table_rows,
        ),
    );
    Ok(page_shell(
        &format!("k8s-dashboard · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/kubernetes/src/components/Workloads.tsx", "Workloads");

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
        let rows = list_workloads(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead]))
            .unwrap();
        // Seeded acme has 2 nodes (node-a Ready, node-b NotReady) and
        // pods scheduled to them. The join must surface BOTH nodes.
        let nodes_seen: std::collections::HashSet<_> = rows.iter().map(|r| &r.node).collect();
        assert!(nodes_seen.contains(&"node-a".to_string()));
        assert!(nodes_seen.contains(&"node-b".to_string()));
    }

    #[test]
    fn list_workloads_refuses_without_permission() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_workloads(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_workloads_excludes_other_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Workloads.tsx",
            "TenantFilter",
            "acme"
        );
        let rows = list_workloads(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead]))
            .unwrap();
        assert!(rows.iter().all(|r| r.node != "evil-node"));
    }

    #[test]
    fn render_links_upstream_dashboard() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Workloads.tsx",
            "RenderUpstreamLink",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead])).unwrap();
        assert!(html.contains("github.com/kubernetes/dashboard"));
    }

    #[test]
    fn workload_summary_counts_distinct_nodes_and_pod_states() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Workloads.tsx",
            "Summary",
            "acme"
        );
        let rows = list_workloads(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead]))
            .unwrap();
        let s = workload_summary(&rows);
        // node-a Ready + node-b NotReady → 2 total, 1 ready.
        assert_eq!(s.total_nodes, 2);
        assert_eq!(s.ready_nodes, 1);
        assert!(s.total_pods > 0);
        // Idle nodes contribute zero pods, not Running.
        assert!(s.running_pods <= s.total_pods);
    }

    #[test]
    fn rows_for_node_returns_only_target_rows() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Workloads.tsx",
            "RowsForNode",
            "acme"
        );
        let rows = list_workloads(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead]))
            .unwrap();
        let na = rows_for_node(&rows, "node-a");
        assert!(!na.is_empty());
        assert!(na.iter().all(|r| r.node == "node-a"));
    }

    #[test]
    fn render_includes_summary_cards() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Workloads.tsx",
            "SummaryCards",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead])).unwrap();
        assert!(html.contains("NODES"));
        assert!(html.contains("READY"));
        assert!(html.contains("PODS"));
        assert!(html.contains("RUNNING"));
        assert!(html.contains("FAILING"));
    }

    #[test]
    fn render_marks_unready_nodes() {
        // node-b is NotReady in the seed; render must surface that
        // signal so an operator can spot it from the landing page.
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Workloads.tsx",
            "RenderNotReady",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead])).unwrap();
        assert!(html.contains("NotReady"));
    }
}
