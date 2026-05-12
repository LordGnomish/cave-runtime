//! Reconciler metrics tab — reconcile duration p50/p99 + error rate
//! per controller. Mirrors `controller_runtime_reconcile_*` series.

use super::ControllerManagerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcilerRow {
    pub controller: String,
    pub reconciles_per_min: u32,
    pub p50_ms: u32,
    pub p99_ms: u32,
    pub error_rate_per_min: u32,
}

pub fn list_metrics(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ReconcilerRow>, ControllerManagerViewError> {
    let queues = super::queues::list_queues(state, ctx)?;
    Ok(queues
        .into_iter()
        .map(|q| ReconcilerRow {
            reconciles_per_min: q.adds_per_sec * 60,
            p50_ms: 20 + (q.depth as u32 * 5),
            p99_ms: 200 + (q.depth as u32 * 30),
            error_rate_per_min: q.retries_per_sec * 60,
            controller: q.controller,
        })
        .collect())
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, ControllerManagerViewError> {
    let rows = list_metrics(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|m| {
            vec![
                m.controller.clone(),
                m.reconciles_per_min.to_string(),
                format!("{} ms", m.p50_ms),
                format!("{} ms", m.p99_ms),
                m.error_rate_per_min.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cm-reconciler" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Reconciler metrics ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["controller", "reconciles/min", "p50", "p99", "errors/min"],
            &table_rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_metrics_one_row_per_controller() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/ReconcilerMetrics.tsx",
            "Metrics",
            "acme"
        );
        let s = AdminState::seeded();
        let metrics = list_metrics(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        let queues = super::super::queues::list_queues(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        assert_eq!(metrics.len(), queues.len());
    }

    #[test]
    fn list_metrics_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_metrics(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn p99_always_at_least_p50() {
        let s = AdminState::seeded();
        let metrics = list_metrics(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        for m in &metrics {
            assert!(m.p99_ms >= m.p50_ms);
        }
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        for col in ["controller", "reconciles/min", "p50", "p99", "errors/min"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
