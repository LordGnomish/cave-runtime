// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/uptime` view — uptime resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, UptimeProbe, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UptimeViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<UptimeProbe>, UptimeViewError> {
    ctx.authorise(Permission::UptimeRead)?;
    Ok(
        scope(&state.uptime_probes.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect(),
    )
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, UptimeViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.name.clone(),
                r.url.clone(),
                r.interval_seconds.to_string(),
                r.last_status.into(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Uptime ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["name", "url", "interval", "status"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/uptime",
        &format!("uptime · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/uptime/src/components/ProbesList.tsx", "ProbesList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/uptime/src/components/ProbesList.tsx",
            "ProbesList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::UptimeRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_owner_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/uptime/src/components/ProbesList.tsx",
            "RenderOwner",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::UptimeRead])).unwrap();
        assert!(html.contains("web-prod"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/uptime/src/components/ProbesList.tsx",
            "RenderEvil",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::UptimeRead])).unwrap();
        assert!(!html.contains("evil-probe"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/uptime/src/components/ProbesList.tsx",
            "Count",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::UptimeRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
