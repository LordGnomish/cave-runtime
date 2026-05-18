// SPDX-License-Identifier: AGPL-3.0-or-later
//! Monitoring sub-page — traffic stats per model.

use super::types::{LiteLlmTraffic, LiteLlmViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LiteLlmTraffic>, LiteLlmViewError> {
    ctx.authorise(Permission::LiteLlmRead)?;
    let mut rows: Vec<LiteLlmTraffic> =
        scope(&state.litellm_traffic.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| b.request_count.cmp(&a.request_count).then(a.model_name.cmp(&b.model_name)));
    Ok(rows)
}

pub fn total_spend_cents(rows: &[LiteLlmTraffic]) -> u64 {
    rows.iter().map(|r| r.spend_usd_cents).sum()
}

pub fn total_requests(rows: &[LiteLlmTraffic]) -> u64 {
    rows.iter().map(|r| r.request_count).sum()
}

pub fn total_errors(rows: &[LiteLlmTraffic]) -> u64 {
    rows.iter().map(|r| r.error_count).sum()
}

pub fn error_rate_pct(rows: &[LiteLlmTraffic]) -> f64 {
    let req = total_requests(rows);
    if req == 0 {
        return 0.0;
    }
    (total_errors(rows) as f64 * 100.0) / req as f64
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LiteLlmViewError> {
    let rows = list(state, ctx)?;
    let req = total_requests(&rows);
    let err_rate = error_rate_pct(&rows);
    let spend = total_spend_cents(&rows);
    let rows_html: Vec<Vec<String>> = rows
        .iter()
        .map(|t| {
            vec![
                escape(&t.model_name),
                t.window_seconds.to_string(),
                t.request_count.to_string(),
                t.error_count.to_string(),
                format!("${}.{}", t.spend_usd_cents / 100, t.spend_usd_cents % 100),
                format!("{} ms", t.avg_latency_ms),
                t.timeouts.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
<div class="grid grid-cols-3 gap-3 mb-3 text-sm">
  <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">requests</div><div class="text-xl font-bold">{req}</div></div>
  <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">error rate</div><div class="text-xl font-bold">{err:.2}%</div></div>
  <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">spend</div><div class="text-xl font-bold">${dollars}.{cents:02}</div></div>
</div>
{tbl}</section>"#,
        req = req,
        err = err_rate,
        dollars = spend / 100,
        cents = spend % 100,
        tbl = table(
            &["model", "window_s", "requests", "errors", "spend", "p50_lat", "timeouts"],
            &rows_html,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/litellm/monitoring",
        &format!("litellm/monitoring · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn t(tenant: &str, model: &str, reqs: u64, errs: u64, spend: u64) -> LiteLlmTraffic {
        LiteLlmTraffic {
            tenant: TenantId::new(tenant).expect("t"),
            model_name: model.into(),
            window_seconds: 60,
            request_count: reqs,
            error_count: errs,
            spend_usd_cents: spend,
            avg_latency_ms: 250,
            timeouts: 0,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.litellm_traffic.write().unwrap();
        g.push(t("acme", "gpt-4o", 10_000, 50, 25_00));
        g.push(t("acme", "claude-3-5-sonnet", 5_000, 5, 12_00));
        g.push(t("evil", "secret-model", 1, 0, 0));
        drop(g);
        s
    }

    #[test]
    fn list_filters_by_tenant_and_sorts_by_request_count() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].model_name, "gpt-4o");
    }

    #[test]
    fn list_refuses_without_perm() {
        let s = seeded();
        assert!(list(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn total_requests_sums() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert_eq!(total_requests(&rows), 15_000);
    }

    #[test]
    fn total_errors_sums() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert_eq!(total_errors(&rows), 55);
    }

    #[test]
    fn total_spend_cents_sums() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert_eq!(total_spend_cents(&rows), 3700);
    }

    #[test]
    fn error_rate_is_zero_on_empty_input() {
        assert_eq!(error_rate_pct(&[]), 0.0);
    }

    #[test]
    fn error_rate_reflects_ratio() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        let r = error_rate_pct(&rows);
        assert!((r - 0.3667).abs() < 0.001, "got {r}");
    }

    #[test]
    fn render_includes_summary_cards() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert!(html.contains("requests"));
        assert!(html.contains("error rate"));
        assert!(html.contains("spend"));
    }

    #[test]
    fn render_excludes_foreign_tenant() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert!(!html.contains("secret-model"));
    }
}
