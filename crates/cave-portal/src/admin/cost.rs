//! `/admin/cost` view — cost resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, CostReport};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CostViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<CostReport>, CostViewError> {
    ctx.authorise(Permission::CostRead)?;
    Ok(scope(&state.cost_reports.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CostViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.period.clone(), r.service.clone(), r.amount_cents.to_string()]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Cost ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["period", "service", "amount_cents"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/cost", &format!("cost · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/cost/src/components/ReportsList.tsx", "ReportsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/cost/src/components/ReportsList.tsx", "ReportsList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::CostRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_owner_row() {
        let (_c, _t) = portal_test_ctx!("plugins/cost/src/components/ReportsList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CostRead])).unwrap();
        assert!(html.contains("compute"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/cost/src/components/ReportsList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CostRead])).unwrap();
        assert!(!html.contains("evil"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/cost/src/components/ReportsList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CostRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
