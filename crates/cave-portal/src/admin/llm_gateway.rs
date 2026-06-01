// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/llm-gateway` view — llm-gateway resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, LlmRoute, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LlmGatewayViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<LlmRoute>, LlmGatewayViewError> {
    ctx.authorise(Permission::LlmGatewayRead)?;
    Ok(scope(&state.llm_routes.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LlmGatewayViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.name.clone(),
                r.upstream.clone(),
                r.rpm_limit.to_string(),
                r.daily_tokens.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Llm Gateway ({n})</h2>{tbl}</section>
<section class="mt-4"><h3 class="text-md font-semibold mb-1">Spend budgets</h3>
<p class="text-sm text-gray-500">Per-consumer USD spend limits (LiteLLM BudgetManager). The live gateway rejects over-budget requests with <code>402</code>.</p>
<ul class="text-sm list-disc ml-5">
<li><code>GET /api/gateway/budgets</code> — list every tracked consumer budget</li>
<li><code>POST /api/gateway/budgets</code> — <code>{{user, total_budget, duration?}}</code> (daily/weekly/monthly/yearly)</li>
<li><code>GET /api/gateway/budgets/:user</code> — one consumer's ledger</li>
<li><code>POST /api/gateway/budgets/:user/reset</code> — clear accrued spend</li>
</ul></section>
<section class="mt-4"><h3 class="text-md font-semibold mb-1">OpenAI-compatible endpoints</h3>
<ul class="text-sm list-disc ml-5">
<li><code>POST /v1/chat/completions</code> — chat completions across every provider</li>
<li><code>POST /v1/embeddings</code> — embeddings (OpenAI + local-compat backends)</li>
<li><code>POST /v1/rerank</code> — Cohere/Jina-shaped rerank, scored in-process with a BM25 lexical cross-encoder (<code>top_n</code> + <code>return_documents</code>)</li>
</ul>
<p class="text-sm text-gray-500 mt-1">SaaS providers (OpenAI-compatible): OpenAI · Anthropic · Mistral · Groq · DeepSeek · Together AI · Fireworks AI. Local: Ollama · llama.cpp · MLX.</p>
</section>"#,
        n = rows.len(),
        tbl = table(
            &["name", "upstream", "rpm_limit", "daily_tokens"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/llm-gateway",
        &format!("llm-gateway · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/llm-gateway/src/components/RoutesList.tsx",
    "RoutesList",
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
            "plugins/llm-gateway/src/components/RoutesList.tsx",
            "RoutesList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::LlmGatewayRead])).unwrap();
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
            "plugins/llm-gateway/src/components/RoutesList.tsx",
            "RenderOwner",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LlmGatewayRead])).unwrap();
        assert!(html.contains("claude-proxy"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/llm-gateway/src/components/RoutesList.tsx",
            "RenderEvil",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LlmGatewayRead])).unwrap();
        assert!(!html.contains("evil-route"));
    }

    #[test]
    fn render_lists_rerank_endpoint_and_new_providers() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/llm-gateway/src/components/RoutesList.tsx",
            "Rerank",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LlmGatewayRead])).unwrap();
        assert!(html.contains("/v1/rerank"), "rerank endpoint advertised");
        assert!(html.contains("Together AI"), "Together AI provider listed");
        assert!(html.contains("Fireworks AI"), "Fireworks AI provider listed");
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/llm-gateway/src/components/RoutesList.tsx",
            "Count",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LlmGatewayRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
