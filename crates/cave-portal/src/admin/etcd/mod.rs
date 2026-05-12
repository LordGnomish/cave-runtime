//! `/admin/etcd` — etcdctl/etcd web-UI parity surface.
//!
//! etcd has no canonical web UI; this page mirrors the operator-
//! facing `etcdctl` subcommands:
//! * **Members** — cluster member list (`etcdctl member list`).
//! * **Keyspace** — KV browser + watch stream.
//! * **Leases** — active leases + TTL.
//! * **Alarms** — etcd alarms (NOSPACE, CORRUPT).
//! * **Metrics** — Raft log size, snapshot, commit duration.
//!
//! Upstream: <https://etcd.io/docs/v3.5/op-guide/>
//!
//! Each submodule owns its accessors + tests; `mod.rs` composes them.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod alarms;
pub mod keyspace;
pub mod leases;
pub mod members;
pub mod metrics;

pub use keyspace::{list_kv, watch_stream};
pub use leases::list_leases;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EtcdViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, EtcdViewError> {
    ctx.authorise(Permission::EtcdRead)?;
    let members_html = members::render_section(state, ctx)?;
    let keyspace_html = keyspace::render_section(state, ctx)?;
    let leases_html = leases::render_section(state, ctx)?;
    let alarms_html = alarms::render_section(state, ctx)?;
    let metrics_html = metrics::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  etcdctl parity surface (cave-etcd). etcd ships no canonical web UI;
  this page mirrors the most-used <code>etcdctl</code> subcommands.
  Upstream: <a class="text-blue-700 underline" href="https://etcd.io/docs/v3.5/op-guide/">etcd.io/docs/v3.5/op-guide</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#etcd-members">Members</a>
  <a href="#etcd-keyspace">Keyspace</a>
  <a href="#etcd-leases">Leases</a>
  <a href="#etcd-alarms">Alarms</a>
  <a href="#etcd-metrics">Metrics</a>
</nav>
{members}
{keyspace}
{leases}
{alarms}
{metrics}"##,
        members = members_html,
        keyspace = keyspace_html,
        leases = leases_html,
        alarms = alarms_html,
        metrics = metrics_html,
    );
    Ok(page_shell(
        &format!("etcd · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/explore/src/components/Tabs/DocsTab.tsx", "DocsTab");

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_etcd_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::EtcdRead, Permission::EtcdWatch])).unwrap();
        for anchor in ["#etcd-members", "#etcd-keyspace", "#etcd-leases", "#etcd-alarms", "#etcd-metrics"] {
            assert!(html.contains(anchor));
        }
        assert!(html.contains("etcd.io"));
    }
}
