// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/embed` view — embedding-model registry browser.
//!
//! Renders the static cave-embed model catalogue (infinity-parity serving
//! surface): per-model dimensionality, context window, pooling strategy, and
//! normalization. The catalogue is global (not tenant-scoped), so the view
//! reads it straight from [`cave_embed::registry`].

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EmbedViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

/// One catalogue row: (id, dims, ctx, pooling, normalize).
pub fn list_models(ctx: &RequestCtx) -> Result<Vec<[String; 5]>, EmbedViewError> {
    ctx.authorise(Permission::EmbedRead)?;
    let reg = cave_embed::registry::ModelRegistry::with_builtins();
    Ok(reg
        .list_ids()
        .into_iter()
        .map(|id| {
            let c = reg.get(&id).expect("listed id resolves");
            [
                id,
                c.dimensions.to_string(),
                c.max_seq_len.to_string(),
                format!("{:?}", c.pooling),
                if c.normalize { "yes".into() } else { "no".into() },
            ]
        })
        .collect())
}

pub fn render(ctx: &RequestCtx) -> Result<String, EmbedViewError> {
    let rows = list_models(ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| r.to_vec()).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Embedding Models ({n})</h2><p class="text-sm text-gray-500 mb-3">infinity-parity registry — OpenAI /v1/embeddings + reranker.</p>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(
            &["model", "dims", "ctx", "pooling", "normalize"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/embed",
        &format!("embed · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("libs/infinity_emb/infinity_emb/primitives.py", "ModelCapabilities");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn lists_builtin_models() {
        let (_c, _t) = portal_test_ctx!(
            "libs/infinity_emb/infinity_emb/primitives.py",
            "ModelCapabilities",
            "acme"
        );
        let r = list_models(&ctx(&[Permission::EmbedRead])).unwrap();
        assert!(r.len() >= 7);
        assert!(r.iter().any(|row| row[0].contains("MiniLM")));
    }

    #[test]
    fn refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_models(&ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_model_row() {
        let (_c, _t) = portal_test_ctx!(
            "libs/infinity_emb/infinity_emb/primitives.py",
            "RenderModels",
            "acme"
        );
        let html = render(&ctx(&[Permission::EmbedRead])).unwrap();
        assert!(html.contains("all-MiniLM-L6-v2"));
        assert!(html.contains("Embedding Models"));
    }
}
