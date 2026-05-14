// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/scheduler` — kube-scheduler Web UI parity surface.
//!
//! Tabs mirror kube-scheduler's operator-facing observability:
//! * **Queue** — pending pods awaiting scheduling.
//! * **Plugins** — scheduling-framework plugin catalog + custom policies.
//! * **Bindings** — recent pod → node decisions.
//! * **Node scores** — per-node score for the last 100 cycles.
//! * **Events** — scheduling events feed.
//!
//! Upstream: <https://kubernetes.io/docs/concepts/scheduling-eviction/>
//!
//! Each submodule owns its accessors + tests; `mod.rs` composes them.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod bindings;
pub mod events;
pub mod nodescores;
pub mod plugins;
pub mod queue;

pub use plugins::{
    create_policy, delete_policy, list_nodes, list_plugins, list_policies, PluginRow,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SchedulerViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("policy {0} already exists in this tenant")]
    DuplicatePolicy(String),
    #[error("policy {0} not found")]
    PolicyNotFound(String),
    #[error("predicate must be non-empty")]
    EmptyPredicate,
    #[error("weight must be between 1 and 100")]
    InvalidWeight,
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SchedulerViewError> {
    ctx.authorise(Permission::SchedulerRead)?;
    let queue_html = queue::render_section(state, ctx)?;
    let plugins_html = plugins::render_section(state, ctx)?;
    let bindings_html = bindings::render_section(state, ctx)?;
    let nodescores_html = nodescores::render_section(state, ctx)?;
    let events_html = events::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  kube-scheduler Web UI parity (cave-scheduler).
  Upstream: <a class="text-blue-700 underline" href="https://kubernetes.io/docs/concepts/scheduling-eviction/">kubernetes.io/.../scheduling-eviction</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#scheduler-queue">Queue</a>
  <a href="#scheduler-plugins">Plugins</a>
  <a href="#scheduler-bindings">Bindings</a>
  <a href="#scheduler-nodescores">Node scores</a>
  <a href="#scheduler-events">Events</a>
</nav>
{queue}
{plugins}
{bindings}
{nodescores}
{events}"##,
        queue = queue_html,
        plugins = plugins_html,
        bindings = bindings_html,
        nodescores = nodescores_html,
        events = events_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/scheduler",
        &format!("scheduler · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/Pods/Pods.tsx",
    "NodeList",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_scheduler_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SchedulerRead])).unwrap();
        for anchor in ["#scheduler-queue", "#scheduler-plugins", "#scheduler-bindings", "#scheduler-nodescores", "#scheduler-events"] {
            assert!(html.contains(anchor));
        }
    }
}
