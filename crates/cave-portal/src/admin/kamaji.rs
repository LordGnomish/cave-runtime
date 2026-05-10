//! `/admin/kamaji` view — TenantControlPlane (TCP) browser + scaler.
//!
//! Mirrors the Backstage `tenancy` plugin tab that surfaces hosted
//! control-plane status. `scale_tcp` adjusts desired_replicas; the
//! reconciler converges ready_replicas separately, so this view
//! exposes both.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, KamajiTcp};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KamajiViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("TCP {0} not found")]
    TcpNotFound(String),
    #[error("desired_replicas must be between 1 and 9")]
    InvalidReplicaCount,
}

pub fn list_tcps(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<KamajiTcp>, KamajiViewError> {
    ctx.authorise(Permission::KamajiRead)?;
    Ok(scope(&state.kamaji_tcps.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn scale_tcp(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
    desired: u32,
) -> Result<(), KamajiViewError> {
    ctx.authorise(Permission::KamajiWrite)?;
    if !(1..=9).contains(&desired) {
        return Err(KamajiViewError::InvalidReplicaCount);
    }
    let mut tcps = state.kamaji_tcps.write().unwrap();
    let target = tcps
        .iter_mut()
        .find(|t| t.tenant == ctx.tenant && t.name == name)
        .ok_or_else(|| KamajiViewError::TcpNotFound(name.into()))?;
    target.desired_replicas = desired;
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KamajiViewError> {
    let tcps = list_tcps(state, ctx)?;
    let rows: Vec<Vec<String>> = tcps
        .iter()
        .map(|t| {
            vec![
                t.name.clone(),
                t.k8s_version.clone(),
                format!("{}/{}", t.ready_replicas, t.desired_replicas),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">TenantControlPlanes ({n})</h2>{tbl}</section>"#,
        n = tcps.len(),
        tbl = table(&["name", "version", "ready/desired"], &rows),
    );
    Ok(page_shell(
        &format!("kamaji · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/tenancy/src/components/TenantControlPlanes.tsx",
    "TenantControlPlanes",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_tcps_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tenancy/src/components/TenantControlPlanes.tsx",
            "TcpList",
            "acme"
        );
        let s = AdminState::seeded();
        let t = list_tcps(&s, &ctx(&[Permission::KamajiRead])).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn scale_tcp_updates_desired_only() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tenancy/src/components/ScaleDialog.tsx",
            "scaleTcp",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::KamajiRead, Permission::KamajiWrite]);
        scale_tcp(&s, &c, "tcp-prod", 5).unwrap();
        let t = list_tcps(&s, &c).unwrap();
        let prod = t.iter().find(|x| x.name == "tcp-prod").unwrap();
        assert_eq!(prod.desired_replicas, 5);
        assert_eq!(prod.ready_replicas, 3); // not touched
    }

    #[test]
    fn scale_tcp_rejects_out_of_range() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tenancy/src/components/ScaleDialog.tsx",
            "validateReplicas",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::KamajiRead, Permission::KamajiWrite]);
        assert!(matches!(
            scale_tcp(&s, &c, "tcp-prod", 0).unwrap_err(),
            KamajiViewError::InvalidReplicaCount
        ));
        assert!(matches!(
            scale_tcp(&s, &c, "tcp-prod", 99).unwrap_err(),
            KamajiViewError::InvalidReplicaCount
        ));
    }

    #[test]
    fn scale_tcp_refuses_cross_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "tenantScopeGuard",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::KamajiRead, Permission::KamajiWrite]);
        assert!(matches!(
            scale_tcp(&s, &c, "evil-tcp", 1).unwrap_err(),
            KamajiViewError::TcpNotFound(_)
        ));
    }

    #[test]
    fn render_shows_ready_over_desired() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tenancy/src/components/TcpStatusCell.tsx",
            "renderStatus",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::KamajiRead])).unwrap();
        assert!(html.contains("3/3"));
        assert!(html.contains("2/3"));
        assert!(!html.contains("evil-tcp"));
    }
}
