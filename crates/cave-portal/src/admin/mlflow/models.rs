//! Models sub-page — registered model list.

use super::types::{MlflowViewError, RegisteredModel};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<RegisteredModel>, MlflowViewError> {
    ctx.authorise(Permission::MlflowRead)?;
    let mut rows: Vec<RegisteredModel> =
        scope(&state.mlflow_models.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| b.last_updated_ms.cmp(&a.last_updated_ms).then(a.name.cmp(&b.name)));
    Ok(rows)
}

pub fn get(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<RegisteredModel, MlflowViewError> {
    list(state, ctx)?
        .into_iter()
        .find(|m| m.name == name)
        .ok_or_else(|| MlflowViewError::ModelNotFound(name.into()))
}

pub fn total_versions(rows: &[RegisteredModel]) -> u64 {
    rows.iter().map(|m| u64::from(m.latest_version)).sum()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MlflowViewError> {
    let rows = list(state, ctx)?;
    let total = total_versions(&rows);
    let rows_html: Vec<Vec<String>> = rows
        .iter()
        .map(|m| {
            vec![
                escape(&m.name),
                m.latest_version.to_string(),
                m.last_updated_ms.to_string(),
                escape(&m.description),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3 text-sm">total model versions across registry: <strong>{total}</strong></div>{tbl}</section>"#,
        total = total,
        tbl = table(&["name", "latest_version", "updated", "description"], &rows_html),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/mlflow/models",
        &format!("mlflow/models · {}", escape(ctx.tenant.as_str())),
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

    fn model(tenant: &str, name: &str, version: u32) -> RegisteredModel {
        RegisteredModel {
            tenant: TenantId::new(tenant).expect("t"),
            name: name.into(),
            creation_time_ms: 0,
            last_updated_ms: 0,
            description: format!("{name} description"),
            latest_version: version,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.mlflow_models.write().unwrap();
        g.push(model("acme", "fraud-detector", 5));
        g.push(model("acme", "churn-predictor", 2));
        g.push(model("evil", "secret-model", 99));
        drop(g);
        s
    }

    #[test]
    fn list_filters_by_tenant() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|m| m.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let s = seeded();
        assert!(list(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn get_returns_model_or_error() {
        let s = seeded();
        let c = ctx(&[Permission::MlflowRead]);
        assert_eq!(get(&s, &c, "fraud-detector").unwrap().latest_version, 5);
        assert!(matches!(get(&s, &c, "nope").unwrap_err(), MlflowViewError::ModelNotFound(_)));
    }

    #[test]
    fn total_versions_sums_per_model() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert_eq!(total_versions(&rows), 7);
    }

    #[test]
    fn render_includes_columns_and_total() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        for col in ["name", "latest_version", "updated"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
        assert!(html.contains("total model versions"));
    }

    #[test]
    fn render_excludes_foreign_tenant() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert!(!html.contains("secret-model"));
    }
}
