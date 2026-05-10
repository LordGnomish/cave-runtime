//! `/admin/cloud-controller` view — managed-cloud volume browser.
//!
//! Surfaces the volumes the cloud-controller-manager has provisioned for
//! this tenant; mirrors the Backstage `CloudResources` panel.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, CloudVolume};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CloudControllerViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_volumes(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<CloudVolume>, CloudControllerViewError> {
    ctx.authorise(Permission::CloudControllerRead)?;
    Ok(scope(&state.cloud_volumes.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn unattached_volumes(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<CloudVolume>, CloudControllerViewError> {
    let all = list_volumes(state, ctx)?;
    Ok(all.into_iter().filter(|v| v.attached_node.is_none()).collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CloudControllerViewError> {
    let vols = list_volumes(state, ctx)?;
    let rows: Vec<Vec<String>> = vols
        .iter()
        .map(|v| {
            vec![
                v.id.clone(),
                v.region.clone(),
                format!("{} GB", v.size_gb),
                v.attached_node.clone().unwrap_or_else(|| "—".into()),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Volumes ({n})</h2>{tbl}</section>"#,
        n = vols.len(),
        tbl = table(&["id", "region", "size", "attached"], &rows),
    );
    Ok(page_shell(
        &format!("cloud-controller · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/CloudResources/Volumes.tsx",
    "VolumesList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_volumes_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/CloudResources/Volumes.tsx",
            "VolumesList",
            "acme"
        );
        let s = AdminState::seeded();
        let v = list_volumes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn unattached_volumes_returns_only_detached() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/CloudResources/UnattachedFilter.tsx",
            "UnattachedFilter",
            "acme"
        );
        let s = AdminState::seeded();
        let u = unattached_volumes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].id, "vol-2");
    }

    #[test]
    fn list_volumes_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(list_volumes(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_does_not_leak_evil_volume() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/CloudResources/VolumesPage.tsx",
            "VolumesPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        assert!(html.contains("vol-1"));
        assert!(!html.contains("evil-vol"));
    }

    #[test]
    fn render_size_uses_gb_suffix() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/CloudResources/SizeCell.tsx",
            "renderSize",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        assert!(html.contains("50 GB"));
    }
}
