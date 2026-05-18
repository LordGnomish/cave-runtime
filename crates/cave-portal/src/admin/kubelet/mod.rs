// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/kubelet` — Kubernetes-Dashboard parity surface for the
//! workload view (Pods, Nodes, Volumes, Events, per-node metrics).
//!
//! Tabs mirror upstream Kubernetes Dashboard:
//! * **Pods** — per-node pod table with status badges + restart count
//! * **Nodes** — per-node summary (pods, capacity, allocatable, taints)
//! * **Volumes** — pod → PVC bindings
//! * **Events** — chronological per-node event tail
//! * **Metrics** — CPU/memory utilisation + IO derived numbers
//!
//! Upstream: <https://github.com/kubernetes/dashboard>
//!
//! Each submodule owns its data accessors, helper functions, render
//! section, and tests. `mod.rs` re-exports the legacy `list_pods`,
//! `pods_on_node`, `restart_pod`, `pod_summary`, `pods_with_status`,
//! `RESTART_HOT_THRESHOLD`, and `KubeletViewError` so any older caller
//! keeps compiling, and composes the five tabs into one page.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod events;
pub mod metrics;
pub mod nodes;
pub mod pods;
pub mod volumes;

// Legacy re-exports.
pub use pods::{
    list_pods, pod_summary, pods_on_node, pods_with_status, restart_pod, PodSummary,
    RESTART_HOT_THRESHOLD,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KubeletViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("pod {0} not found on this tenant")]
    PodNotFound(String),
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KubeletViewError> {
    ctx.authorise(Permission::KubeletRead)?;
    let pods_html = pods::render_section(state, ctx)?;
    let nodes_html = nodes::render_section(state, ctx)?;
    let volumes_html = volumes::render_section(state, ctx)?;
    let events_html = events::render_section(state, ctx)?;
    let metrics_html = metrics::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Kubernetes Dashboard per-workload surface.
  Upstream: <a class="text-blue-700 underline" href="https://github.com/kubernetes/dashboard">github.com/kubernetes/dashboard</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#kubelet-pods">Pods</a>
  <a href="#kubelet-nodes">Nodes</a>
  <a href="#kubelet-volumes">Volumes</a>
  <a href="#kubelet-events">Events</a>
  <a href="#kubelet-metrics">Metrics</a>
</nav>
{pods}
{nodes}
{volumes}
{events}
{metrics}"##,
        pods = pods_html,
        nodes = nodes_html,
        volumes = volumes_html,
        events = events_html,
        metrics = metrics_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/kubelet",
        &format!("kubelet · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/Pods/PodDrawer.tsx",
    "PodDrawer",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/PodsPage.tsx",
            "AllTabs",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KubeletRead])).unwrap();
        for anchor in [
            "#kubelet-pods",
            "#kubelet-nodes",
            "#kubelet-volumes",
            "#kubelet-events",
            "#kubelet-metrics",
        ] {
            assert!(html.contains(anchor), "missing anchor {anchor}");
        }
        assert!(html.contains("github.com/kubernetes/dashboard"));
    }

    #[test]
    fn render_excludes_evil_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/PodsPage.tsx",
            "TenantIsolation",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KubeletRead])).unwrap();
        assert!(html.contains("web-0"));
        assert!(!html.contains("x-0"));
        assert!(!html.contains("evil-node"));
    }

    #[test]
    fn render_requires_kubelet_read_permission() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }
}
