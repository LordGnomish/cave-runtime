// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/cloud-controller` — Kubernetes cloud-controller-manager UI parity.
//!
//! Tabs mirror upstream `cloud-controller-manager` per-controller scope:
//! * **Node controller** — node registration + InitializeProvider state.
//! * **Route controller** — pod CIDR routes per node.
//! * **Service controller** — LoadBalancer provisioning state.
//! * **Volume controller** — managed cloud volumes (attach/detach).
//! * **Instance metadata** — InstanceID, zone, region per node.
//!
//! Upstream: <https://kubernetes.io/docs/concepts/architecture/cloud-controller/>
//!
//! Each submodule owns its accessors + tests; `mod.rs` composes them.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod instance_metadata;
pub mod node_controller;
pub mod route_controller;
pub mod service_controller;
pub mod volume_controller;

pub use volume_controller::{list_volumes, unattached_volumes};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CloudControllerViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn render(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CloudControllerViewError> {
    ctx.authorise(Permission::CloudControllerRead)?;
    let nodes_html = node_controller::render_section(state, ctx)?;
    let routes_html = route_controller::render_section(state, ctx)?;
    let services_html = service_controller::render_section(state, ctx)?;
    let volumes_html = volume_controller::render_section(state, ctx)?;
    let meta_html = instance_metadata::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Cloud-controller-manager UI (cave-cloud-controller-manager).
  Upstream: <a class="text-blue-700 underline" href="https://kubernetes.io/docs/concepts/architecture/cloud-controller/">kubernetes.io/docs/.../cloud-controller</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#ccm-nodes">NodeController</a>
  <a href="#ccm-routes">RouteController</a>
  <a href="#ccm-services">ServiceController</a>
  <a href="#ccm-volumes">VolumeController</a>
  <a href="#ccm-meta">InstanceMetadata</a>
</nav>
{nodes}
{routes}
{services}
{volumes}
{meta}"##,
        nodes = nodes_html,
        routes = routes_html,
        services = services_html,
        volumes = volumes_html,
        meta = meta_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/cloud-controller",
        &format!("cloud-controller · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/CloudResources/Volumes.tsx",
    "VolumesList",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_cloud_controller_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CloudControllerRead])).unwrap();
        for anchor in ["#ccm-nodes", "#ccm-routes", "#ccm-services", "#ccm-volumes", "#ccm-meta"] {
            assert!(html.contains(anchor));
        }
    }
}
