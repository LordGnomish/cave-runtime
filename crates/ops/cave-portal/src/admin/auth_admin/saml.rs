// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/saml` — SAML broker configuration UI. Calls A1's
//! cave-auth surfaces. Visual port of
//! `js/apps/admin-ui/src/identity-providers/add/AddSamlConnect.tsx`.

use super::{AuthAdminError, render_admin_nav, require_platform};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamlBrokerConfig {
    pub entity_id: String,
    pub sso_url: String,
    pub slo_url: String,
    pub binding: SamlBinding,
    pub sign_request: bool,
    pub want_assertions_signed: bool,
    pub want_assertions_encrypted: bool,
    pub principal_attribute: String,
    pub name_id_format: NameIdFormat,
    pub signature_algorithm: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamlBinding {
    Post,
    Redirect,
}
impl SamlBinding {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Post => "HTTP-POST",
            Self::Redirect => "HTTP-Redirect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameIdFormat {
    Persistent,
    Email,
    Transient,
    Unspecified,
}
impl NameIdFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Persistent => "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
            Self::Email => "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
            Self::Transient => "urn:oasis:names:tc:SAML:2.0:nameid-format:transient",
            Self::Unspecified => "urn:oasis:names:tc:SAML:1.1:nameid-format:unspecified",
        }
    }
}

pub fn default_config() -> SamlBrokerConfig {
    SamlBrokerConfig {
        entity_id: "urn:cave:realms:cave".into(),
        sso_url: "https://idp.example.com/saml/sso".into(),
        slo_url: "https://idp.example.com/saml/slo".into(),
        binding: SamlBinding::Post,
        sign_request: true,
        want_assertions_signed: true,
        want_assertions_encrypted: false,
        principal_attribute: "NameID".into(),
        name_id_format: NameIdFormat::Persistent,
        signature_algorithm: "RSA_SHA256".into(),
    }
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let c = default_config();
    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-2">SAML broker</h2>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Service-provider configuration for SAML 2.0 federation.
    Upstream: cave-auth A1 SAML broker.
  </p>
  <form method="post" action="/admin/auth/saml" class="space-y-3 max-w-2xl" data-saml-form>
    <label class="block">
      <span class="block text-sm font-medium">SP entity ID</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="entityId" value="{eid}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">IdP SSO URL</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="ssoUrl" value="{sso}" type="url">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">IdP SLO URL</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="sloUrl" value="{slo}" type="url">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">SAML binding</span>
      <select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="binding">
        <option value="HTTP-POST" selected>HTTP-POST</option>
        <option value="HTTP-Redirect">HTTP-Redirect</option>
      </select>
    </label>
    <label class="block">
      <span class="block text-sm font-medium">NameID format</span>
      <select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="nameIdFormat">
        <option value="persistent" selected>persistent</option>
        <option value="email">email</option>
        <option value="transient">transient</option>
        <option value="unspecified">unspecified</option>
      </select>
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Signature algorithm</span>
      <select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="sigAlg">
        <option value="RSA_SHA256" selected>RSA-SHA256</option>
        <option value="RSA_SHA512">RSA-SHA512</option>
        <option value="DSA_SHA1">DSA-SHA1 (legacy)</option>
      </select>
    </label>
    <fieldset class="space-y-1">
      <legend class="text-sm font-medium">Signing &amp; encryption</legend>
      <label class="inline-flex items-center mr-4">
        <input type="checkbox" name="signRequest" {sr}> <span class="ml-1 text-sm">Sign AuthnRequest</span>
      </label>
      <label class="inline-flex items-center mr-4">
        <input type="checkbox" name="wantAssertionsSigned" {was}> <span class="ml-1 text-sm">Want assertions signed</span>
      </label>
      <label class="inline-flex items-center">
        <input type="checkbox" name="wantAssertionsEncrypted" {wae}> <span class="ml-1 text-sm">Want assertions encrypted</span>
      </label>
    </fieldset>
    <label class="block">
      <span class="block text-sm font-medium">Principal attribute</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="principalAttribute" value="{pa}">
    </label>
    <div class="flex gap-2 pt-2">
      <button type="submit" class="px-4 py-2 rounded bg-blue-600 text-white">Save</button>
      <a href="/admin/auth/saml/metadata.xml" class="px-4 py-2 rounded bg-zinc-200 dark:bg-zinc-700">Download SP metadata</a>
    </div>
  </form>
</section>"#,
        nav = render_admin_nav("/admin/auth/saml"),
        eid = escape(&c.entity_id),
        sso = escape(&c.sso_url),
        slo = escape(&c.slo_url),
        pa = escape(&c.principal_attribute),
        sr = if c.sign_request { "checked" } else { "" },
        was = if c.want_assertions_signed {
            "checked"
        } else {
            ""
        },
        wae = if c.want_assertions_encrypted {
            "checked"
        } else {
            ""
        },
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/saml",
        &format!("auth/saml · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn default_config_uses_post_binding_persistent_nameid_and_signed_request() {
        let c = default_config();
        assert_eq!(c.binding, SamlBinding::Post);
        assert_eq!(c.name_id_format, NameIdFormat::Persistent);
        assert!(c.sign_request);
        assert!(c.want_assertions_signed);
    }

    #[test]
    fn nameid_format_wire_strings_match_saml_2_0_urns() {
        assert_eq!(
            NameIdFormat::Persistent.as_str(),
            "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent"
        );
        assert_eq!(
            NameIdFormat::Email.as_str(),
            "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress"
        );
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_all_saml_form_fields() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains(r#"name="entityId""#));
        assert!(html.contains(r#"name="ssoUrl""#));
        assert!(html.contains(r#"name="sloUrl""#));
        assert!(html.contains(r#"name="binding""#));
        assert!(html.contains(r#"name="nameIdFormat""#));
        assert!(html.contains(r#"name="sigAlg""#));
        assert!(html.contains(r#"name="signRequest""#));
        assert!(html.contains(r#"name="wantAssertionsSigned""#));
        assert!(html.contains(r#"name="principalAttribute""#));
    }

    #[test]
    fn render_offers_sp_metadata_download_link() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("/admin/auth/saml/metadata.xml"));
        assert!(html.contains("Download SP metadata"));
    }
}
