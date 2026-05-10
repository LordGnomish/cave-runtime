//! `/admin/artifacts` view — artifact / image registry browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, ArtifactRecord};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArtifactsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_artifacts(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ArtifactRecord>, ArtifactsViewError> {
    ctx.authorise(Permission::ArtifactsRead)?;
    Ok(scope(&state.artifact_records.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn artifacts_by_registry(state: &AdminState, ctx: &RequestCtx, registry_glob: &str) -> Result<Vec<ArtifactRecord>, ArtifactsViewError> {
    let all = list_artifacts(state, ctx)?;
    Ok(all.into_iter().filter(|a| a.registry.contains(registry_glob)).collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ArtifactsViewError> {
    let arts = list_artifacts(state, ctx)?;
    let rows: Vec<Vec<String>> = arts.iter().map(|a| vec![
        a.registry.clone(), a.name.clone(), a.digest.clone(),
        format!("{} B", a.size_bytes), a.pushed_unix.to_string(),
    ]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Artifacts ({n})</h2>{tbl}</section>"#,
        n = arts.len(),
        tbl = table(&["registry", "name", "digest", "size", "pushed"], &rows),
    );
    Ok(page_shell(&format!("artifacts · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/artifacts/src/components/ArtifactsList.tsx", "ArtifactsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/artifacts/src/components/ArtifactsList.tsx", "ArtifactsList", "acme");
        let s = AdminState::seeded();
        let a = list_artifacts(&s, &ctx(&[Permission::ArtifactsRead])).unwrap();
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_artifacts(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn artifacts_by_registry_filters() {
        let (_c, _t) = portal_test_ctx!("plugins/artifacts/src/components/RegistryFilter.tsx", "RegistryFilter", "acme");
        let s = AdminState::seeded();
        let a = artifacts_by_registry(&s, &ctx(&[Permission::ArtifactsRead]), "acme/web").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "web:v17");
    }

    #[test]
    fn artifacts_by_registry_excludes_evil() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-backend/src/PermissionsService.ts", "tenantScopeGuard", "acme");
        let s = AdminState::seeded();
        let a = artifacts_by_registry(&s, &ctx(&[Permission::ArtifactsRead]), "evil").unwrap();
        assert!(a.is_empty());
    }

    #[test]
    fn render_does_not_leak_evil() {
        let (_c, _t) = portal_test_ctx!("plugins/artifacts/src/components/ArtifactsPage.tsx", "ArtifactsPage", "acme");
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ArtifactsRead])).unwrap();
        assert!(html.contains("Artifacts (2)"));
        assert!(html.contains("registry.acme/web"));
        assert!(!html.contains("registry.evil"));
    }
}
