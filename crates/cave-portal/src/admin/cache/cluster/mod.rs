// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! `/admin/cache/cluster` — Valkey/Redis Cluster admin console.
//!
//! Five tabs mirror what RedisInsight + redis-cli `--cluster` expose:
//!
//! * **Topology** — known nodes, slot ownership ranges, FAIL/PFAIL state.
//! * **Gossip** — recent PING/PONG/MEET/FAIL events with sender/peer
//!   info.
//! * **Migration** — active resharding (slot, direction, progress).
//! * **Persistence** — last RDB save, AOF size, rewrite status, fsync
//!   policy.
//! * **Commands** — operator actions (snapshot now, rewrite AOF).
//!
//! All tabs return server-side HTML behind the same RequestCtx /
//! Permission gate as the rest of `/admin/cache`.

use super::CacheViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;

pub mod commands;
pub mod gossip;
pub mod migration;
pub mod persistence;
pub mod topology;

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    let topology_html = topology::render_section(state, ctx)?;
    let gossip_html = gossip::render_section(state, ctx)?;
    let migration_html = migration::render_section(state, ctx)?;
    let persistence_html = persistence::render_section(state, ctx)?;
    let commands_html = commands::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Valkey/Redis Cluster console (cave-cache). Mirrors
  <code>CLUSTER INFO</code> / <code>CLUSTER NODES</code> / <code>CLUSTER SLOTS</code>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#cache-cluster-topology">Topology</a>
  <a href="#cache-cluster-gossip">Gossip</a>
  <a href="#cache-cluster-migration">Migration</a>
  <a href="#cache-cluster-persistence">Persistence</a>
  <a href="#cache-cluster-commands">Commands</a>
</nav>
{topology}
{gossip}
{migration}
{persistence}
{commands}"##,
        topology = topology_html,
        gossip = gossip_html,
        migration = migration_html,
        persistence = persistence_html,
        commands = commands_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/cache/cluster",
        &format!("cache · cluster · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_cache_read_permission() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tab_anchors() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        for anchor in [
            "#cache-cluster-topology",
            "#cache-cluster-gossip",
            "#cache-cluster-migration",
            "#cache-cluster-persistence",
            "#cache-cluster-commands",
        ] {
            assert!(html.contains(anchor), "missing anchor {anchor}");
        }
    }

    #[test]
    fn render_mentions_cluster_info_in_header() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(html.contains("CLUSTER INFO"));
    }

    #[test]
    fn render_does_not_leak_evil_tenant_data() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(!html.contains("evil-1"));
    }
}
