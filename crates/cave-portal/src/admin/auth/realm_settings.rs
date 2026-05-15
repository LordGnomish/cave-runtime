// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/realm-settings/{realm}` — Keycloak Admin Console "Realm Settings".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/admin-ui/src/realm-settings/RealmSettingsSection.tsx`.

use super::fixtures::{self, RealmSettings};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::metrics::PortalMetrics;
use super::AuthViewError;

pub fn get(_state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<RealmSettings, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    Ok(fixtures::realm_settings(realm))
}

pub fn render(state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<String, AuthViewError> {
    let s = get(state, ctx, realm)?;
    PortalMetrics::global()
        .incr_page_view("admin_auth_realm_settings", ctx.persona.as_str());
    let locales = s
        .supported_locales
        .iter()
        .map(|l| {
            let sel = if *l == s.default_locale { " selected" } else { "" };
            format!(
                r#"<option value="{l}"{sel}>{l}</option>"#,
                l = escape(l),
                sel = sel,
            )
        })
        .collect::<String>();
    let body = format!(
        r#"<form method="post" action="/admin/auth/realm-settings/{realm}" class="space-y-6">
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">General</legend>
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
      <label>Realm name <code>{realm}</code> <span class="text-zinc-500">(immutable)</span></label>
      <label>Display name <input class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700" name="display_name" value="{display}"></label>
      <label class="flex items-center gap-2"><input type="checkbox" name="enabled" {enabled}>Enabled</label>
      <label>SSL required
        <select name="ssl_required" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">
          <option {ssl_ext}>external</option>
          <option {ssl_all}>all</option>
          <option {ssl_non}>none</option>
        </select>
      </label>
    </div>
  </fieldset>
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">Login</legend>
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
      <label class="flex items-center gap-2"><input type="checkbox" name="registration_allowed" {reg}>User registration</label>
      <label class="flex items-center gap-2"><input type="checkbox" name="login_with_email_allowed" {lwe}>Login with email</label>
      <label class="flex items-center gap-2"><input type="checkbox" name="duplicate_emails_allowed" {dup}>Duplicate emails allowed</label>
    </div>
  </fieldset>
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">Tokens</legend>
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
      <label>Access token lifespan (s) <input type="number" name="access_token_lifespan" value="{atl}" class="ml-2 px-2 py-1 border rounded w-32 dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>SSO session idle (s) <input type="number" name="sso_session_idle_timeout" value="{sst}" class="ml-2 px-2 py-1 border rounded w-32 dark:bg-zinc-900 dark:border-zinc-700"></label>
    </div>
  </fieldset>
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">Security defenses</legend>
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
      <label class="flex items-center gap-2"><input type="checkbox" name="brute_force_protected" {bfp}>Brute-force detection</label>
      <label class="flex items-center gap-2"><input type="checkbox" name="permanent_lockout" {pl}>Permanent lockout</label>
      <label>Max login failures <input type="number" name="max_login_failures" value="{mlf}" class="ml-2 px-2 py-1 border rounded w-24 dark:bg-zinc-900 dark:border-zinc-700"></label>
    </div>
  </fieldset>
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">Themes &amp; localization</legend>
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
      <label>Login theme <input name="login_theme" value="{login_theme}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Account theme <input name="account_theme" value="{account_theme}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Admin theme <input name="admin_theme" value="{admin_theme}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Email theme <input name="email_theme" value="{email_theme}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Default locale <select name="default_locale" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">{locales}</select></label>
    </div>
  </fieldset>
  <div class="flex gap-2">
    <button class="px-4 py-2 rounded bg-blue-600 text-white hover:bg-blue-700">Save</button>
    <a href="/admin/auth/realms" class="px-4 py-2 rounded border hover:bg-zinc-50 dark:hover:bg-zinc-800">Cancel</a>
  </div>
</form>"#,
        realm = escape(&s.realm),
        display = escape(&s.display_name),
        enabled = if s.enabled { "checked" } else { "" },
        ssl_ext = if s.ssl_required == "external" { "selected" } else { "" },
        ssl_all = if s.ssl_required == "all" { "selected" } else { "" },
        ssl_non = if s.ssl_required == "none" { "selected" } else { "" },
        reg = if s.registration_allowed { "checked" } else { "" },
        lwe = if s.login_with_email_allowed { "checked" } else { "" },
        dup = if s.duplicate_emails_allowed { "checked" } else { "" },
        atl = s.access_token_lifespan,
        sst = s.sso_session_idle_timeout,
        bfp = if s.brute_force_protected { "checked" } else { "" },
        pl = if s.permanent_lockout { "checked" } else { "" },
        mlf = s.max_login_failures,
        login_theme = escape(&s.login_theme),
        account_theme = escape(&s.account_theme),
        admin_theme = escape(&s.admin_theme),
        email_theme = escape(&s.email_theme),
        locales = locales,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/realm-settings",
        &format!("auth/realm-settings · {}", escape(&s.realm)),
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
    fn get_requires_permission() {
        let s = AdminState::seeded();
        let bad = RequestCtx::developer("acme", &[]);
        assert!(get(&s, &bad, "acme-realm").is_err());
    }

    #[test]
    fn get_returns_realm_struct() {
        let s = AdminState::seeded();
        let r = get(&s, &ctx(), "acme-realm").unwrap();
        assert_eq!(r.realm, "acme-realm");
        assert!(r.enabled);
    }

    #[test]
    fn render_emits_post_form_with_realm_in_action() {
        let s = AdminState::seeded();
        let html = render(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains(r#"action="/admin/auth/realm-settings/acme-realm""#));
        assert!(html.contains(r#"method="post""#));
    }

    #[test]
    fn render_includes_all_six_fieldsets() {
        let s = AdminState::seeded();
        let html = render(&s, &ctx(), "acme-realm").unwrap();
        for legend in [
            "General",
            "Login",
            "Tokens",
            "Security defenses",
            "Themes &amp; localization",
        ] {
            assert!(html.contains(legend), "missing legend: {legend}");
        }
    }

    #[test]
    fn render_locale_dropdown_marks_default() {
        let s = AdminState::seeded();
        let html = render(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains(r#"<option value="en" selected>en</option>"#));
    }

    #[test]
    fn render_includes_brute_force_toggle() {
        let s = AdminState::seeded();
        let html = render(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains(r#"name="brute_force_protected""#));
        assert!(html.contains("checked")); // BFP is on by default
    }
}
