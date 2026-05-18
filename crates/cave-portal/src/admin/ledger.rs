// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/ledger` view — ledger resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, LedgerEntry};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LedgerViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LedgerEntry>, LedgerViewError> {
    ctx.authorise(Permission::LedgerRead)?;
    Ok(scope(&state.ledger_entries.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LedgerViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.id.clone(), r.actor.clone(), r.action.clone(), r.at_unix.to_string()]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Ledger ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["id", "actor", "action", "at"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/ledger", &format!("ledger · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/ledger/src/components/EntriesList.tsx", "EntriesList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/ledger/src/components/EntriesList.tsx", "EntriesList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::LedgerRead])).unwrap();
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
        let (_c, _t) = portal_test_ctx!("plugins/ledger/src/components/EntriesList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LedgerRead])).unwrap();
        assert!(html.contains("led-1"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/ledger/src/components/EntriesList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LedgerRead])).unwrap();
        assert!(!html.contains("evil-led"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/ledger/src/components/EntriesList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LedgerRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
