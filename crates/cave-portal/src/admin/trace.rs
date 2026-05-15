// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/trace` — Jaeger UI parity. Service list with span-rate +
//! error-rate health classification.
//!
//! Upstream UI: <https://www.jaegertracing.io/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, TraceService};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TraceViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<TraceService>, TraceViewError> {
    ctx.authorise(Permission::TraceRead)?;
    let mut rows: Vec<TraceService> = scope(&state.trace_services.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    rows.sort_by(|a, b| b.span_count_per_sec.cmp(&a.span_count_per_sec).then(a.service.cmp(&b.service)));
    Ok(rows)
}

/// Threshold (errors/1000) above which a service earns the
/// "degraded" badge. Matches Jaeger's standard SLO breakpoint.
pub const ERROR_RATE_DEGRADED_PER_K: u32 = 10;
pub const ERROR_RATE_FAILING_PER_K: u32 = 50;

pub fn service_health(svc: &TraceService) -> &'static str {
    if svc.error_rate_per_thousand >= ERROR_RATE_FAILING_PER_K { "Failing" }
    else if svc.error_rate_per_thousand >= ERROR_RATE_DEGRADED_PER_K { "Degraded" }
    else { "Healthy" }
}

pub fn total_span_rate(rows: &[TraceService]) -> u64 {
    rows.iter().map(|r| u64::from(r.span_count_per_sec)).sum()
}

pub fn degraded_services<'a>(rows: &'a [TraceService]) -> Vec<&'a TraceService> {
    rows.iter().filter(|s| service_health(s) != "Healthy").collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, TraceViewError> {
    let rows = list_records(state, ctx)?;
    let total = total_span_rate(&rows);
    let degraded = degraded_services(&rows).len();
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![
        escape(&r.service), r.span_count_per_sec.to_string(), r.error_rate_per_thousand.to_string(),
        service_health(r).into(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Jaeger UI (cave-trace). Upstream: <a class="text-blue-700 underline" href="https://www.jaegertracing.io/">jaegertracing.io</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> services</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{total}</strong> spans/s</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{degraded}</strong> not-healthy</span>
  </div>
  <h2 class="text-lg font-semibold mb-2">Services ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        total = total,
        degraded = degraded,
        tbl = table(&["service", "spans/s", "errors/1k", "health"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/trace", &format!("trace · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/trace/src/components/ServicesList.tsx", "ServicesList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    use cave_kernel::ns::TenantId;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner_sorted_by_spans_desc() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::TraceRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) { assert!(w[0].span_count_per_sec >= w[1].span_count_per_sec); }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn service_health_classifies_buckets() {
        let t = TenantId::new("t").unwrap();
        let mk = |e: u32| TraceService { tenant: t.clone(), service: "s".into(), span_count_per_sec: 10, error_rate_per_thousand: e };
        assert_eq!(service_health(&mk(0)), "Healthy");
        assert_eq!(service_health(&mk(9)), "Healthy");
        assert_eq!(service_health(&mk(10)), "Degraded");
        assert_eq!(service_health(&mk(49)), "Degraded");
        assert_eq!(service_health(&mk(50)), "Failing");
    }

    #[test]
    fn total_span_rate_sums() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::TraceRead])).unwrap();
        let expected: u64 = r.iter().map(|x| u64::from(x.span_count_per_sec)).sum();
        assert_eq!(total_span_rate(&r), expected);
    }

    #[test]
    fn degraded_services_filters() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::TraceRead])).unwrap();
        let d = degraded_services(&r);
        assert!(d.iter().all(|s| service_health(s) != "Healthy"));
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::TraceRead])).unwrap();
        assert!(html.contains("web"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::TraceRead])).unwrap();
        assert!(!html.contains("evil-svc"));
    }

    #[test]
    fn render_includes_summary_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::TraceRead])).unwrap();
        assert!(html.contains("services"));
        assert!(html.contains("jaegertracing.io"));
    }
}
