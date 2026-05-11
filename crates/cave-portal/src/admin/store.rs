//! `/admin/store` view — store resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, StoreBucket};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<StoreBucket>, StoreViewError> {
    ctx.authorise(Permission::StoreRead)?;
    Ok(scope(&state.store_buckets.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StoreViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.name.clone(), r.backend.clone(), r.object_count.to_string(), r.size_bytes.to_string()]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Store ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["name", "backend", "objects", "size_bytes"], &table_rows),
    );
    Ok(page_shell(&format!("store · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/store/src/components/BucketsList.tsx", "BucketsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/store/src/components/BucketsList.tsx", "BucketsList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::StoreRead])).unwrap();
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
        let (_c, _t) = portal_test_ctx!("plugins/store/src/components/BucketsList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StoreRead])).unwrap();
        assert!(html.contains("prod-images"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/store/src/components/BucketsList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StoreRead])).unwrap();
        assert!(!html.contains("evil-bucket"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/store/src/components/BucketsList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StoreRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
