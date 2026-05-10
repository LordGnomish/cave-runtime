//! `/admin/controller-manager` view — leader-election lease browser.
//!
//! Mirrors the Backstage `kube-system/Lease` widget that surfaces which
//! controller currently holds the loop and how often it has renewed.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, ControllerLease};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ControllerManagerViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_leases(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ControllerLease>, ControllerManagerViewError> {
    ctx.authorise(Permission::ControllerManagerRead)?;
    let mut rows: Vec<ControllerLease> =
        scope(&state.controller_leases.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.controller.cmp(&b.controller));
    Ok(rows)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ControllerManagerViewError> {
    let leases = list_leases(state, ctx)?;
    let rows: Vec<Vec<String>> = leases
        .iter()
        .map(|l| {
            vec![
                l.controller.clone(),
                l.leader_id.clone(),
                l.renewals.to_string(),
                l.expires_unix.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Leases ({n})</h2>{tbl}</section>"#,
        n = leases.len(),
        tbl = table(&["controller", "leader", "renewals", "expires_unix"], &rows),
    );
    Ok(page_shell(
        &format!("controller-manager · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/Resources/Leases.tsx",
    "LeaseList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_leases_filters_to_owner_and_sorts() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/Leases.tsx",
            "LeaseList",
            "acme"
        );
        let s = AdminState::seeded();
        let l = list_leases(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].controller, "deployment");
        assert_eq!(l[1].controller, "replicaset");
        assert!(l.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_leases_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(list_leases(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_leases_excludes_evil_controllers() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/LeasesPage.tsx",
            "tenantFilter",
            "acme"
        );
        let s = AdminState::seeded();
        let l = list_leases(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        assert!(!l.iter().any(|x| x.controller == "evil-loop"));
    }

    #[test]
    fn render_includes_owner_leases_and_excludes_others() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/LeasesPage.tsx",
            "LeasesPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        assert!(html.contains("Leases (2)"));
        assert!(html.contains("deployment"));
        assert!(!html.contains("evil-loop"));
    }

    #[test]
    fn render_returns_error_when_unauthorised() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/components/PermissionedRoute.tsx",
            "PermissionedRoute",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }
}
