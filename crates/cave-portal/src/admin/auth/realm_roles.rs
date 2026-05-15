// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/realm-roles/{realm}` — Keycloak Admin "Realm roles".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/admin-ui/src/realm-roles/RealmRoleTabs.tsx`.

use super::fixtures::{self, RealmRole};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::metrics::PortalMetrics;
use super::AuthViewError;

pub fn list(_state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<Vec<RealmRole>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    Ok(fixtures::realm_roles(realm))
}

pub fn detail(state: &AdminState, ctx: &RequestCtx, realm: &str, name: &str) -> Result<Option<RealmRole>, AuthViewError> {
    Ok(list(state, ctx, realm)?.into_iter().find(|r| r.name == name))
}

pub fn render_list(state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<String, AuthViewError> {
    let roles = list(state, ctx, realm)?;
    PortalMetrics::global().incr_page_view("admin_auth_realm_roles", ctx.persona.as_str());
    let mut rows = String::new();
    for r in &roles {
        let comp_badge = if r.composite {
            r#"<span class="px-2 py-0.5 rounded bg-purple-100 dark:bg-purple-900/30 text-purple-900 dark:text-purple-100 text-xs">composite</span>"#
        } else {
            ""
        };
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2 font-medium"><a class="text-blue-700 underline" href="/admin/auth/realm-roles/{realm}/{n}">{n}</a> {badge}</td>
  <td class="px-3 py-2">{desc}</td>
  <td class="px-3 py-2 text-zinc-500 text-sm">{ncomp}</td>
  <td class="px-3 py-2">
    <form method="post" action="/admin/auth/realm-roles/{realm}/{n}/delete" class="inline" onsubmit="return confirm('Delete role {nj}?');">
      <button class="text-red-700 text-sm hover:underline">Delete</button>
    </form>
  </td>
</tr>"#,
            realm = escape(realm),
            n = escape(&r.name),
            nj = escape(&r.name).replace('\'', "\\'"),
            badge = comp_badge,
            desc = escape(&r.description),
            ncomp = r.composites.len(),
        ));
    }
    let body = format!(
        r#"<section>
  <div class="flex justify-between items-center mb-3">
    <h2 class="text-lg font-semibold">Realm roles ({n})</h2>
    <form method="post" action="/admin/auth/realm-roles/{realm}/new">
      <button class="px-3 py-2 rounded bg-blue-600 text-white">+ Create role</button>
    </form>
  </div>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Name</th>
      <th class="px-3 py-2 text-left">Description</th>
      <th class="px-3 py-2 text-left">Composites</th>
      <th class="px-3 py-2 text-left">Actions</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</section>"#,
        realm = escape(realm),
        n = roles.len(),
        rows = rows,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/realm-roles",
        &format!("auth/realm-roles · {}", escape(realm)),
        &body,
    ))
}

pub fn render_detail(state: &AdminState, ctx: &RequestCtx, realm: &str, name: &str) -> Result<String, AuthViewError> {
    let role = match detail(state, ctx, realm, name)? {
        Some(r) => r,
        None => {
            return Ok(page_shell_full(
                ctx,
                "/admin/auth/realm-roles",
                &format!("auth/realm-roles · {} · {}", escape(realm), escape(name)),
                &format!(
                    r#"<p class="text-red-700">No realm role <code>{}</code>.</p>"#,
                    escape(name),
                ),
            ));
        }
    };
    PortalMetrics::global().incr_page_view("admin_auth_realm_role_detail", ctx.persona.as_str());
    let composites: String = role
        .composites
        .iter()
        .map(|c| {
            format!(
                r#"<li><code>{c}</code> <form method="post" class="inline" action="/admin/auth/realm-roles/{r}/{n}/composites/{c}/remove"><button class="text-red-700 text-xs ml-2 hover:underline">remove</button></form></li>"#,
                c = escape(c),
                r = escape(realm),
                n = escape(&role.name),
            )
        })
        .collect();
    let body = format!(
        r#"<section>
  <form method="post" action="/admin/auth/realm-roles/{realm}/{name}" class="grid grid-cols-1 gap-3 text-sm mb-6">
    <label>Name <input name="name" value="{name}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
    <label>Description <input name="description" value="{desc}" class="ml-2 px-2 py-1 border rounded w-full dark:bg-zinc-900 dark:border-zinc-700"></label>
    <label class="flex items-center gap-2"><input type="checkbox" name="composite" {comp}>Composite role</label>
    <div><button class="px-3 py-2 rounded bg-blue-600 text-white">Save</button></div>
  </form>
  <h2 class="text-lg font-semibold mb-2">Associated roles ({n})</h2>
  <ul class="list-disc ml-6 text-sm">{list}</ul>
  <form method="post" action="/admin/auth/realm-roles/{realm}/{name}/composites/add" class="flex gap-2 mt-2 text-sm">
    <input type="text" name="composite_name" placeholder="role name" class="px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">
    <button class="px-3 py-1 rounded border hover:bg-zinc-50 dark:hover:bg-zinc-800">+ Add associated role</button>
  </form>
</section>"#,
        realm = escape(realm),
        name = escape(&role.name),
        desc = escape(&role.description),
        comp = if role.composite { "checked" } else { "" },
        n = role.composites.len(),
        list = composites,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/realm-roles",
        &format!("auth/realm-roles · {} · {}", escape(realm), escape(&role.name)),
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
        assert!(list(&s, &RequestCtx::developer("acme", &[]), "acme-realm").is_err());
    }

    #[test]
    fn list_returns_seeded_roles_with_known_names() {
        let s = AdminState::seeded();
        let r = list(&s, &ctx(), "acme-realm").unwrap();
        assert!(r.iter().any(|x| x.name == "platform_admin"));
        assert!(r.iter().any(|x| x.name == "tenant_admin"));
    }

    #[test]
    fn detail_finds_composite_default_role() {
        let s = AdminState::seeded();
        let r = detail(&s, &ctx(), "acme-realm", "default-roles").unwrap().unwrap();
        assert!(r.composite);
        assert!(r.composites.contains(&"offline_access".to_string()));
    }

    #[test]
    fn render_list_emits_create_button_and_delete_forms() {
        let s = AdminState::seeded();
        let html = render_list(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains("+ Create role"));
        assert!(html.contains("/delete"));
        assert!(html.contains(">composite<"));
    }

    #[test]
    fn render_detail_lists_associated_roles_and_offers_add_form() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "default-roles").unwrap();
        assert!(html.contains("offline_access"));
        assert!(html.contains("+ Add associated role"));
    }

    #[test]
    fn render_detail_unknown_role_renders_red_text() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "nope").unwrap();
        assert!(html.contains("No realm role"));
    }
}
