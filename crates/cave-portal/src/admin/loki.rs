//! `/admin/loki` — Loki LogQL query upstream-UI parity scaffold.
//!
//! Distinct from `admin/logs.rs` (cave-logs catalog view). This page
//! mirrors the **upstream-UI** shape of Grafana's Explore → Loki view
//! — list streams, show their ingest rate and retention, and (in a
//! later port) accept a LogQL query.
//!
//! Upstream UI: <https://grafana.com/docs/loki/>
//!
//! Status: scaffold. The 5 tests below pin the list/render contracts.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LokiViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LokiStreamRow {
    pub name: String,
    pub sink: String,
    pub ingest_rate_per_sec: u32,
    pub retention_days: u32,
}

pub fn list_streams(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<LokiStreamRow>, LokiViewError> {
    ctx.authorise(Permission::LokiRead)?;
    let streams = state.log_streams.read().unwrap();
    let rows = streams
        .iter()
        .filter(|r| r.tenant.as_str() == ctx.tenant.as_str())
        .map(|r| LokiStreamRow {
            name: r.name.clone(),
            sink: r.sink.clone(),
            ingest_rate_per_sec: r.ingest_rate_per_sec,
            retention_days: r.retention_days,
        })
        .collect();
    Ok(rows)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LokiViewError> {
    let rows = list_streams(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.name),
                escape(&r.sink),
                r.ingest_rate_per_sec.to_string(),
                r.retention_days.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Loki LogQL query scaffold (cave-logs).
    Upstream: <a class="text-blue-700 underline" href="https://grafana.com/docs/loki/">grafana.com/docs/loki</a>.
  </p>
  <h2 class="text-lg font-semibold mb-2">Streams ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["name", "sink", "rate/s", "retention_days"], &table_rows),
    );
    Ok(page_shell(
        &format!("loki · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/loki/src/components/StreamsList.tsx", "StreamsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_streams_filters_to_caller_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/loki/src/components/StreamsList.tsx",
            "TenantFilter",
            "acme"
        );
        let rows = list_streams(&AdminState::seeded(), &ctx(&[Permission::LokiRead])).unwrap();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| !r.name.contains("evil")));
    }

    #[test]
    fn list_streams_refuses_without_permission() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_streams(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_lists_count_in_heading() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/loki/src/components/StreamsList.tsx",
            "RenderCount",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LokiRead])).unwrap();
        assert!(html.contains("Streams ("));
    }

    #[test]
    fn render_links_loki_docs() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/loki/src/components/StreamsList.tsx",
            "RenderUpstreamLink",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LokiRead])).unwrap();
        assert!(html.contains("grafana.com/docs/loki"));
    }

    #[test]
    fn render_excludes_other_tenant_streams() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/loki/src/components/StreamsList.tsx",
            "TenantIsolation",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LokiRead])).unwrap();
        assert!(!html.contains("evil-stream"));
    }
}
