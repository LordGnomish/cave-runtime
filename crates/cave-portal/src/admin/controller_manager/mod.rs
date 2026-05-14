//! `/admin/controller-manager` — kube-controller-manager Web UI parity.
//!
//! Tabs mirror the upstream controller-manager's per-controller scope
//! and observability surface:
//! * **Controllers** — full registered controller catalog.
//! * **Leader election** — lease browser (legacy).
//! * **Events** — recent reconcile-loop events.
//! * **Queues** — per-controller work-queue depth + processing rate.
//! * **Reconciler metrics** — reconcile latency p50/p99 + error rate.
//!
//! Upstream: <https://kubernetes.io/docs/reference/command-line-tools-reference/kube-controller-manager/>
//!
//! Each submodule owns its accessors + tests; `mod.rs` composes them.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod controllers;
pub mod events;
pub mod leader_election;
pub mod queues;
pub mod reconciler_metrics;

pub use leader_election::list_leases;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ControllerManagerViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ControllerManagerViewError> {
    ctx.authorise(Permission::ControllerManagerRead)?;
    let controllers_html = controllers::render_section(state, ctx)?;
    let leader_html = leader_election::render_section(state, ctx)?;
    let events_html = events::render_section(state, ctx)?;
    let queues_html = queues::render_section(state, ctx)?;
    let metrics_html = reconciler_metrics::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  kube-controller-manager Web UI parity (cave-controller-manager).
  Upstream: <a class="text-blue-700 underline" href="https://kubernetes.io/docs/reference/command-line-tools-reference/kube-controller-manager/">kubernetes.io/.../kube-controller-manager</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#cm-controllers">Controllers</a>
  <a href="#cm-leader-election">Leader election</a>
  <a href="#cm-events">Events</a>
  <a href="#cm-queues">Queues</a>
  <a href="#cm-reconciler">Metrics</a>
</nav>
{controllers}
{leader}
{events}
{queues}
{metrics}"##,
        controllers = controllers_html,
        leader = leader_html,
        events = events_html,
        queues = queues_html,
        metrics = metrics_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/controller-manager",
        &format!("controller-manager · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/Resources/Leases.tsx",
    "LeaseList",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_controller_manager_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ControllerManagerRead])).unwrap();
        for anchor in ["#cm-controllers", "#cm-leader-election", "#cm-events", "#cm-queues", "#cm-reconciler"] {
            assert!(html.contains(anchor));
        }
    }
}
