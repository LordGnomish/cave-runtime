//! Deployments sub-page — model-serving endpoints + traffic stats.

use super::types::{MlflowViewError, ModelDeployment};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ModelDeployment>, MlflowViewError> {
    ctx.authorise(Permission::MlflowRead)?;
    let mut rows: Vec<ModelDeployment> = scope(
        &state.mlflow_deployments.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| b.request_count_24h.cmp(&a.request_count_24h).then(a.deployment_name.cmp(&b.deployment_name)));
    Ok(rows)
}

pub fn list_ready(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ModelDeployment>, MlflowViewError> {
    Ok(list(state, ctx)?.into_iter().filter(|d| d.status == "READY").collect())
}

pub fn status_histogram(rows: &[ModelDeployment]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.status.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn total_requests_24h(rows: &[ModelDeployment]) -> u64 {
    rows.iter().map(|d| d.request_count_24h).sum()
}

pub fn slow_deployments<'a>(rows: &'a [ModelDeployment], p95_threshold_ms: u32) -> Vec<&'a ModelDeployment> {
    rows.iter().filter(|d| d.p95_latency_ms > p95_threshold_ms).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MlflowViewError> {
    let rows = list(state, ctx)?;
    let hist = status_histogram(&rows);
    let total_24h = total_requests_24h(&rows);
    let chips: String = hist
        .iter()
        .map(|(s, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
                s = escape(s),
                n = n
            )
        })
        .collect();
    let rows_html: Vec<Vec<String>> = rows
        .iter()
        .map(|d| {
            vec![
                escape(&d.deployment_name),
                escape(&d.registered_model_name),
                d.model_version.to_string(),
                d.status.clone(),
                escape(&d.endpoint_url),
                d.request_count_24h.to_string(),
                format!("{} ms", d.p95_latency_ms),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3 text-sm">24h requests: <strong>{total}</strong></div><div class="mb-3">{chips}</div>{tbl}</section>"#,
        total = total_24h,
        chips = chips,
        tbl = table(
            &["name", "model", "version", "status", "endpoint", "req_24h", "p95"],
            &rows_html,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/mlflow/deployments",
        &format!("mlflow/deployments · {}", escape(ctx.tenant.as_str())),
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

    fn dep(tenant: &str, name: &str, status: &str, reqs: u64, p95: u32) -> ModelDeployment {
        ModelDeployment {
            tenant: TenantId::new(tenant).expect("t"),
            deployment_name: name.into(),
            registered_model_name: format!("{name}-model"),
            model_version: 1,
            status: status.into(),
            endpoint_url: format!("https://serve/{name}"),
            deployed_at_ms: 0,
            last_request_unix: Some(0),
            request_count_24h: reqs,
            p95_latency_ms: p95,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.mlflow_deployments.write().unwrap();
        g.push(dep("acme", "fraud-prod", "READY", 100_000, 45));
        g.push(dep("acme", "churn-canary", "READY", 5_000, 220));
        g.push(dep("acme", "old-model", "FAILED", 0, 0));
        g.push(dep("evil", "secret-dep", "READY", 1, 1));
        drop(g);
        s
    }

    #[test]
    fn list_filters_by_tenant() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|d| d.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let s = seeded();
        assert!(list(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_ready_excludes_failed_or_pending() {
        let s = seeded();
        let rows = list_ready(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|d| d.status == "READY"));
    }

    #[test]
    fn status_histogram_counts_statuses() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        let h = status_histogram(&rows);
        let ready = h.iter().find(|(s, _)| s == "READY").map(|(_, n)| *n).unwrap();
        assert_eq!(ready, 2);
    }

    #[test]
    fn total_requests_24h_sums() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert_eq!(total_requests_24h(&rows), 105_000);
    }

    #[test]
    fn slow_deployments_uses_threshold() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        let slow = slow_deployments(&rows, 100);
        assert_eq!(slow.len(), 1);
        assert_eq!(slow[0].deployment_name, "churn-canary");
    }

    #[test]
    fn render_includes_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        for col in ["name", "model", "version", "status", "endpoint", "p95"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
        assert!(html.contains("READY"));
    }

    #[test]
    fn render_includes_24h_total() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert!(html.contains("105000"));
    }

    #[test]
    fn render_excludes_foreign_tenant() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert!(!html.contains("secret-dep"));
    }
}
