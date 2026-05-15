// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/authn-config/{realm}` — Keycloak Admin "Authentication > Settings".
//!
//! Bundles: required actions, password policy, OTP policy, WebAuthn
//! policy, and the flow bindings.
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/admin-ui/src/authentication/`.

use super::fixtures::{self, AuthnConfig};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::metrics::PortalMetrics;
use super::AuthViewError;

pub fn get(_state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<AuthnConfig, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    Ok(fixtures::authn_config(realm))
}

pub fn render(state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<String, AuthViewError> {
    let c = get(state, ctx, realm)?;
    PortalMetrics::global().incr_page_view("admin_auth_authn_config", ctx.persona.as_str());
    let known_flows: Vec<String> = fixtures::flows(realm).into_iter().map(|f| f.alias).collect();
    let flow_select = |name: &str, value: &str| -> String {
        let opts: String = known_flows
            .iter()
            .map(|f| {
                let sel = if f == value { " selected" } else { "" };
                format!(r#"<option value="{f}"{sel}>{f}</option>"#, f = escape(f), sel = sel)
            })
            .collect();
        format!(
            r#"<label>{label}
  <select name="{name}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">{opts}</select>
</label>"#,
            label = escape(&humanise(name)),
            name = escape(name),
            opts = opts,
        )
    };
    let req_rows: String = c
        .required_actions
        .iter()
        .map(|r| {
            format!(
                r#"<tr class="border-t">
  <td class="px-3 py-2"><code class="text-xs">{a}</code></td>
  <td class="px-3 py-2">{n}</td>
  <td class="px-3 py-2">{e}</td>
  <td class="px-3 py-2">{d}</td>
</tr>"#,
                a = escape(&r.alias),
                n = escape(&r.name),
                e = if r.enabled { "on" } else { "off" },
                d = if r.default_action { "yes" } else { "no" },
            )
        })
        .collect();
    let body = format!(
        r#"<form method="post" action="/admin/auth/authn-config/{realm}" class="space-y-6">
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">Flow bindings</legend>
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
      {browser}
      {direct_grant}
      {reset}
      {client_auth}
      {registration}
      {docker}
    </div>
  </fieldset>
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">Password policy</legend>
    <div class="grid grid-cols-1 md:grid-cols-3 gap-3 text-sm">
      <label>Min length <input type="number" name="pwd_min_length" value="{pwd_min}" class="ml-2 px-2 py-1 border rounded w-20 dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Digits <input type="number" name="pwd_digits" value="{pwd_d}" class="ml-2 px-2 py-1 border rounded w-20 dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Upper case <input type="number" name="pwd_upper" value="{pwd_u}" class="ml-2 px-2 py-1 border rounded w-20 dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Special chars <input type="number" name="pwd_special" value="{pwd_s}" class="ml-2 px-2 py-1 border rounded w-20 dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label class="flex items-center gap-2"><input type="checkbox" name="pwd_not_username" {pwd_nu}>Not username</label>
      <label class="flex items-center gap-2"><input type="checkbox" name="pwd_not_email" {pwd_ne}>Not email</label>
      <label>Hash iterations <input type="number" name="pwd_hash_iter" value="{pwd_it}" class="ml-2 px-2 py-1 border rounded w-28 dark:bg-zinc-900 dark:border-zinc-700"></label>
    </div>
  </fieldset>
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">OTP policy</legend>
    <div class="grid grid-cols-1 md:grid-cols-3 gap-3 text-sm">
      <label>Type
        <select name="otp_kind" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">
          <option {totp}>totp</option>
          <option {hotp}>hotp</option>
        </select>
      </label>
      <label>Algorithm <input name="otp_alg" value="{otp_alg}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Digits <input type="number" name="otp_digits" value="{otp_d}" class="ml-2 px-2 py-1 border rounded w-20 dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Period (s) <input type="number" name="otp_period" value="{otp_p}" class="ml-2 px-2 py-1 border rounded w-20 dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Lookahead window <input type="number" name="otp_lookahead" value="{otp_l}" class="ml-2 px-2 py-1 border rounded w-20 dark:bg-zinc-900 dark:border-zinc-700"></label>
    </div>
  </fieldset>
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">WebAuthn policy</legend>
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm">
      <label>RP name <input name="rp_name" value="{rp}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Signature algorithms <input name="rp_sig_alg" value="{rp_sig}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Attestation conveyance
        <select name="rp_att_conv" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">
          <option {acn}>none</option>
          <option {aci}>indirect</option>
          <option {acd}>direct</option>
        </select>
      </label>
      <label>Authenticator attachment <input name="rp_auth_att" value="{rp_att}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>User verification <input name="rp_uv" value="{rp_uv}" class="ml-2 px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700"></label>
      <label>Create timeout (s) <input type="number" name="rp_create_to" value="{rp_to}" class="ml-2 px-2 py-1 border rounded w-24 dark:bg-zinc-900 dark:border-zinc-700"></label>
    </div>
  </fieldset>
  <fieldset class="border rounded p-4 dark:border-zinc-800">
    <legend class="text-sm font-medium px-2">Required actions</legend>
    <table class="min-w-full text-sm border-collapse">
      <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
        <th class="px-3 py-2 text-left">Alias</th>
        <th class="px-3 py-2 text-left">Name</th>
        <th class="px-3 py-2 text-left">Enabled</th>
        <th class="px-3 py-2 text-left">Default action</th>
      </tr></thead>
      <tbody>{req_rows}</tbody>
    </table>
  </fieldset>
  <div><button class="px-4 py-2 rounded bg-blue-600 text-white">Save</button></div>
</form>"#,
        realm = escape(realm),
        browser = flow_select("browser_flow", &c.browser_flow),
        direct_grant = flow_select("direct_grant_flow", &c.direct_grant_flow),
        reset = flow_select("reset_credentials_flow", &c.reset_credentials_flow),
        client_auth = flow_select("client_authentication_flow", &c.client_authentication_flow),
        registration = flow_select("registration_flow", &c.registration_flow),
        docker = flow_select("docker_authentication_flow", &c.docker_authentication_flow),
        pwd_min = c.password_policy.min_length,
        pwd_d = c.password_policy.digits,
        pwd_u = c.password_policy.upper_case,
        pwd_s = c.password_policy.special_chars,
        pwd_nu = if c.password_policy.not_username { "checked" } else { "" },
        pwd_ne = if c.password_policy.not_email { "checked" } else { "" },
        pwd_it = c.password_policy.hash_iterations,
        totp = if c.otp_policy.kind == "totp" { "selected" } else { "" },
        hotp = if c.otp_policy.kind == "hotp" { "selected" } else { "" },
        otp_alg = escape(&c.otp_policy.algorithm),
        otp_d = c.otp_policy.digits,
        otp_p = c.otp_policy.period_seconds,
        otp_l = c.otp_policy.lookahead_window,
        rp = escape(&c.webauthn_policy.rp_name),
        rp_sig = escape(&c.webauthn_policy.signature_algorithms.join(",")),
        acn = if c.webauthn_policy.attestation_conveyance_preference == "none" { "selected" } else { "" },
        aci = if c.webauthn_policy.attestation_conveyance_preference == "indirect" { "selected" } else { "" },
        acd = if c.webauthn_policy.attestation_conveyance_preference == "direct" { "selected" } else { "" },
        rp_att = escape(&c.webauthn_policy.authenticator_attachment),
        rp_uv = escape(&c.webauthn_policy.user_verification_requirement),
        rp_to = c.webauthn_policy.create_timeout_seconds,
        req_rows = req_rows,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/authn-config",
        &format!("auth/authn-config · {}", escape(realm)),
        &body,
    ))
}

fn humanise(s: &str) -> String {
    s.replace('_', " ")
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
        assert!(get(&s, &RequestCtx::developer("acme", &[]), "acme-realm").is_err());
    }

    #[test]
    fn render_emits_all_five_fieldsets() {
        let s = AdminState::seeded();
        let html = render(&s, &ctx(), "acme-realm").unwrap();
        for legend in [
            "Flow bindings",
            "Password policy",
            "OTP policy",
            "WebAuthn policy",
            "Required actions",
        ] {
            assert!(html.contains(legend), "missing legend: {legend}");
        }
    }

    #[test]
    fn render_flow_select_binds_browser_flow_to_known_alias() {
        let s = AdminState::seeded();
        let html = render(&s, &ctx(), "acme-realm").unwrap();
        // Browser flow defaults to "browser" — its <option value="browser" selected>browser</option>
        assert!(html.contains(r#"<option value="browser" selected>browser</option>"#));
    }

    #[test]
    fn render_otp_select_marks_totp_as_default() {
        let s = AdminState::seeded();
        let html = render(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains(r#"<option selected>totp</option>"#));
    }

    #[test]
    fn render_required_actions_table_includes_webauthn_and_otp() {
        let s = AdminState::seeded();
        let html = render(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains("CONFIGURE_TOTP"));
        assert!(html.contains("webauthn-register-passwordless"));
    }

    #[test]
    fn render_includes_post_form_to_correct_endpoint() {
        let s = AdminState::seeded();
        let html = render(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains(r#"action="/admin/auth/authn-config/acme-realm""#));
    }

    #[test]
    fn humanise_replaces_underscores_with_spaces() {
        assert_eq!(humanise("browser_flow"), "browser flow");
    }
}
