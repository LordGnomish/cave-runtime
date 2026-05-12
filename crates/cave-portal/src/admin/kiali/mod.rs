//! `/admin/kiali` — Istio Kiali Web UI parity surface.
//!
//! Tabs mirror Kiali's top navigation:
//! * **Topology** — service graph (workload → service edges, traffic).
//! * **Workloads** — workload list (sidecar status, health, traffic).
//! * **Services** — service list (endpoints, traffic split).
//! * **Traffic** — VirtualService / DestinationRule / Gateway view.
//! * **Validations** — Istio config validation results.
//!
//! Upstream: <https://kiali.io/>
//!
//! Each submodule owns its accessors + tests; mod.rs composes them
//! into one page and re-exports the legacy `list_edges` /
//! `TopologyEdge` / `GraphNode` / `list_nodes` / `edge_health` /
//! `KialiViewError` so legacy callers keep compiling.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod services;
pub mod topology;
pub mod traffic;
pub mod validations;
pub mod workloads;

pub use topology::{edge_health, list_edges, list_nodes, GraphNode, TopologyEdge};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KialiViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KialiViewError> {
    ctx.authorise(Permission::KialiRead)?;
    let topology_html = topology::render_section(state, ctx)?;
    let workloads_html = workloads::render_section(state, ctx)?;
    let services_html = services::render_section(state, ctx)?;
    let traffic_html = traffic::render_section(state, ctx)?;
    let validations_html = validations::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Istio Kiali Web UI parity (cave-mesh).
  Upstream: <a class="text-blue-700 underline" href="https://kiali.io/">kiali.io</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#kiali-topology">Topology</a>
  <a href="#kiali-workloads">Workloads</a>
  <a href="#kiali-services">Services</a>
  <a href="#kiali-traffic">Traffic</a>
  <a href="#kiali-validations">Validations</a>
</nav>
{topology}
{workloads}
{services}
{traffic}
{validations}"##,
        topology = topology_html,
        workloads = workloads_html,
        services = services_html,
        traffic = traffic_html,
        validations = validations_html,
    );
    Ok(page_shell(
        &format!("kiali · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/kiali/src/components/Topology.tsx", "Topology");

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_kiali_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KialiRead])).unwrap();
        for anchor in [
            "#kiali-topology",
            "#kiali-workloads",
            "#kiali-services",
            "#kiali-traffic",
            "#kiali-validations",
        ] {
            assert!(html.contains(anchor), "missing anchor {anchor}");
        }
        assert!(html.contains("kiali.io"));
    }
}
