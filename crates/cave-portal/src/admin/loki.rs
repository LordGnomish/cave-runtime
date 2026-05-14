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
use crate::admin::render::{escape, page_shell_full, table};
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
    let mut rows: Vec<LokiStreamRow> = streams
        .iter()
        .filter(|r| r.tenant.as_str() == ctx.tenant.as_str())
        .map(|r| LokiStreamRow {
            name: r.name.clone(),
            sink: r.sink.clone(),
            ingest_rate_per_sec: r.ingest_rate_per_sec,
            retention_days: r.retention_days,
        })
        .collect();
    rows.sort_by(|a, b| a.sink.cmp(&b.sink).then(a.name.cmp(&b.name)));
    Ok(rows)
}

/// Total ingest rate across all streams (samples/sec aggregate).
/// Mirrors the Grafana-Explore-Loki header metric.
pub fn total_ingest_rate(rows: &[LokiStreamRow]) -> u64 {
    rows.iter().map(|r| u64::from(r.ingest_rate_per_sec)).sum()
}

/// Trivially validate a LogQL query — used by the query editor's
/// inline syntax hint before the request is even sent. Real LogQL has
/// a richer grammar; this checks the basic stream selector + filter
/// shape (`{label="value"} |= "pattern"`).
///
/// Returns `Ok(())` if the query parses as a stream selector,
/// `Err(reason)` otherwise.
pub fn validate_logql(query: &str) -> Result<(), String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("empty query".into());
    }
    if !trimmed.starts_with('{') {
        return Err("LogQL queries must start with a stream selector `{...}`".into());
    }
    let close = trimmed.find('}').ok_or_else(|| {
        "stream selector `{...}` is unclosed".to_string()
    })?;
    let selector = &trimmed[1..close];
    if selector.trim().is_empty() {
        return Err("stream selector must contain at least one label matcher".into());
    }
    // Each comma-separated piece must look like `k="v"` or `k=~"re"`.
    for piece in selector.split(',') {
        let p = piece.trim();
        if !(p.contains("=\"") || p.contains("=~\"") || p.contains("!=\"") || p.contains("!~\"")) {
            return Err(format!("label matcher `{p}` is malformed"));
        }
    }
    Ok(())
}

/// Filter streams to those matching a substring (Grafana Explore's
/// quick-filter chip).
pub fn filter_by_name<'a>(
    rows: &'a [LokiStreamRow],
    needle: &str,
) -> Vec<&'a LokiStreamRow> {
    let lc = needle.to_lowercase();
    rows.iter().filter(|r| r.name.to_lowercase().contains(&lc)).collect()
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
    Ok(page_shell_full(
        ctx,
        "/admin/loki",
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

    #[test]
    fn validate_logql_accepts_basic_stream_selector() {
        assert!(validate_logql(r#"{app="web"}"#).is_ok());
        assert!(validate_logql(r#"{app="web",env="prod"}"#).is_ok());
        assert!(validate_logql(r#"{app=~"web.*"}"#).is_ok());
        assert!(validate_logql(r#"{app!="web"} |= "error""#).is_ok());
    }

    #[test]
    fn validate_logql_rejects_malformed_queries() {
        assert!(validate_logql("").is_err());
        assert!(validate_logql("app=web").is_err());
        assert!(validate_logql("{").is_err());
        assert!(validate_logql("{}").is_err());
        assert!(validate_logql("{bad}").is_err());
    }

    #[test]
    fn total_ingest_rate_sums_rates() {
        let rows = vec![
            LokiStreamRow { name: "a".into(), sink: "loki".into(), ingest_rate_per_sec: 100, retention_days: 7 },
            LokiStreamRow { name: "b".into(), sink: "loki".into(), ingest_rate_per_sec: 250, retention_days: 7 },
        ];
        assert_eq!(total_ingest_rate(&rows), 350);
        assert_eq!(total_ingest_rate(&[]), 0);
    }

    #[test]
    fn filter_by_name_substring_match() {
        let rows = vec![
            LokiStreamRow { name: "web-stdout".into(), sink: "loki".into(), ingest_rate_per_sec: 100, retention_days: 7 },
            LokiStreamRow { name: "api-stdout".into(), sink: "loki".into(), ingest_rate_per_sec: 100, retention_days: 7 },
            LokiStreamRow { name: "auth-trace".into(), sink: "loki".into(), ingest_rate_per_sec: 100, retention_days: 7 },
        ];
        let matches = filter_by_name(&rows, "stdout");
        assert_eq!(matches.len(), 2);
        let empty = filter_by_name(&rows, "no-match");
        assert!(empty.is_empty());
    }

    #[test]
    fn list_streams_sorted_by_sink_then_name() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/loki/src/components/StreamsList.tsx",
            "SortedOrder",
            "acme"
        );
        let rows = list_streams(&AdminState::seeded(), &ctx(&[Permission::LokiRead])).unwrap();
        for w in rows.windows(2) {
            let a = (&w[0].sink, &w[0].name);
            let b = (&w[1].sink, &w[1].name);
            assert!(a <= b);
        }
    }
}
