// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/cache` — Valkey/Redis Web UI parity surface.
//!
//! Tabs mirror RedisInsight + Valkey native console:
//! * **Keyspace** — db0..db15 stats + key browser + TTL editor + delete.
//! * **Commands** — command history + SLOWLOG.
//! * **Clients** — connected client list (CLIENT LIST equivalent).
//! * **Replication** — master/replica state, offset, backlog.
//! * **PubSub** — registered channels + subscriber counts.
//!
//! Upstream: <https://redis.io/insight/> + <https://valkey.io/>
//!
//! Each submodule owns its accessors + tests; mod.rs composes them
//! and re-exports the legacy `list_entries` / `entries_in_namespace` /
//! `set_ttl` / `delete_key` / `CacheViewError` so existing callers
//! keep compiling.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod clients;
pub mod commands;
pub mod keyspace;
pub mod pubsub;
pub mod replication;

pub use keyspace::{delete_key, entries_in_namespace, list_entries, set_ttl};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CacheViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("key {ns}/{key} not found")]
    KeyNotFound { ns: String, key: String },
    #[error("ttl_seconds must be between 1 and 86400")]
    InvalidTtl,
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    let keyspace_html = keyspace::render_section(state, ctx)?;
    let commands_html = commands::render_section(state, ctx)?;
    let clients_html = clients::render_section(state, ctx)?;
    let replication_html = replication::render_section(state, ctx)?;
    let pubsub_html = pubsub::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Valkey/Redis Web UI parity (cave-cache).
  Upstream: <a class="text-blue-700 underline" href="https://redis.io/insight/">redis.io/insight</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#cache-keyspace">Keyspace</a>
  <a href="#cache-commands">Commands</a>
  <a href="#cache-clients">Clients</a>
  <a href="#cache-replication">Replication</a>
  <a href="#cache-pubsub">PubSub</a>
</nav>
{keyspace}
{commands}
{clients}
{replication}
{pubsub}"##,
        keyspace = keyspace_html,
        commands = commands_html,
        clients = clients_html,
        replication = replication_html,
        pubsub = pubsub_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/cache",
        &format!("cache · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/redis-cache/src/components/CacheKeysList.tsx",
    "CacheKeysList",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_cache_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        for anchor in [
            "#cache-keyspace",
            "#cache-commands",
            "#cache-clients",
            "#cache-replication",
            "#cache-pubsub",
        ] {
            assert!(html.contains(anchor));
        }
        assert!(html.contains("redis.io/insight"));
    }

    #[test]
    fn render_excludes_evil_tenant() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(!html.contains("evil-1"));
    }
}
