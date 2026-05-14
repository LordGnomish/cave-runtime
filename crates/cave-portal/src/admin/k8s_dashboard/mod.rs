//! `/admin/k8s-dashboard` — Kubernetes Dashboard Web UI parity surface.
//!
//! Tabs mirror the upstream Kubernetes Dashboard top navigation:
//! * **Workloads** — Deployments / StatefulSets / DaemonSets / Jobs / CronJobs.
//! * **Services** — Services / Endpoints / Ingresses.
//! * **Config** — ConfigMaps / Secrets / ResourceQuotas.
//! * **Storage** — PersistentVolumes / PVCs / StorageClasses.
//! * **Cluster** — Nodes / Namespaces / Events.
//!
//! Upstream: <https://github.com/kubernetes/dashboard>
//!
//! Each submodule owns its accessors + tests; mod.rs composes them
//! into one page and re-exports the legacy `list_workloads` /
//! `WorkloadRow` / `WorkloadSummary` / `workload_summary` /
//! `rows_for_node` / `K8sDashboardViewError` so legacy callers keep
//! compiling.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod cluster;
pub mod config;
pub mod services;
pub mod storage;
pub mod workloads;

pub use workloads::{
    list_workloads, rows_for_node, workload_summary, WorkloadRow, WorkloadSummary,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum K8sDashboardViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, K8sDashboardViewError> {
    ctx.authorise(Permission::K8sDashboardRead)?;
    let workloads_html = workloads::render_section(state, ctx)?;
    let services_html = services::render_section(state, ctx)?;
    let config_html = config::render_section(state, ctx)?;
    let storage_html = storage::render_section(state, ctx)?;
    let cluster_html = cluster::render_section(state, ctx)?;
    let tid = escape(ctx.tenant.as_str());
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Kubernetes Dashboard Web UI parity (cave-apiserver + cave-kubelet + cave-scheduler).
  Upstream: <a class="text-blue-700 underline" href="https://github.com/kubernetes/dashboard">github.com/kubernetes/dashboard</a>.
  <br/>
  <span class="text-xs text-blue-700">2026-05-14 consolidation: <code>/admin/kubelet</code> + <code>/admin/scheduler</code> redirect here.</span>
</section>
<nav class="mb-4 flex flex-wrap gap-x-4 gap-y-1 text-sm text-blue-700">
  <a href="#k8s-dashboard-workloads">Workloads</a>
  <a href="#k8s-dashboard-services">Services</a>
  <a href="#k8s-dashboard-config">Config</a>
  <a href="#k8s-dashboard-storage">Storage</a>
  <a href="#k8s-dashboard-cluster">Cluster</a>
  <span class="text-gray-400">·</span>
  <a href="/admin/k8s-dashboard/pods?tenant_id={tid}">Pods</a>
  <a href="/admin/k8s-dashboard/nodes?tenant_id={tid}">Nodes</a>
  <a href="/admin/k8s-dashboard/volumes?tenant_id={tid}">Volumes</a>
  <a href="/admin/k8s-dashboard/events?tenant_id={tid}">Events</a>
  <a href="/admin/k8s-dashboard/metrics?tenant_id={tid}">Metrics</a>
  <span class="text-gray-400">·</span>
  <a href="/admin/k8s-dashboard/scheduler/queue?tenant_id={tid}">Scheduler · Queue</a>
  <a href="/admin/k8s-dashboard/scheduler/plugins?tenant_id={tid}">Plugins</a>
  <a href="/admin/k8s-dashboard/scheduler/bindings?tenant_id={tid}">Bindings</a>
  <a href="/admin/k8s-dashboard/scheduler/nodescores?tenant_id={tid}">Node scores</a>
  <a href="/admin/k8s-dashboard/scheduler/events?tenant_id={tid}">Scheduler events</a>
</nav>
{workloads}
{services}
{config}
{storage}
{cluster}"##,
        tid = tid,
        workloads = workloads_html,
        services = services_html,
        config = config_html,
        storage = storage_html,
        cluster = cluster_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/k8s-dashboard",
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

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_k8s_dashboard_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::K8sDashboardRead])).unwrap();
        for anchor in [
            "#k8s-dashboard-workloads",
            "#k8s-dashboard-services",
            "#k8s-dashboard-config",
            "#k8s-dashboard-storage",
            "#k8s-dashboard-cluster",
        ] {
            assert!(html.contains(anchor), "missing anchor {anchor}");
        }
        assert!(html.contains("github.com/kubernetes/dashboard"));
    }
}
