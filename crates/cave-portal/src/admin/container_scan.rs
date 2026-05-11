//! `/admin/container-scan` view — container-scan resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, ContainerScanResult};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContainerScanViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ContainerScanResult>, ContainerScanViewError> {
    ctx.authorise(Permission::ContainerScanRead)?;
    Ok(scope(&state.container_scan_results.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ContainerScanViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.image.clone(), r.digest.clone(), r.critical_cves.to_string(), r.scanned_at_unix.to_string()]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Container Scan ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["image", "digest", "critical_cves", "scanned_at"], &table_rows),
    );
    Ok(page_shell(&format!("container-scan · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/container-scan/src/components/ResultsList.tsx", "ResultsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/container-scan/src/components/ResultsList.tsx", "ResultsList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::ContainerScanRead])).unwrap();
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
        let (_c, _t) = portal_test_ctx!("plugins/container-scan/src/components/ResultsList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(html.contains("web:v17"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/container-scan/src/components/ResultsList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(!html.contains("evil:latest"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/container-scan/src/components/ResultsList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
