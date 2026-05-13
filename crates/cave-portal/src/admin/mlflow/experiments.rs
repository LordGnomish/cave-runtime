//! Experiments sub-page.

use super::types::{MlflowExperiment, MlflowViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<MlflowExperiment>, MlflowViewError> {
    ctx.authorise(Permission::MlflowRead)?;
    let mut rows: Vec<MlflowExperiment> =
        scope(&state.mlflow_experiments.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| b.last_update_time_ms.cmp(&a.last_update_time_ms).then(a.name.cmp(&b.name)));
    Ok(rows)
}

pub fn list_active(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<MlflowExperiment>, MlflowViewError> {
    Ok(list(state, ctx)?.into_iter().filter(|e| e.lifecycle_stage == "active").collect())
}

pub fn get(
    state: &AdminState,
    ctx: &RequestCtx,
    experiment_id: &str,
) -> Result<MlflowExperiment, MlflowViewError> {
    list(state, ctx)?
        .into_iter()
        .find(|e| e.experiment_id == experiment_id)
        .ok_or_else(|| MlflowViewError::ExperimentNotFound(experiment_id.into()))
}

pub fn lifecycle_histogram(rows: &[MlflowExperiment]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.lifecycle_stage.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MlflowViewError> {
    let rows = list(state, ctx)?;
    let hist = lifecycle_histogram(&rows);
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
        .map(|e| {
            vec![
                escape(&e.experiment_id),
                escape(&e.name),
                e.lifecycle_stage.clone(),
                escape(&e.artifact_location),
                e.last_update_time_ms.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3">{chips}</div>{tbl}</section>"#,
        chips = chips,
        tbl = table(&["id", "name", "stage", "artifact", "updated"], &rows_html),
    );
    Ok(page_shell(
        &format!("mlflow/experiments · {}", escape(ctx.tenant.as_str())),
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

    fn exp(tenant: &str, id: &str, name: &str, stage: &str) -> MlflowExperiment {
        MlflowExperiment {
            tenant: TenantId::new(tenant).expect("t"),
            experiment_id: id.into(),
            name: name.into(),
            artifact_location: format!("s3://{id}"),
            lifecycle_stage: stage.into(),
            creation_time_ms: 0,
            last_update_time_ms: 0,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.mlflow_experiments.write().unwrap();
        g.push(exp("acme", "exp-1", "fraud", "active"));
        g.push(exp("acme", "exp-2", "churn", "active"));
        g.push(exp("acme", "exp-3", "old-model", "deleted"));
        g.push(exp("evil", "exp-9", "secret", "active"));
        drop(g);
        s
    }

    #[test]
    fn list_filters_by_tenant() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|e| e.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let s = seeded();
        assert!(list(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_active_excludes_deleted() {
        let s = seeded();
        let rows = list_active(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|e| e.lifecycle_stage == "active"));
    }

    #[test]
    fn get_returns_experiment_or_error() {
        let s = seeded();
        let c = ctx(&[Permission::MlflowRead]);
        assert_eq!(get(&s, &c, "exp-1").unwrap().name, "fraud");
        assert!(matches!(get(&s, &c, "nope").unwrap_err(), MlflowViewError::ExperimentNotFound(_)));
    }

    #[test]
    fn lifecycle_histogram_counts() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        let h = lifecycle_histogram(&rows);
        let active = h.iter().find(|(s, _)| s == "active").map(|(_, n)| *n).unwrap();
        assert_eq!(active, 2);
    }

    #[test]
    fn render_includes_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        for col in ["id", "name", "stage"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
        assert!(html.contains("active"));
    }

    #[test]
    fn render_excludes_foreign_tenant() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert!(!html.contains("secret"));
    }
}
