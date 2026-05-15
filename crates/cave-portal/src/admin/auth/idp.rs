// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/idp/{realm}` — Keycloak Admin "Identity providers".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/admin-ui/src/identity-providers/IdentityProvidersSection.tsx`.

use super::fixtures::{self, IdentityProvider};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::metrics::PortalMetrics;
use super::AuthViewError;

pub fn list(_state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<Vec<IdentityProvider>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    Ok(fixtures::identity_providers(realm))
}

pub fn detail(state: &AdminState, ctx: &RequestCtx, realm: &str, alias: &str) -> Result<Option<IdentityProvider>, AuthViewError> {
    Ok(list(state, ctx, realm)?.into_iter().find(|p| p.alias == alias))
}

pub fn render_list(state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<String, AuthViewError> {
    let providers = list(state, ctx, realm)?;
    PortalMetrics::global().incr_page_view("admin_auth_idp", ctx.persona.as_str());
    let mut rows = String::new();
    for p in &providers {
        let badge = if p.enabled {
            r#"<span class="px-2 py-0.5 rounded bg-green-100 dark:bg-green-900/30 text-green-900 dark:text-green-200 text-xs">enabled</span>"#
        } else {
            r#"<span class="px-2 py-0.5 rounded bg-zinc-200 dark:bg-zinc-700 text-zinc-700 dark:text-zinc-200 text-xs">disabled</span>"#
        };
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2 font-medium"><a class="text-blue-700 underline" href="/admin/auth/idp/{realm}/{a}">{name}</a></td>
  <td class="px-3 py-2"><code class="text-xs">{a}</code></td>
  <td class="px-3 py-2"><code class="text-xs">{pid}</code></td>
  <td class="px-3 py-2">{badge}</td>
  <td class="px-3 py-2">
    <form method="post" action="/admin/auth/idp/{realm}/{a}/delete" class="inline" onsubmit="return confirm('Delete provider {ajs}?');">
      <button class="text-red-700 text-sm hover:underline">Delete</button>
    </form>
  </td>
</tr>"#,
            realm = escape(realm),
            a = escape(&p.alias),
            ajs = escape(&p.alias).replace('\'', "\\'"),
            name = escape(&p.display_name),
            pid = escape(&p.provider_id),
            badge = badge,
        ));
    }
    let body = format!(
        r#"<section>
  <div class="flex justify-between items-center mb-3">
    <h2 class="text-lg font-semibold">Identity providers ({n})</h2>
    <div class="flex gap-2">
      <form method="post" action="/admin/auth/idp/{realm}/new/oidc" class="inline">
        <button class="px-3 py-2 rounded border hover:bg-zinc-50 dark:hover:bg-zinc-800">+ Add OIDC provider</button>
      </form>
      <form method="post" action="/admin/auth/idp/{realm}/new/saml" class="inline">
        <button class="px-3 py-2 rounded border hover:bg-zinc-50 dark:hover:bg-zinc-800">+ Add SAML provider</button>
      </form>
      <form method="post" action="/admin/auth/idp/{realm}/discover" class="inline">
        <input type="text" name="discovery_url" placeholder="https://example.com/.well-known/openid-configuration"
               class="px-2 py-2 border rounded dark:bg-zinc-900 dark:border-zinc-700 w-80">
        <button class="px-3 py-2 rounded bg-blue-600 text-white">Discover</button>
      </form>
    </div>
  </div>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Name</th>
      <th class="px-3 py-2 text-left">Alias</th>
      <th class="px-3 py-2 text-left">Provider</th>
      <th class="px-3 py-2 text-left">Status</th>
      <th class="px-3 py-2 text-left">Actions</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</section>"#,
        realm = escape(realm),
        n = providers.len(),
        rows = rows,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/idp",
        &format!("auth/idp · {}", escape(realm)),
        &body,
    ))
}

pub fn render_detail(state: &AdminState, ctx: &RequestCtx, realm: &str, alias: &str) -> Result<String, AuthViewError> {
    let p = match detail(state, ctx, realm, alias)? {
        Some(p) => p,
        None => {
            return Ok(page_shell_full(
                ctx,
                "/admin/auth/idp",
                &format!("auth/idp · {} · {}", escape(realm), escape(alias)),
                &format!(r#"<p class="text-red-700">No identity provider <code>{}</code>.</p>"#, escape(alias)),
            ));
        }
    };
    PortalMetrics::global().incr_page_view("admin_auth_idp_detail", ctx.persona.as_str());
    let body = format!(
        r#"<form method="post" action="/admin/auth/idp/{realm}/{a}" class="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
  <label>Alias <input name="alias" value="{a}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
  <label>Display name <input name="display_name" value="{name}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
  <label>Provider id <input name="provider_id" value="{pid}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
  <label class="flex items-center gap-2"><input type="checkbox" name="enabled" {en}>Enabled</label>
  <label class="flex items-center gap-2"><input type="checkbox" name="trust_email" {te}>Trust email</label>
  <label class="flex items-center gap-2"><input type="checkbox" name="store_token" {st}>Store tokens</label>
  <label class="flex items-center gap-2"><input type="checkbox" name="link_only" {lo}>Link only (no new accounts)</label>
  <div class="md:col-span-2 mt-2">
    <button class="px-3 py-2 rounded bg-blue-600 text-white">Save</button>
    <a href="/admin/auth/idp/{realm}/{a}/mappers" class="px-3 py-2 ml-2 border rounded hover:bg-zinc-50 dark:hover:bg-zinc-800">Mappers</a>
  </div>
</form>"#,
        realm = escape(realm),
        a = escape(&p.alias),
        name = escape(&p.display_name),
        pid = escape(&p.provider_id),
        en = if p.enabled { "checked" } else { "" },
        te = if p.trust_email { "checked" } else { "" },
        st = if p.store_token { "checked" } else { "" },
        lo = if p.link_only { "checked" } else { "" },
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/idp",
        &format!("auth/idp · {} · {}", escape(realm), escape(&p.alias)),
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
    fn list_returns_seeded_providers() {
        let s = AdminState::seeded();
        let p = list(&s, &ctx(), "acme-realm").unwrap();
        assert!(p.iter().any(|x| x.alias == "github"));
        assert!(p.iter().any(|x| x.alias == "saml-azure"));
    }

    #[test]
    fn render_list_includes_discover_form_and_two_create_buttons() {
        let s = AdminState::seeded();
        let html = render_list(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains(r#"action="/admin/auth/idp/acme-realm/discover""#));
        assert!(html.contains(r#"action="/admin/auth/idp/acme-realm/new/oidc""#));
        assert!(html.contains(r#"action="/admin/auth/idp/acme-realm/new/saml""#));
    }

    #[test]
    fn render_list_marks_enabled_disabled_badges() {
        let s = AdminState::seeded();
        let html = render_list(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains(">enabled<"));
        assert!(html.contains(">disabled<"));
    }

    #[test]
    fn render_detail_emits_all_seven_checkboxes_and_links_mappers() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "github").unwrap();
        for input in ["alias", "display_name", "provider_id", "enabled", "trust_email", "store_token", "link_only"] {
            assert!(html.contains(&format!(r#"name="{input}""#)), "missing input {input}");
        }
        assert!(html.contains("Mappers"));
    }

    #[test]
    fn render_detail_unknown_alias_falls_to_404() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "nope").unwrap();
        assert!(html.contains("No identity provider"));
    }

    #[test]
    fn list_requires_permission() {
        let s = AdminState::seeded();
        assert!(list(&s, &RequestCtx::developer("acme", &[]), "acme-realm").is_err());
    }
}
