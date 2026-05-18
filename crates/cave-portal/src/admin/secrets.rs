// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/secrets` view — secrets resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, SecretMetadata};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<SecretMetadata>, SecretsViewError> {
    ctx.authorise(Permission::SecretsBrowserRead)?;
    Ok(scope(&state.secret_metadatas.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SecretsViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.path.clone(), r.backend.clone(), r.version.to_string(), r.created_unix.to_string()]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Secrets ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["path", "backend", "version", "created"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/secrets", &format!("secrets · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/secrets/src/components/SecretsList.tsx", "SecretsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/secrets/src/components/SecretsList.tsx", "SecretsList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::SecretsBrowserRead])).unwrap();
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
        let (_c, _t) = portal_test_ctx!("plugins/secrets/src/components/SecretsList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SecretsBrowserRead])).unwrap();
        assert!(html.contains("app/db-password"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/secrets/src/components/SecretsList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SecretsBrowserRead])).unwrap();
        assert!(!html.contains("evil/secret"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/secrets/src/components/SecretsList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SecretsBrowserRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
