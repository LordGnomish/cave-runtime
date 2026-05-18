// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/net` — Cilium Hubble Web UI parity surface.
//!
//! Tabs mirror Cilium Hubble UI + the Kubernetes Dashboard Network
//! tab:
//! * **Flows** — L3/L4/L7 source → destination flow viewer (verdict).
//! * **Policies** — NetworkPolicy CRUD + impact analysis.
//! * **Services** — ClusterIP/NodePort/LoadBalancer service list.
//! * **Nodes** — Cilium node + endpoint browser with agent health.
//! * **Identities** — Cilium security identity catalog.
//!
//! Upstream: <https://docs.cilium.io/en/stable/observability/hubble/hubble-ui/>
//!
//! Each submodule owns its accessors + tests; mod.rs composes them
//! and re-exports `list_endpoints`, `list_policies`, `create_policy`,
//! `delete_policy`, `NetViewError` so legacy callers keep compiling.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod flows;
pub mod identities;
pub mod nodes;
pub mod policies;
pub mod services;

pub use nodes::list_endpoints;
pub use policies::{create_policy, delete_policy, list_policies};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NetViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("policy {0} already exists in this tenant")]
    DuplicatePolicy(String),
    #[error("policy {0} not found")]
    PolicyNotFound(String),
    #[error("invalid direction {0}: must be Ingress, Egress or Both")]
    InvalidDirection(String),
    #[error("selector must be non-empty")]
    EmptySelector,
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, NetViewError> {
    ctx.authorise(Permission::NetRead)?;
    let flows_html = flows::render_section(state, ctx)?;
    let policies_html = policies::render_section(state, ctx)?;
    let services_html = services::render_section(state, ctx)?;
    let nodes_html = nodes::render_section(state, ctx)?;
    let identities_html = identities::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Cilium Hubble Web UI parity (cave-net).
  Upstream: <a class="text-blue-700 underline" href="https://docs.cilium.io/en/stable/observability/hubble/hubble-ui/">docs.cilium.io/.../hubble-ui</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#net-flows">Flows</a>
  <a href="#net-policies">Policies</a>
  <a href="#net-services">Services</a>
  <a href="#net-nodes">Nodes</a>
  <a href="#net-identities">Identities</a>
</nav>
{flows}
{policies}
{services}
{nodes}
{identities}"##,
        flows = flows_html,
        policies = policies_html,
        services = services_html,
        nodes = nodes_html,
        identities = identities_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/net",
        &format!("net · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/Network/NetworkPoliciesTab.tsx",
    "NetworkPoliciesTab",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_net_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::NetRead])).unwrap();
        for anchor in ["#net-flows", "#net-policies", "#net-services", "#net-nodes", "#net-identities"] {
            assert!(html.contains(anchor));
        }
        assert!(html.contains("docs.cilium.io"));
    }
}
