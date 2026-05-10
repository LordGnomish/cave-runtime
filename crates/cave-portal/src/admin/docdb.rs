//! `/admin/docdb` view — document-store collection browser.
//!
//! Surfaces per-database collection sizes; mirrors the Backstage
//! `mongo-explorer` plugin without exposing per-document data.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, DocdbCollection};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DocdbViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_collections(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<DocdbCollection>, DocdbViewError> {
    ctx.authorise(Permission::DocdbRead)?;
    Ok(scope(&state.docdb_collections.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn collections_in(
    state: &AdminState,
    ctx: &RequestCtx,
    database: &str,
) -> Result<Vec<DocdbCollection>, DocdbViewError> {
    let all = list_collections(state, ctx)?;
    Ok(all.into_iter().filter(|c| c.database == database).collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, DocdbViewError> {
    let cols = list_collections(state, ctx)?;
    let rows: Vec<Vec<String>> = cols
        .iter()
        .map(|c| vec![c.database.clone(), c.collection.clone(), c.document_count.to_string()])
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Collections ({n})</h2>{tbl}</section>"#,
        n = cols.len(),
        tbl = table(&["database", "collection", "documents"], &rows),
    );
    Ok(page_shell(
        &format!("docdb · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/mongo-explorer/src/components/CollectionsList.tsx",
    "CollectionsList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_collections_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/mongo-explorer/src/components/CollectionsList.tsx",
            "CollectionsList",
            "acme"
        );
        let s = AdminState::seeded();
        let c = list_collections(&s, &ctx(&[Permission::DocdbRead])).unwrap();
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_collections_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(list_collections(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn collections_in_filters_by_database() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/mongo-explorer/src/components/DatabaseFilter.tsx",
            "DatabaseFilter",
            "acme"
        );
        let s = AdminState::seeded();
        let c = collections_in(&s, &ctx(&[Permission::DocdbRead]), "orders").unwrap();
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn collections_in_excludes_evil_database() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "tenantScopeGuard",
            "acme"
        );
        let s = AdminState::seeded();
        let c = collections_in(&s, &ctx(&[Permission::DocdbRead]), "secrets").unwrap();
        assert!(c.is_empty());
    }

    #[test]
    fn render_excludes_evil_database() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/mongo-explorer/src/components/CollectionsPage.tsx",
            "CollectionsPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::DocdbRead])).unwrap();
        assert!(html.contains("orders"));
        assert!(html.contains("items"));
        assert!(!html.contains("secrets"));
        assert!(!html.contains("tokens"));
    }
}
