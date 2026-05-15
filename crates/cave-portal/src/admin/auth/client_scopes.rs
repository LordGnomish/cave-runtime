// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/client-scopes/{realm}` — Keycloak Admin "Client scopes".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/admin-ui/src/client-scopes/ClientScopesSection.tsx`.

use super::fixtures::{self, ClientScope};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::metrics::PortalMetrics;
use super::AuthViewError;

pub fn list(_state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<Vec<ClientScope>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    Ok(fixtures::client_scopes(realm))
}

pub fn detail(state: &AdminState, ctx: &RequestCtx, realm: &str, name: &str) -> Result<Option<ClientScope>, AuthViewError> {
    Ok(list(state, ctx, realm)?.into_iter().find(|s| s.name == name))
}

pub fn render_list(state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<String, AuthViewError> {
    let scopes = list(state, ctx, realm)?;
    PortalMetrics::global().incr_page_view("admin_auth_client_scopes", ctx.persona.as_str());
    let mut rows = String::new();
    for s in &scopes {
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2 font-medium"><a class="text-blue-700 underline" href="/admin/auth/client-scopes/{realm}/{name}">{name}</a></td>
  <td class="px-3 py-2"><code class="text-xs">{proto}</code></td>
  <td class="px-3 py-2">{desc}</td>
  <td class="px-3 py-2">{mappers}</td>
  <td class="px-3 py-2">{token_scope}</td>
</tr>"#,
            realm = escape(realm),
            name = escape(&s.name),
            proto = escape(&s.protocol),
            desc = escape(&s.description),
            mappers = s.mappers.len(),
            token_scope = if s.include_in_token_scope { "yes" } else { "no" },
        ));
    }
    let body = format!(
        r#"<section>
  <div class="flex justify-between items-center mb-3">
    <h2 class="text-lg font-semibold">Client scopes ({n})</h2>
    <form method="post" action="/admin/auth/client-scopes/{realm}/new" class="inline">
      <button class="px-3 py-2 rounded bg-blue-600 text-white">+ New scope</button>
    </form>
  </div>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Name</th>
      <th class="px-3 py-2 text-left">Protocol</th>
      <th class="px-3 py-2 text-left">Description</th>
      <th class="px-3 py-2 text-left">Mappers</th>
      <th class="px-3 py-2 text-left">Token scope</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</section>"#,
        n = scopes.len(),
        realm = escape(realm),
        rows = rows,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/client-scopes",
        &format!("auth/client-scopes · {}", escape(realm)),
        &body,
    ))
}

pub fn render_detail(state: &AdminState, ctx: &RequestCtx, realm: &str, name: &str) -> Result<String, AuthViewError> {
    let scope = match detail(state, ctx, realm, name)? {
        Some(s) => s,
        None => {
            return Ok(page_shell_full(
                ctx,
                "/admin/auth/client-scopes",
                &format!("auth/client-scopes · {} · {}", escape(realm), escape(name)),
                &format!(
                    r#"<p class="text-red-700">No client scope <code>{}</code> in realm <code>{}</code>.</p>"#,
                    escape(name),
                    escape(realm)
                ),
            ));
        }
    };
    PortalMetrics::global()
        .incr_page_view("admin_auth_client_scope_detail", ctx.persona.as_str());
    let mut mappers = String::new();
    for m in &scope.mappers {
        mappers.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2">{n}</td>
  <td class="px-3 py-2"><code class="text-xs">{k}</code></td>
  <td class="px-3 py-2"><code class="text-xs">{c}</code></td>
  <td class="px-3 py-2">
    <form method="post" action="/admin/auth/client-scopes/{realm}/{name}/mappers/{n}/delete"
          class="inline" onsubmit="return confirm('Delete mapper {n_js}?');">
      <button class="text-red-700 text-sm hover:underline">Delete</button>
    </form>
  </td>
</tr>"#,
            n = escape(&m.name),
            n_js = escape(&m.name).replace('\'', "\\'"),
            k = escape(&m.kind),
            c = escape(&m.claim_name),
            realm = escape(realm),
            name = escape(&scope.name),
        ));
    }
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Settings</h2>
  <form method="post" action="/admin/auth/client-scopes/{realm}/{name}" class="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm mb-6">
    <label>Name <input name="name" value="{name}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
    <label>Protocol <input name="protocol" value="{proto}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
    <label class="md:col-span-2">Description <input name="description" value="{desc}" class="ml-2 px-2 py-1 border rounded w-full dark:bg-zinc-900 dark:border-zinc-700"></label>
    <label class="flex items-center gap-2"><input type="checkbox" name="include_in_token_scope" {its}>Include in token scope</label>
    <div class="md:col-span-2"><button class="px-3 py-2 rounded bg-blue-600 text-white">Save</button></div>
  </form>
  <h2 class="text-lg font-semibold mb-2">Mappers ({nm})</h2>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Name</th>
      <th class="px-3 py-2 text-left">Type</th>
      <th class="px-3 py-2 text-left">Claim name</th>
      <th class="px-3 py-2 text-left">Actions</th>
    </tr></thead>
    <tbody>{mappers}</tbody>
  </table>
</section>"#,
        realm = escape(realm),
        name = escape(&scope.name),
        proto = escape(&scope.protocol),
        desc = escape(&scope.description),
        its = if scope.include_in_token_scope { "checked" } else { "" },
        nm = scope.mappers.len(),
        mappers = mappers,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/client-scopes",
        &format!("auth/client-scopes · {} · {}", escape(realm), escape(&scope.name)),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RequestCtx {
        RequestCtx::developer("acme", &[Permission::AuthSessionsRead])
    }

    #[test]
    fn list_requires_permission() {
        let s = AdminState::seeded();
        let bad = RequestCtx::developer("acme", &[]);
        assert!(list(&s, &bad, "acme-realm").is_err());
    }

    #[test]
    fn list_returns_seeded_scopes() {
        let s = AdminState::seeded();
        let r = list(&s, &ctx(), "acme-realm").unwrap();
        assert!(r.iter().any(|x| x.name == "openid"));
        assert!(r.iter().any(|x| x.name == "offline_access"));
    }

    #[test]
    fn detail_returns_some_for_known_and_none_for_unknown() {
        let s = AdminState::seeded();
        assert!(detail(&s, &ctx(), "acme-realm", "openid").unwrap().is_some());
        assert!(detail(&s, &ctx(), "acme-realm", "nope").unwrap().is_none());
    }

    #[test]
    fn render_list_includes_new_scope_button_and_table_rows() {
        let s = AdminState::seeded();
        let html = render_list(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains("+ New scope"));
        assert!(html.contains("openid"));
        assert!(html.contains("offline_access"));
    }

    #[test]
    fn render_detail_includes_settings_and_mapper_table() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "profile").unwrap();
        assert!(html.contains("Settings"));
        assert!(html.contains("Mappers ("));
        assert!(html.contains("given_name"));
        assert!(html.contains("family_name"));
    }

    #[test]
    fn render_detail_unknown_scope_renders_404_text() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "nope").unwrap();
        assert!(html.contains("No client scope"));
    }
}
