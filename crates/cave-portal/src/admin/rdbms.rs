//! `/admin/rdbms` view — operator-managed RDBMS cluster browser.
//!
//! Shows the per-tenant Postgres-flavour clusters the rdbms operator
//! manages, including the elected primary and replica count.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, RdbmsCluster};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RdbmsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("cluster {0} not found")]
    ClusterNotFound(String),
}

pub fn list_clusters(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<RdbmsCluster>, RdbmsViewError> {
    ctx.authorise(Permission::RdbmsRead)?;
    Ok(scope(&state.rdbms_clusters.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn inspect_cluster(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<RdbmsCluster, RdbmsViewError> {
    let clusters = list_clusters(state, ctx)?;
    clusters
        .into_iter()
        .find(|c| c.name == name)
        .ok_or_else(|| RdbmsViewError::ClusterNotFound(name.into()))
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, RdbmsViewError> {
    let clusters = list_clusters(state, ctx)?;
    let rows: Vec<Vec<String>> = clusters
        .iter()
        .map(|c| {
            vec![
                c.name.clone(),
                c.version.clone(),
                c.replicas.to_string(),
                c.primary_node.clone(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">RDBMS clusters ({n})</h2>{tbl}</section>"#,
        n = clusters.len(),
        tbl = table(&["name", "version", "replicas", "primary"], &rows),
    );
    Ok(page_shell(
        &format!("rdbms · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/database/src/components/PostgresClusters/PostgresClustersList.tsx",
    "PostgresClustersList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_clusters_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/PostgresClusters/PostgresClustersList.tsx",
            "PostgresClustersList",
            "acme"
        );
        let s = AdminState::seeded();
        let c = list_clusters(&s, &ctx(&[Permission::RdbmsRead])).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "pg-prod");
    }

    #[test]
    fn list_clusters_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(list_clusters(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn inspect_cluster_returns_owner_record() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/PostgresClusters/ClusterDetail.tsx",
            "ClusterDetail",
            "acme"
        );
        let s = AdminState::seeded();
        let c = inspect_cluster(&s, &ctx(&[Permission::RdbmsRead]), "pg-prod").unwrap();
        assert_eq!(c.primary_node, "node-a");
    }

    #[test]
    fn inspect_cluster_refuses_cross_tenant_lookup() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "tenantScopeGuard",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(matches!(
            inspect_cluster(&s, &ctx(&[Permission::RdbmsRead]), "evil-pg").unwrap_err(),
            RdbmsViewError::ClusterNotFound(_)
        ));
    }

    #[test]
    fn render_excludes_evil_cluster() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/PostgresClusters/ClustersPage.tsx",
            "ClustersPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::RdbmsRead])).unwrap();
        assert!(html.contains("pg-prod"));
        assert!(!html.contains("evil-pg"));
    }
}
