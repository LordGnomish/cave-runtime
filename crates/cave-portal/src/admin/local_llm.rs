//! `/admin/local-llm` view — local-llm resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, LocalLlmModel};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LocalLlmViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LocalLlmModel>, LocalLlmViewError> {
    ctx.authorise(Permission::LocalLlmRead)?;
    Ok(scope(&state.local_llm_models.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LocalLlmViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.tag.clone(), r.size_bytes.to_string(), r.quant.clone(), if r.loaded { "yes".into() } else { "no".into() }]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Local Llm ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["tag", "size_bytes", "quant", "loaded"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/local-llm", &format!("local-llm · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/local-llm/src/components/ModelsList.tsx", "ModelsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/local-llm/src/components/ModelsList.tsx", "ModelsList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::LocalLlmRead])).unwrap();
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
        let (_c, _t) = portal_test_ctx!("plugins/local-llm/src/components/ModelsList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LocalLlmRead])).unwrap();
        assert!(html.contains("qwen3.6:35b-a3b-coding-mxfp8"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/local-llm/src/components/ModelsList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LocalLlmRead])).unwrap();
        assert!(!html.contains("evil-model"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/local-llm/src/components/ModelsList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LocalLlmRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
