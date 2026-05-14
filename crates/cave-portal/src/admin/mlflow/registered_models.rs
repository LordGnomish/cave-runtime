//! Registered models sub-page — per-model version registry.

use super::types::{MlflowViewError, ModelVersion};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

pub fn list_all(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ModelVersion>, MlflowViewError> {
    ctx.authorise(Permission::MlflowRead)?;
    let mut rows: Vec<ModelVersion> = scope(
        &state.mlflow_model_versions.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| {
        a.registered_model_name
            .cmp(&b.registered_model_name)
            .then(b.version.cmp(&a.version))
    });
    Ok(rows)
}

pub fn list_for_model(
    state: &AdminState,
    ctx: &RequestCtx,
    model_name: &str,
) -> Result<Vec<ModelVersion>, MlflowViewError> {
    Ok(list_all(state, ctx)?
        .into_iter()
        .filter(|v| v.registered_model_name == model_name)
        .collect())
}

pub fn latest_in_stage(rows: &[ModelVersion], stage: &str) -> Option<ModelVersion> {
    rows.iter()
        .filter(|v| v.current_stage == stage)
        .max_by_key(|v| v.version)
        .cloned()
}

pub fn stage_histogram(rows: &[ModelVersion]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.current_stage.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MlflowViewError> {
    let rows = list_all(state, ctx)?;
    let hist = stage_histogram(&rows);
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
        .map(|v| {
            vec![
                escape(&v.registered_model_name),
                v.version.to_string(),
                v.current_stage.clone(),
                escape(&v.source_run_id),
                v.status.clone(),
                v.creation_time_ms.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3">{chips}</div>{tbl}</section>"#,
        chips = chips,
        tbl = table(
            &["model", "version", "stage", "source_run", "status", "created"],
            &rows_html,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/mlflow/registered-models",
        &format!("mlflow/registered-models · {}", escape(ctx.tenant.as_str())),
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

    fn v(tenant: &str, model: &str, version: u32, stage: &str) -> ModelVersion {
        ModelVersion {
            tenant: TenantId::new(tenant).expect("t"),
            registered_model_name: model.into(),
            version,
            current_stage: stage.into(),
            source_run_id: format!("run-{model}-{version}"),
            creation_time_ms: i64::from(version) * 1000,
            status: "READY".into(),
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.mlflow_model_versions.write().unwrap();
        g.push(v("acme", "fraud-detector", 1, "Archived"));
        g.push(v("acme", "fraud-detector", 2, "Production"));
        g.push(v("acme", "fraud-detector", 3, "Staging"));
        g.push(v("acme", "churn-predictor", 1, "Production"));
        g.push(v("evil", "secret-model", 1, "Production"));
        drop(g);
        s
    }

    #[test]
    fn list_all_filters_by_tenant() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|v| v.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_all_refuses_without_perm() {
        let s = seeded();
        assert!(list_all(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_for_model_filters() {
        let s = seeded();
        let rows = list_for_model(&s, &ctx(&[Permission::MlflowRead]), "fraud-detector").unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn latest_in_production_returns_highest_version() {
        let s = seeded();
        let rows = list_for_model(&s, &ctx(&[Permission::MlflowRead]), "fraud-detector").unwrap();
        let prod = latest_in_stage(&rows, "Production").unwrap();
        assert_eq!(prod.version, 2);
    }

    #[test]
    fn latest_in_unknown_stage_returns_none() {
        let s = seeded();
        let rows = list_for_model(&s, &ctx(&[Permission::MlflowRead]), "fraud-detector").unwrap();
        assert!(latest_in_stage(&rows, "Sandbox").is_none());
    }

    #[test]
    fn stage_histogram_groups_by_stage() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        let h = stage_histogram(&rows);
        let prod = h.iter().find(|(s, _)| s == "Production").map(|(_, n)| *n).unwrap();
        assert_eq!(prod, 2);
    }

    #[test]
    fn render_includes_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        for col in ["model", "version", "stage", "status"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
        assert!(html.contains("Production"));
    }

    #[test]
    fn render_excludes_foreign_tenant() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert!(!html.contains("secret-model"));
    }
}
