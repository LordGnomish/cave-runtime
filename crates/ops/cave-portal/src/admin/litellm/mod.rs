// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/litellm` — LiteLLM gateway admin views.

pub mod api_keys;
pub mod budgets;
pub mod models;
pub mod monitoring;
pub mod routes;
pub mod types;

pub use types::{
    LiteLlmApiKey, LiteLlmBudget, LiteLlmModel, LiteLlmRoute, LiteLlmTraffic, LiteLlmViewError,
};

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LiteLlmViewError> {
    ctx.authorise(Permission::LiteLlmRead)?;
    let m = models::list(state, ctx)?.len();
    let r = routes::list(state, ctx)?.len();
    let k = api_keys::list_active(state, ctx)?.len();
    let b = budgets::list(state, ctx)?.len();
    let body = format!(
        r#"<section class="grid grid-cols-4 gap-3 mb-4">
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">models</div><div class="text-2xl font-bold">{m}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">routes</div><div class="text-2xl font-bold">{r}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">active keys</div><div class="text-2xl font-bold">{k}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">budgets</div><div class="text-2xl font-bold">{b}</div></div>
</section>
<nav class="flex gap-4 mb-3 text-sm">
  <a class="text-blue-700 underline" href="/admin/litellm/models?tenant_id={tid}">models</a>
  <a class="text-blue-700 underline" href="/admin/litellm/routes?tenant_id={tid}">routes</a>
  <a class="text-blue-700 underline" href="/admin/litellm/api-keys?tenant_id={tid}">api keys</a>
  <a class="text-blue-700 underline" href="/admin/litellm/budgets?tenant_id={tid}">budgets</a>
  <a class="text-blue-700 underline" href="/admin/litellm/monitoring?tenant_id={tid}">monitoring</a>
</nav>"#,
        m = m,
        r = r,
        k = k,
        b = b,
        tid = escape(ctx.tenant.as_str()),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/litellm",
        &format!("litellm · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_nav_links() {
        let s = AdminState::seeded();
        s.litellm_models.write().unwrap().push(LiteLlmModel {
            tenant: TenantId::new("acme").unwrap(),
            name: "gpt-4o".into(),
            provider: "openai".into(),
            model_id: "gpt-4o-2024-08-06".into(),
            status: "active".into(),
            rpm_limit: 1000,
            tpm_limit: 1_000_000,
            fallback_chain: vec![],
            created_at_unix: 0,
        });
        let html = render(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        for link in [
            "/admin/litellm/models",
            "/admin/litellm/routes",
            "/admin/litellm/api-keys",
            "/admin/litellm/budgets",
            "/admin/litellm/monitoring",
        ] {
            assert!(html.contains(link), "missing nav link {link}");
        }
    }
}
