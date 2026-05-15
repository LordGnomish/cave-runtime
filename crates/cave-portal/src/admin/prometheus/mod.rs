//! `/admin/prometheus` — Prometheus Web UI parity surface.
//!
//! Tabs mirror upstream Prometheus's top navigation:
//! * **Targets** — scrape targets + sample counts + retention.
//! * **Rules** — alerting + recording rules with state.
//! * **TSDB Status** — head series, chunks, symbol table, WAL size.
//! * **Flags** — runtime flags (`--storage.tsdb.retention`, …).
//! * **Status** — service discovery + config + alertmanagers.
//!
//! Upstream: <https://prometheus.io/docs/>
//!
//! Each submodule owns its accessors + tests; mod.rs composes them
//! into one page and re-exports `list_targets` /
//! `PrometheusTargetRow` / `PrometheusViewError` so legacy callers
//! keep compiling.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod flags;
pub mod rules;
pub mod status;
pub mod targets;
pub mod tsdb;

pub use targets::{list_targets, TargetRow as PrometheusTargetRow};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PrometheusViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, PrometheusViewError> {
    ctx.authorise(Permission::PrometheusRead)?;
    let targets_html = targets::render_section(state, ctx)?;
    let rules_html = rules::render_section(state, ctx)?;
    let tsdb_html = tsdb::render_section(state, ctx)?;
    let flags_html = flags::render_section(state, ctx)?;
    let status_html = status::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Prometheus Web UI parity (cave-metrics).
  Upstream: <a class="text-blue-700 underline" href="https://prometheus.io/docs/">prometheus.io/docs</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#prometheus-targets">Targets</a>
  <a href="#prometheus-rules">Rules</a>
  <a href="#prometheus-tsdb">TSDB</a>
  <a href="#prometheus-flags">Flags</a>
  <a href="#prometheus-status">Status</a>
</nav>
{targets}
{rules}
{tsdb}
{flags}
{status}"##,
        targets = targets_html,
        rules = rules_html,
        tsdb = tsdb_html,
        flags = flags_html,
        status = status_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/prometheus",
        &format!("prometheus · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/prometheus/src/components/Targets.tsx", "Targets");

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_prometheus_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PrometheusRead])).unwrap();
        for anchor in [
            "#prometheus-targets",
            "#prometheus-rules",
            "#prometheus-tsdb",
            "#prometheus-flags",
            "#prometheus-status",
        ] {
            assert!(html.contains(anchor), "missing anchor {anchor}");
        }
        assert!(html.contains("prometheus.io/docs"));
    }
}
