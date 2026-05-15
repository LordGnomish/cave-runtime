//! Cluster tab — Nodes / Namespaces / Events.

use super::K8sDashboardViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{scope, AdminState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterNodeRow {
    pub name: String,
    pub ready: bool,
    pub allocatable_cpu_milli: u64,
    pub allocatable_mem_mib: u64,
    pub taints: Vec<String>,
}

pub fn list_nodes(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ClusterNodeRow>, K8sDashboardViewError> {
    ctx.authorise(Permission::K8sDashboardRead)?;
    Ok(scope(
        &state.scheduler_nodes.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .map(|n| ClusterNodeRow {
        name: n.name.clone(),
        ready: n.ready,
        allocatable_cpu_milli: n.allocatable_cpu_milli,
        allocatable_mem_mib: n.allocatable_mem_mib,
        taints: n.taints.clone(),
    })
    .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceRow {
    pub name: String,
    pub status: &'static str,
    pub pod_count: u32,
}

pub fn list_namespaces(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<NamespaceRow>, K8sDashboardViewError> {
    let workloads = super::workloads::list_workloads(state, ctx)?;
    let total_pods = workloads.iter().filter(|w| !w.pod_name.is_empty()).count() as u32;
    Ok(vec![
        NamespaceRow {
            name: "default".into(),
            status: "Active",
            pod_count: total_pods,
        },
        NamespaceRow {
            name: "kube-system".into(),
            status: "Active",
            pod_count: 0,
        },
        NamespaceRow {
            name: "kube-public".into(),
            status: "Active",
            pod_count: 0,
        },
        NamespaceRow {
            name: "cave-system".into(),
            status: "Active",
            pod_count: 0,
        },
    ])
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, K8sDashboardViewError> {
    let nodes = list_nodes(state, ctx)?;
    let namespaces = list_namespaces(state, ctx)?;
    let node_rows: Vec<Vec<String>> = nodes
        .iter()
        .map(|n| {
            vec![
                n.name.clone(),
                if n.ready { "Ready" } else { "NotReady" }.into(),
                format!("{}m", n.allocatable_cpu_milli),
                format!("{}Mi", n.allocatable_mem_mib),
                if n.taints.is_empty() { "—".into() } else { n.taints.join(", ") },
            ]
        })
        .collect();
    let ns_rows: Vec<Vec<String>> = namespaces
        .iter()
        .map(|n| {
            vec![
                n.name.clone(),
                n.status.into(),
                n.pod_count.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="k8s-dashboard-cluster" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Cluster</h2>
  <h3 class="text-md font-semibold mt-3 mb-1">Nodes ({nn})</h3>
  {node_tbl}
  <h3 class="text-md font-semibold mt-3 mb-1">Namespaces ({nsn})</h3>
  {ns_tbl}
</section>"#,
        nn = nodes.len(),
        nsn = namespaces.len(),
        node_tbl = table(&["name", "status", "cpu", "mem", "taints"], &node_rows),
        ns_tbl = table(&["name", "status", "pods"], &ns_rows),
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
    fn list_nodes_filters_to_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Cluster.tsx",
            "Nodes",
            "acme"
        );
        let s = AdminState::seeded();
        let nodes = list_nodes(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        assert!(!nodes.iter().any(|n| n.name == "evil-node"));
    }

    #[test]
    fn list_nodes_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_nodes(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_namespaces_includes_kube_system_set() {
        let s = AdminState::seeded();
        let ns = list_namespaces(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        let names: std::collections::HashSet<_> = ns.iter().map(|n| n.name.clone()).collect();
        for expected in ["default", "kube-system", "kube-public", "cave-system"] {
            assert!(names.contains(expected), "missing namespace {expected}");
        }
    }

    #[test]
    fn render_section_includes_both_subsections() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        assert!(html.contains("Nodes ("));
        assert!(html.contains("Namespaces ("));
    }
}
