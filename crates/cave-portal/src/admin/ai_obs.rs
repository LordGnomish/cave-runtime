// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/ai-obs` view — ai-obs resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, AiModelMetric};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AiObsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<AiModelMetric>, AiObsViewError> {
    ctx.authorise(Permission::AiObsRead)?;
    Ok(scope(&state.ai_model_metrics.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AiObsViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.model.clone(), r.tokens_in.to_string(), r.tokens_out.to_string(), r.latency_p99_ms.to_string()]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Ai Obs ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["model", "tokens_in", "tokens_out", "p99_ms"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/ai-obs", &format!("ai-obs · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/ai-obs/src/components/ModelsList.tsx", "ModelsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/ai-obs/src/components/ModelsList.tsx", "ModelsList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::AiObsRead])).unwrap();
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
        let (_c, _t) = portal_test_ctx!("plugins/ai-obs/src/components/ModelsList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AiObsRead])).unwrap();
        assert!(html.contains("gpt-4"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/ai-obs/src/components/ModelsList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AiObsRead])).unwrap();
        assert!(!html.contains("evil-model"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/ai-obs/src/components/ModelsList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AiObsRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
