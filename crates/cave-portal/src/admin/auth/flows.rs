// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/flows/{realm}` — Keycloak Admin "Authentication > Flows".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/admin-ui/src/authentication/AuthenticationSection.tsx`.

use super::fixtures::{self, AuthFlow};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::metrics::PortalMetrics;
use super::AuthViewError;

pub fn list(_state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<Vec<AuthFlow>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    Ok(fixtures::flows(realm))
}

pub fn detail(state: &AdminState, ctx: &RequestCtx, realm: &str, alias: &str) -> Result<Option<AuthFlow>, AuthViewError> {
    Ok(list(state, ctx, realm)?.into_iter().find(|f| f.alias == alias))
}

pub fn render_list(state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<String, AuthViewError> {
    let flows = list(state, ctx, realm)?;
    PortalMetrics::global().incr_page_view("admin_auth_flows", ctx.persona.as_str());
    let mut rows = String::new();
    for f in &flows {
        let badge = if f.built_in {
            r#"<span class="px-2 py-0.5 rounded bg-zinc-200 dark:bg-zinc-700 text-xs">built-in</span>"#
        } else {
            ""
        };
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2 font-medium"><a class="text-blue-700 underline" href="/admin/auth/flows/{realm}/{a}">{a}</a> {badge}</td>
  <td class="px-3 py-2">{desc}</td>
  <td class="px-3 py-2 text-zinc-500 text-sm">{ne}</td>
  <td class="px-3 py-2">
    <form method="post" action="/admin/auth/flows/{realm}/{a}/duplicate" class="inline">
      <button class="text-blue-700 text-sm hover:underline">Duplicate</button>
    </form>
  </td>
</tr>"#,
            realm = escape(realm),
            a = escape(&f.alias),
            badge = badge,
            desc = escape(&f.description),
            ne = f.executions.len(),
        ));
    }
    let body = format!(
        r#"<section>
  <div class="flex justify-between items-center mb-3">
    <h2 class="text-lg font-semibold">Authentication flows ({n})</h2>
    <form method="post" action="/admin/auth/flows/{realm}/new" class="inline">
      <button class="px-3 py-2 rounded bg-blue-600 text-white">+ Create flow</button>
    </form>
  </div>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Alias</th>
      <th class="px-3 py-2 text-left">Description</th>
      <th class="px-3 py-2 text-left">Executions</th>
      <th class="px-3 py-2 text-left">Actions</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</section>"#,
        realm = escape(realm),
        n = flows.len(),
        rows = rows,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/flows",
        &format!("auth/flows · {}", escape(realm)),
        &body,
    ))
}

pub fn render_detail(state: &AdminState, ctx: &RequestCtx, realm: &str, alias: &str) -> Result<String, AuthViewError> {
    let f = match detail(state, ctx, realm, alias)? {
        Some(f) => f,
        None => {
            return Ok(page_shell_full(
                ctx,
                "/admin/auth/flows",
                &format!("auth/flows · {} · {}", escape(realm), escape(alias)),
                &format!(r#"<p class="text-red-700">No flow <code>{}</code>.</p>"#, escape(alias)),
            ));
        }
    };
    PortalMetrics::global().incr_page_view("admin_auth_flow_detail", ctx.persona.as_str());
    let exec_rows: String = f
        .executions
        .iter()
        .map(|e| {
            let opts = ["REQUIRED", "ALTERNATIVE", "OPTIONAL", "DISABLED", "CONDITIONAL"]
                .iter()
                .map(|opt| {
                    let sel = if e.requirement == *opt { " selected" } else { "" };
                    format!(r#"<option{sel}>{opt}</option>"#)
                })
                .collect::<String>();
            format!(
                r#"<tr class="border-t">
  <td class="px-3 py-2">{name}</td>
  <td class="px-3 py-2"><code class="text-xs">{auth}</code></td>
  <td class="px-3 py-2">
    <form method="post" action="/admin/auth/flows/{r}/{a}/exec/{name_e}" class="inline">
      <select name="requirement" class="px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">{opts}</select>
      <button class="ml-2 text-blue-700 text-xs hover:underline">Save</button>
    </form>
  </td>
</tr>"#,
                name = escape(&e.display_name),
                name_e = escape(&e.display_name),
                auth = escape(&e.authenticator),
                r = escape(realm),
                a = escape(&f.alias),
                opts = opts,
            )
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm mb-2"><strong>Description:</strong> {desc}</p>
  <p class="text-sm mb-4"><strong>Provider id:</strong> <code>{pid}</code></p>
  <h2 class="text-lg font-semibold mb-2">Executions ({n})</h2>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Step</th>
      <th class="px-3 py-2 text-left">Authenticator</th>
      <th class="px-3 py-2 text-left">Requirement</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</section>"#,
        desc = escape(&f.description),
        pid = escape(&f.provider_id),
        n = f.executions.len(),
        rows = exec_rows,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/flows",
        &format!("auth/flows · {} · {}", escape(realm), escape(&f.alias)),
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
    fn list_returns_five_seeded_flows() {
        let s = AdminState::seeded();
        let f = list(&s, &ctx(), "acme-realm").unwrap();
        assert_eq!(f.len(), 5);
    }

    #[test]
    fn list_requires_permission() {
        let s = AdminState::seeded();
        assert!(list(&s, &RequestCtx::developer("acme", &[]), "acme-realm").is_err());
    }

    #[test]
    fn render_list_includes_create_and_duplicate_buttons() {
        let s = AdminState::seeded();
        let html = render_list(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains("+ Create flow"));
        assert!(html.contains("Duplicate"));
        assert!(html.contains(">built-in<"));
    }

    #[test]
    fn render_detail_lists_executions_with_requirement_selects() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "browser").unwrap();
        assert!(html.contains("Cookie"));
        assert!(html.contains("Identity provider redirector"));
        // Each row carries a <select name="requirement"> with the five options.
        for opt in ["REQUIRED", "ALTERNATIVE", "OPTIONAL", "DISABLED", "CONDITIONAL"] {
            assert!(html.contains(opt), "missing requirement option {opt}");
        }
    }

    #[test]
    fn render_detail_unknown_flow_falls_to_404() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "nope").unwrap();
        assert!(html.contains("No flow"));
    }

    #[test]
    fn render_detail_browser_flow_marks_alternative_selected_for_cookie() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "browser").unwrap();
        // Crude check: an <option selected>ALTERNATIVE</option> must
        // exist somewhere (every Cookie execution is ALTERNATIVE in
        // the seeded fixture).
        assert!(html.contains(r#"<option selected>ALTERNATIVE</option>"#));
    }
}
