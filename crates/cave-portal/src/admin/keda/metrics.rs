// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/keda/metrics` — per-scaler Prometheus-backed stats.
//!
//! Mirrors the upstream KEDA Grafana dashboard
//! (<https://grafana.com/grafana/dashboards/17265-keda/>) but renders the
//! same metric set as a server-side HTML table so operators don't need
//! the dashboard plumbing to see the headline numbers.
//!
//! The numbers below come from upstream-defined Prometheus series:
//! * `keda_scaler_metrics_value{scaler}` (gauge) — last observed metric.
//! * `keda_scaler_metrics_latency_seconds_bucket{scaler}` — Histogram.
//! * `keda_scaler_errors_total{scaler}` (counter) — sync error count.
//! * `keda_scaler_events_total{scaledobject,kind}` (counter) — scale events.
//!
//! Today we serve a static derivation off the per-ScaledObject status
//! table so the page is meaningful even without a live cave-metrics
//! scrape; once the parallel RuntimeClient session lands real metric
//! queries, the `events_per_min`/`errors_per_min` fields swap in.

use crate::admin::keda::types::KedaScaledObjectDetail;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

/// One row in the metrics table. The fields map to the upstream Prom
/// series enumerated in the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct ScalerMetricsRow {
    pub namespace: String,
    pub scaled_object: String,
    pub scaler_kind: String,
    pub events_per_min: f32,
    pub errors_per_min: f32,
    pub latency_p50_ms: f32,
    pub latency_p99_ms: f32,
    pub last_metric_value: f32,
}

pub fn rows(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ScalerMetricsRow>, Error> {
    ctx.authorise(Permission::KedaMetricsRead)?;
    let sos: Vec<KedaScaledObjectDetail> =
        scope(&state.keda_scaled_object_details.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect();
    let mut out = Vec::new();
    for so in sos {
        for t in &so.triggers {
            // Derive shape: scale-events count scales with min/max and
            // current activity; failure count derives from
            // status.health.overall.
            let (errors_per_min, latency_p99) = match so.status.health.overall.as_str() {
                "Healthy" => (0.0, 18.0),
                "Degraded" => (0.4, 90.0),
                "Unhealthy" => (3.5, 410.0),
                _ => (0.1, 42.0),
            };
            let active_factor = if so.status.active_triggers.contains(&t.kind) { 4.0 } else { 0.5 };
            let events_per_min = active_factor * (so.max_replica_count.max(1) as f32 / 8.0);
            let last_value = match t.kind.as_str() {
                "kafka" => 2400.0,
                "prometheus" => 87.0,
                "cron" => so.max_replica_count as f32,
                "cpu" => 75.0,
                _ => 1.0,
            };
            out.push(ScalerMetricsRow {
                namespace: so.namespace.clone(),
                scaled_object: so.name.clone(),
                scaler_kind: t.kind.clone(),
                events_per_min,
                errors_per_min,
                latency_p50_ms: latency_p99 / 6.0,
                latency_p99_ms: latency_p99,
                last_metric_value: last_value,
            });
        }
    }
    Ok(out)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, Error> {
    let rows = rows(state, ctx)?;
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.namespace.clone(),
                r.scaled_object.clone(),
                r.scaler_kind.clone(),
                format!("{:.2}", r.events_per_min),
                format!("{:.2}", r.errors_per_min),
                format!("{:.0}", r.latency_p50_ms),
                format!("{:.0}", r.latency_p99_ms),
                format!("{:.2}", r.last_metric_value),
            ]
        })
        .collect();
    let body = format!(
        r#"<h2 class="text-lg font-semibold mb-2">Scaler metrics ({n})</h2>
{tbl}
<p class="mt-3 text-xs text-gray-500">Series mirror upstream Grafana dashboard 17265:
<code>keda_scaler_metrics_value</code>, <code>keda_scaler_errors_total</code>,
<code>keda_scaler_metrics_latency_seconds_bucket</code>. Sourced from the
admin store fixtures today; the parallel RuntimeClient session swaps these
for live cave-metrics queries when it lands.</p>"#,
        n = rows.len(),
        tbl = table(
            &[
                "namespace",
                "scaledObject",
                "scaler",
                "events/min",
                "errors/min",
                "p50 latency (ms)",
                "p99 latency (ms)",
                "lastMetricValue",
            ],
            &cells
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/metrics",
        &format!("keda · metrics · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn rows_emit_one_per_trigger_per_scaledobject() {
        let state = AdminState::seeded();
        let rs = rows(&state, &ctx(&[Permission::KedaMetricsRead])).unwrap();
        // acme tenant: ingest-worker (2 triggers) + report-runner (1) = 3 rows
        assert_eq!(rs.len(), 3, "expected 3 rows for acme tenant");
        // active triggers report higher events_per_min than idle ones.
        let active = rs
            .iter()
            .find(|r| r.scaler_kind == "kafka")
            .expect("kafka trigger row present");
        let idle = rs
            .iter()
            .find(|r| r.scaler_kind == "prometheus")
            .expect("prometheus trigger row present");
        assert!(
            active.events_per_min > idle.events_per_min,
            "active scaler should report higher events/min"
        );
    }

    #[test]
    fn rows_without_permission_refused() {
        let state = AdminState::seeded();
        assert!(matches!(rows(&state, &ctx(&[])).unwrap_err(), Error::Auth(_)));
    }

    #[test]
    fn render_lists_canonical_grafana_columns() {
        let state = AdminState::seeded();
        let html = render(&state, &ctx(&[Permission::KedaMetricsRead])).unwrap();
        for h in [
            "events/min",
            "errors/min",
            "p50 latency (ms)",
            "p99 latency (ms)",
            "lastMetricValue",
        ] {
            assert!(html.contains(h), "missing metric column `{}`", h);
        }
        // Cross-link to upstream Grafana dashboard.
        assert!(html.contains("Grafana dashboard 17265"));
    }
}
