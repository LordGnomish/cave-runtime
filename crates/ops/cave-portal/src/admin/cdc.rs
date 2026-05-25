// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/cdc` view — cdc resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, CdcPipeline, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CdcViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<CdcPipeline>, CdcViewError> {
    ctx.authorise(Permission::CdcRead)?;
    Ok(
        scope(&state.cdc_pipelines.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect(),
    )
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CdcViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.name.clone(),
                r.source.clone(),
                r.sink.clone(),
                r.state.into(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Cdc ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["name", "source", "sink", "state"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/cdc",
        &format!("cdc · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/cdc/src/components/PipelinesList.tsx",
    "PipelinesList",
);

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
            "plugins/cdc/src/components/PipelinesList.tsx",
            "PipelinesList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::CdcRead])).unwrap();
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
            "plugins/cdc/src/components/PipelinesList.tsx",
            "RenderOwner",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CdcRead])).unwrap();
        assert!(html.contains("orders-cdc"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/cdc/src/components/PipelinesList.tsx",
            "RenderEvil",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CdcRead])).unwrap();
        assert!(!html.contains("evil-cdc"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/cdc/src/components/PipelinesList.tsx",
            "Count",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CdcRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
