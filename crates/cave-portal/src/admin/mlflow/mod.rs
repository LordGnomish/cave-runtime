// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/mlflow` — MLflow tracking + model registry views.
//!
//! Mirrors mlflow/mlflow v2.x REST API:
//!
//! * [`experiments`]       — experiment list + active/deleted split
//! * [`runs`]              — runs per experiment + status histogram
//! * [`models`]            — registered model list
//! * [`registered_models`] — model versions + stage transitions
//! * [`deployments`]       — model-serving endpoints + traffic
//!
//! Upstream UI: <https://mlflow.org/docs/latest/index.html>

pub mod deployments;
pub mod experiments;
pub mod models;
pub mod registered_models;
pub mod runs;
pub mod types;

pub use types::{
    MlflowExperiment, MlflowRun, MlflowViewError, ModelDeployment, ModelVersion, RegisteredModel,
};

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MlflowViewError> {
    ctx.authorise(Permission::MlflowRead)?;
    let exp = experiments::list(state, ctx)?;
    let runs_total = runs::list_all(state, ctx)?.len();
    let models_total = models::list(state, ctx)?.len();
    let deployed = deployments::list(state, ctx)?
        .into_iter()
        .filter(|d| d.status == "READY")
        .count();
    let body = format!(
        r#"<section class="grid grid-cols-4 gap-3 mb-4">
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">experiments</div><div class="text-2xl font-bold">{ec}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">runs</div><div class="text-2xl font-bold">{rc}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">models</div><div class="text-2xl font-bold">{mc}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">deployed</div><div class="text-2xl font-bold">{dc}</div></div>
</section>
<nav class="flex gap-4 mb-3 text-sm">
  <a class="text-blue-700 underline" href="/admin/mlflow/experiments?tenant_id={tid}">experiments</a>
  <a class="text-blue-700 underline" href="/admin/mlflow/runs?tenant_id={tid}">runs</a>
  <a class="text-blue-700 underline" href="/admin/mlflow/models?tenant_id={tid}">models</a>
  <a class="text-blue-700 underline" href="/admin/mlflow/registered-models?tenant_id={tid}">registered models</a>
  <a class="text-blue-700 underline" href="/admin/mlflow/deployments?tenant_id={tid}">deployments</a>
</nav>"#,
        ec = exp.len(),
        rc = runs_total,
        mc = models_total,
        dc = deployed,
        tid = escape(ctx.tenant.as_str()),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/mlflow",
        &format!("mlflow · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let acme = TenantId::new("acme").expect("t");
        s.mlflow_experiments
            .write()
            .unwrap()
            .push(MlflowExperiment {
                tenant: acme.clone(),
                experiment_id: "exp-1".into(),
                name: "fraud-detection".into(),
                artifact_location: "s3://artifacts/fraud".into(),
                lifecycle_stage: "active".into(),
                creation_time_ms: 0,
                last_update_time_ms: 0,
            });
        s
    }

    #[test]
    fn render_refuses_without_permission() {
        let s = seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_nav_links() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        for link in [
            "/admin/mlflow/experiments",
            "/admin/mlflow/runs",
            "/admin/mlflow/models",
        ] {
            assert!(html.contains(link));
        }
    }

    #[test]
    fn render_shows_experiment_count() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert!(html.contains(">1<"));
    }
}
