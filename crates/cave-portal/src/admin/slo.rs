//! `/admin/slo` view — SLO catalog + error-budget burn snapshot.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, Slo};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SloViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_slos(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<Slo>, SloViewError> {
    ctx.authorise(Permission::SloRead)?;
    Ok(scope(&state.slos.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn breaching_slos(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<Slo>, SloViewError> {
    Ok(list_slos(state, ctx)?.into_iter().filter(|s| s.error_budget_remaining_pct < 0.0).collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SloViewError> {
    let slos = list_slos(state, ctx)?;
    let rows: Vec<Vec<String>> = slos.iter().map(|s| vec![
        s.name.clone(), s.service.clone(),
        format!("{:.2}%", s.objective_pct),
        format!("{}d", s.window_days),
        format!("{:.2}%", s.current_pct),
        format!("{:+.1}%", s.error_budget_remaining_pct),
    ]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">SLOs ({n})</h2>{tbl}</section>"#,
        n = slos.len(),
        tbl = table(&["name", "service", "objective", "window", "current", "budget"], &rows),
    );
    Ok(page_shell_full(ctx, "/admin/slo", &format!("slo · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/slo/src/components/SloList.tsx", "SloList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/slo/src/components/SloList.tsx", "SloList", "acme");
        let s = AdminState::seeded();
        let l = list_slos(&s, &ctx(&[Permission::SloRead])).unwrap();
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_slos(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn breaching_slos_returns_negative_budget_only() {
        let (_c, _t) = portal_test_ctx!("plugins/slo/src/components/BurnBoard.tsx", "BurnBoard", "acme");
        let s = AdminState::seeded();
        let b = breaching_slos(&s, &ctx(&[Permission::SloRead])).unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].name, "api-latency-p99");
    }

    #[test]
    fn breaching_does_not_leak_evil_slo() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-backend/src/PermissionsService.ts", "tenantScopeGuard", "acme");
        let s = AdminState::seeded();
        let b = breaching_slos(&s, &ctx(&[Permission::SloRead])).unwrap();
        assert!(b.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn render_excludes_evil_slo() {
        let (_c, _t) = portal_test_ctx!("plugins/slo/src/components/SloPage.tsx", "SloPage", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SloRead])).unwrap();
        assert!(html.contains("SLOs (2)"));
        assert!(html.contains("web-availability"));
        assert!(!html.contains("evil-slo"));
    }
}
