// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/jwe` — JWE config (alg / enc) per realm. Visual port
//! of `js/apps/admin-ui/src/realm-settings/keys/key-providers/*` ID-token
//! encryption section.

use super::{AuthAdminError, render_admin_nav, require_platform};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JweConfig {
    pub id_token_alg: String,
    pub id_token_enc: String,
    pub userinfo_alg: String,
    pub userinfo_enc: String,
    pub request_object_alg: String,
    pub request_object_enc: String,
}

impl JweConfig {
    pub fn defaults() -> Self {
        Self {
            id_token_alg: "RSA-OAEP-256".into(),
            id_token_enc: "A256GCM".into(),
            userinfo_alg: "none".into(),
            userinfo_enc: "none".into(),
            request_object_alg: "none".into(),
            request_object_enc: "none".into(),
        }
    }
}

const KEY_ALGS: &[&str] = &[
    "none",
    "RSA-OAEP",
    "RSA-OAEP-256",
    "RSA1_5",
    "ECDH-ES",
    "ECDH-ES+A128KW",
    "ECDH-ES+A192KW",
    "ECDH-ES+A256KW",
    "A128KW",
    "A192KW",
    "A256KW",
    "dir",
];
const CONTENT_ENCS: &[&str] = &[
    "none",
    "A128CBC-HS256",
    "A192CBC-HS384",
    "A256CBC-HS512",
    "A128GCM",
    "A192GCM",
    "A256GCM",
];

fn select(name: &str, options: &[&str], current: &str) -> String {
    let mut out = format!(
        r#"<select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="{}">"#,
        escape(name)
    );
    for opt in options {
        let sel = if *opt == current { " selected" } else { "" };
        out.push_str(&format!(
            r#"<option value="{v}"{sel}>{v}</option>"#,
            v = escape(opt),
            sel = sel
        ));
    }
    out.push_str("</select>");
    out
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let c = JweConfig::defaults();
    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-2">JWE encryption</h2>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Per-realm JWE algorithm + content-encryption defaults for ID
    tokens, userinfo, and request objects (FAPI 2.0).
  </p>
  <form method="post" action="/admin/auth/jwe" class="grid grid-cols-2 gap-3 max-w-3xl" data-jwe-form>
    <label class="block">
      <span class="block text-sm font-medium">ID token alg</span>
      {alg_id}
    </label>
    <label class="block">
      <span class="block text-sm font-medium">ID token enc</span>
      {enc_id}
    </label>
    <label class="block">
      <span class="block text-sm font-medium">UserInfo alg</span>
      {alg_ui}
    </label>
    <label class="block">
      <span class="block text-sm font-medium">UserInfo enc</span>
      {enc_ui}
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Request object alg</span>
      {alg_ro}
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Request object enc</span>
      {enc_ro}
    </label>
    <div class="col-span-2 pt-2">
      <button type="submit" class="px-4 py-2 rounded bg-blue-600 text-white">Save</button>
    </div>
  </form>
</section>"#,
        nav = render_admin_nav("/admin/auth/jwe"),
        alg_id = select("idTokenAlg", KEY_ALGS, &c.id_token_alg),
        enc_id = select("idTokenEnc", CONTENT_ENCS, &c.id_token_enc),
        alg_ui = select("userInfoAlg", KEY_ALGS, &c.userinfo_alg),
        enc_ui = select("userInfoEnc", CONTENT_ENCS, &c.userinfo_enc),
        alg_ro = select("requestObjectAlg", KEY_ALGS, &c.request_object_alg),
        enc_ro = select("requestObjectEnc", CONTENT_ENCS, &c.request_object_enc),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/jwe",
        &format!("auth/jwe · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn defaults_use_rsa_oaep_256_and_a256gcm_for_id_tokens() {
        let c = JweConfig::defaults();
        assert_eq!(c.id_token_alg, "RSA-OAEP-256");
        assert_eq!(c.id_token_enc, "A256GCM");
    }

    #[test]
    fn key_algs_list_includes_rsa_oaep_ecdh_and_a256kw() {
        assert!(KEY_ALGS.contains(&"RSA-OAEP"));
        assert!(KEY_ALGS.contains(&"ECDH-ES+A256KW"));
        assert!(KEY_ALGS.contains(&"A256KW"));
    }

    #[test]
    fn content_encs_list_includes_aes_gcm_and_cbc_variants() {
        assert!(CONTENT_ENCS.contains(&"A128GCM"));
        assert!(CONTENT_ENCS.contains(&"A256GCM"));
        assert!(CONTENT_ENCS.contains(&"A256CBC-HS512"));
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_six_select_fields_for_id_userinfo_requestobject() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains(r#"name="idTokenAlg""#));
        assert!(html.contains(r#"name="idTokenEnc""#));
        assert!(html.contains(r#"name="userInfoAlg""#));
        assert!(html.contains(r#"name="userInfoEnc""#));
        assert!(html.contains(r#"name="requestObjectAlg""#));
        assert!(html.contains(r#"name="requestObjectEnc""#));
    }
}
