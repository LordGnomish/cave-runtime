//! `/admin/pam` view — pam resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, PamSession};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PamViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<PamSession>, PamViewError> {
    ctx.authorise(Permission::PamRead)?;
    Ok(scope(&state.pam_sessions.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, PamViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.id.clone(), r.principal.clone(), r.target.clone(), r.started_unix.to_string(), r.ended_unix.map(|x| x.to_string()).unwrap_or_else(|| "open".into())]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Pam ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["id", "principal", "target", "started", "ended"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/pam", &format!("pam · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/pam/src/components/SessionsList.tsx", "SessionsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/pam/src/components/SessionsList.tsx", "SessionsList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::PamRead])).unwrap();
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
        let (_c, _t) = portal_test_ctx!("plugins/pam/src/components/SessionsList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PamRead])).unwrap();
        assert!(html.contains("sess-1"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/pam/src/components/SessionsList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PamRead])).unwrap();
        assert!(!html.contains("evil-sess"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/pam/src/components/SessionsList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PamRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
