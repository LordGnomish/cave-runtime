//! `/admin/artifacts` — Pulp Web UI parity. Image / file registry
//! browser grouped by upstream registry with size totals.
//!
//! Upstream UI: <https://pulpproject.org/>

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

pub fn group_by_registry(arts: &[ArtifactRecord]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in arts { *acc.entry(r.registry.clone()).or_insert(0) += 1; }
    acc.into_iter().collect()
}

pub fn total_size_bytes(arts: &[ArtifactRecord]) -> u64 {
    arts.iter().map(|a| a.size_bytes).sum()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ArtifactsViewError> {
    let arts = list_artifacts(state, ctx)?;
    let total = total_size_bytes(&arts);
    let regs = group_by_registry(&arts);
    let chips: String = regs.iter().map(|(r, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{r} <strong>×{n}</strong></span>"#,
        r = escape(r), n = n)).collect();
    let rows: Vec<Vec<String>> = arts.iter().map(|a| vec![
        a.registry.clone(), a.name.clone(), a.digest.clone(),
        format!("{} B", a.size_bytes), a.pushed_unix.to_string(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Pulp Web UI (cave-artifacts). Upstream: <a class="text-blue-700 underline" href="https://pulpproject.org/">pulpproject.org</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> artifacts</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{total}</strong> B total</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Artifacts ({n})</h2>{tbl}
</section>"#,
        n = arts.len(),
        total = total,
        chips = chips,
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
    fn group_by_registry_counts() {
        let a = list_artifacts(&AdminState::seeded(), &ctx(&[Permission::ArtifactsRead])).unwrap();
        let g = group_by_registry(&a);
        assert_eq!(g.iter().map(|(_, n)| n).sum::<usize>(), a.len());
    }

    #[test]
    fn total_size_bytes_sums() {
        let a = list_artifacts(&AdminState::seeded(), &ctx(&[Permission::ArtifactsRead])).unwrap();
        let expected: u64 = a.iter().map(|x| x.size_bytes).sum();
        assert_eq!(total_size_bytes(&a), expected);
    }

    #[test]
    fn render_includes_total_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ArtifactsRead])).unwrap();
        assert!(html.contains("B total"));
        assert!(html.contains("pulpproject.org"));
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
