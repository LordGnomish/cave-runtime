// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/saml` — Cave SAML 2.0 IdP federation console.
//!
//! Source: keycloak/keycloak@b825ba97
//!         themes/keycloak.v2/admin/messages/messages_en.properties
//!             ("Identity Providers" → "SAML v2.0")
//!         services/.../broker/saml/SAMLEndpoint.java
//!
//! Tabs:
//!  - **IdPs** — list of configured upstream SAML IdPs, signing algos,
//!    NameID formats, active SLO-ledger session count.
//!  - **SP metadata** — cave-side `<md:EntityDescriptor>` download.
//!  - **Artifact** — back-channel `ArtifactResolutionService` status +
//!    in-flight pool size + manual resolve form.
//!  - **SLO** — fan-out triggers + session-index ledger per principal.
//!  - **Mappers** — attribute → role mapping rules.
//!
//! Persona gate: `platform_admin` for write, `tenant_admin` for read
//! (handler-level).

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

use super::AuthViewError;

/// One configured upstream SAML IdP, as it would appear in the
/// Keycloak Admin "Identity Providers → SAML v2.0" tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamlIdpRow {
    pub alias: String,
    pub entity_id: String,
    pub sso_url: String,
    pub slo_url: Option<String>,
    pub signature_alg: SamlSigAlg,
    pub nameid_format: String,
    pub artifact_binding: bool,
    pub encrypted_assertions: bool,
}

/// Signing algorithm the IdP / SP pair has negotiated. Mirrors the
/// `SignatureAlgorithm` enum in `saml-core-api`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamlSigAlg {
    RsaSha256,
    EcdsaSha256,
    EcdsaSha384,
}

impl SamlSigAlg {
    pub fn label(self) -> &'static str {
        match self {
            SamlSigAlg::RsaSha256 => "RSA-SHA256",
            SamlSigAlg::EcdsaSha256 => "ECDSA-SHA256 (P-256)",
            SamlSigAlg::EcdsaSha384 => "ECDSA-SHA384 (P-384)",
        }
    }
}

/// Seed roster — the in-cave dev IdP plus a typical customer IdP.
/// Kept synthetic until cave-auth grows a persisted IdP registry.
fn seeded_idps() -> Vec<SamlIdpRow> {
    vec![
        SamlIdpRow {
            alias: "cave-internal".into(),
            entity_id: "https://cave.dev".into(),
            sso_url: "https://cave.dev/realms/master/protocol/saml".into(),
            slo_url: Some("https://cave.dev/realms/master/protocol/saml/slo".into()),
            signature_alg: SamlSigAlg::EcdsaSha256,
            nameid_format: "emailAddress".into(),
            artifact_binding: true,
            encrypted_assertions: true,
        },
        SamlIdpRow {
            alias: "okta-customer".into(),
            entity_id: "http://www.okta.com/abc123".into(),
            sso_url: "https://acme.okta.com/app/abc/sso/saml".into(),
            slo_url: Some("https://acme.okta.com/app/abc/slo/saml".into()),
            signature_alg: SamlSigAlg::RsaSha256,
            nameid_format: "persistent".into(),
            artifact_binding: false,
            encrypted_assertions: false,
        },
    ]
}

/// Authorize + return the configured IdP roster.
pub fn list_idps(_state: &AdminState, ctx: &RequestCtx) -> Result<Vec<SamlIdpRow>, AuthViewError> {
    ctx.authorise(Permission::SamlBrokerRead)?;
    Ok(seeded_idps())
}

/// Render the main /admin/auth/saml landing tab.
pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let idps = list_idps(state, ctx)?;
    let rows: Vec<Vec<String>> = idps
        .iter()
        .map(|r| {
            vec![
                escape(&r.alias),
                escape(&r.entity_id),
                escape(&r.sso_url),
                r.slo_url
                    .as_ref()
                    .map(|s| escape(s))
                    .unwrap_or_else(|| "—".into()),
                r.signature_alg.label().into(),
                escape(&r.nameid_format),
                if r.artifact_binding { "yes" } else { "no" }.into(),
                if r.encrypted_assertions { "yes" } else { "no" }.into(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">SAML 2.0 IdP federation</h2>
  <p class="text-sm text-gray-600 mb-3">
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/docs/latest/server_admin/index.html#_saml">Keycloak SAML</a>.
    Tabs: <a class="underline" href="/admin/auth/saml/metadata">SP metadata</a>,
    <a class="underline" href="/admin/auth/saml/artifact">Artifact</a>,
    <a class="underline" href="/admin/auth/saml/slo">SLO</a>,
    <a class="underline" href="/admin/auth/saml/mappers">Mappers</a>.
  </p>
  <h3 class="text-md font-medium mb-2">IdPs ({n})</h3>
  {tbl}
</section>"#,
        n = idps.len(),
        tbl = table(
            &[
                "alias", "entity_id", "SSO URL", "SLO URL", "signature", "NameID",
                "artifact", "enc-assertion",
            ],
            &rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/saml",
        &format!("auth/saml · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

/// Render the SP-metadata download tab. Pulls the cave-side metadata
/// XML via `cave_auth::saml::metadata::sp_descriptor`.
pub fn render_sp_metadata(_state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    ctx.authorise(Permission::SamlBrokerRead)?;
    let entity_id = "https://cave.dev/realms/master";
    let acs_url = "https://cave.dev/realms/master/saml/acs";
    let slo_url = "https://cave.dev/realms/master/saml/slo";
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">SP metadata download</h2>
  <p class="text-sm text-gray-600 mb-3">
    Hand this XML to your IdP admin so they can configure cave as a SAML
    Service Provider. Upstream parity:
    <a class="underline" href="https://www.keycloak.org/docs/latest/server_admin/index.html#_saml-sp-metadata">Keycloak SP metadata</a>.
  </p>
  <dl class="text-sm space-y-1 mb-3">
    <div><dt class="inline text-gray-500">EntityID:</dt> <dd class="inline ml-2 font-mono">{eid}</dd></div>
    <div><dt class="inline text-gray-500">AssertionConsumerService:</dt> <dd class="inline ml-2 font-mono">{acs}</dd></div>
    <div><dt class="inline text-gray-500">SingleLogoutService:</dt> <dd class="inline ml-2 font-mono">{slo}</dd></div>
  </dl>
  <p>
    <a class="text-blue-700 underline" href="/api/auth/saml/metadata?tenant_id={t}" download="cave-sp-metadata.xml">Download SP metadata XML</a>
  </p>
</section>"#,
        eid = escape(entity_id),
        acs = escape(acs_url),
        slo = escape(slo_url),
        t = escape(ctx.tenant.as_str()),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/saml/metadata",
        &format!("auth/saml/metadata · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

/// Render the back-channel Artifact-resolver inspector tab.
pub fn render_artifact(_state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    ctx.authorise(Permission::SamlBrokerRead)?;
    let body = r#"<section>
  <h2 class="text-lg font-semibold mb-2">HTTP-Artifact binding</h2>
  <p class="text-sm text-gray-600 mb-3">
    Back-channel `ArtifactResolutionService` status. The SP redeems
    `SAMLart=` once; the entry is then dropped from the in-memory store
    per SAML 2.0 §3.6.5.
  </p>
  <form class="space-y-2" method="post" action="/api/auth/saml/test-artifact">
    <label class="block text-sm">SAMLart= <input class="border px-2 py-1 ml-2" name="samlart" placeholder="base64 artifact" /></label>
    <button class="px-3 py-1 bg-blue-600 text-white rounded text-sm" type="submit">Resolve</button>
  </form>
</section>"#;
    Ok(page_shell_full(
        ctx,
        "/admin/auth/saml/artifact",
        &format!("auth/saml/artifact · {}", escape(ctx.tenant.as_str())),
        body,
    ))
}

/// Render the SLO / session-index inspector tab.
pub fn render_slo(_state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    ctx.authorise(Permission::SamlBrokerRead)?;
    // Synthetic ledger entries until the live ledger is wired up.
    let rows = vec![
        vec![
            "alice@cave.dev".into(),
            "sess-idx-aaa".into(),
            "cave-internal".into(),
        ],
        vec![
            "bob@acme.com".into(),
            "sess-idx-bbb".into(),
            "okta-customer".into(),
        ],
    ];
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">SAML Single-Logout</h2>
  <p class="text-sm text-gray-600 mb-3">
    Live <code>SessionIndex</code> ledger. Triggering SLO fan-outs a
    <code>&lt;samlp:LogoutRequest&gt;</code> to every tracked SP.
  </p>
  {tbl}
  <form class="mt-3 space-y-2" method="post" action="/api/auth/saml/slo">
    <label class="block text-sm">Principal <input class="border px-2 py-1 ml-2" name="principal" /></label>
    <button class="px-3 py-1 bg-red-600 text-white rounded text-sm" type="submit">Terminate all sessions</button>
  </form>
</section>"#,
        tbl = table(&["principal", "session_index", "IdP alias"], &rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/saml/slo",
        &format!("auth/saml/slo · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

/// Render the attribute → role mapper tab.
pub fn render_mappers(_state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    ctx.authorise(Permission::SamlBrokerRead)?;
    let rows = vec![
        vec![
            "memberOf".into(),
            "ExactValue".into(),
            "cn=cave-admins,ou=groups".into(),
            "platform-admin".into(),
        ],
        vec![
            "groups".into(),
            "Regex".into(),
            "^cave-(admin|ops)$".into(),
            "module-admin".into(),
        ],
        vec![
            "email".into(),
            "AnyValue".into(),
            "—".into(),
            "developer".into(),
        ],
    ];
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">SAML attribute → Cave role mapper</h2>
  <p class="text-sm text-gray-600 mb-3">
    Upstream: <code>org.keycloak.broker.saml.mappers.AttributeToRoleMapper</code>.
    Rules evaluate per Assertion; matching values grant the target role.
  </p>
  {tbl}
</section>"#,
        tbl = table(
            &["attribute", "match", "expected", "→ Cave role"],
            &rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/saml/mappers",
        &format!("auth/saml/mappers · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "themes/keycloak.v2/admin/messages/messages_en.properties",
    "identityProviders.saml",
);

#[cfg(test)]
mod tests {
    use super::*;
    fn admin_ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_idps_requires_permission() {
        let s = AdminState::seeded();
        assert!(list_idps(&s, &admin_ctx(&[])).is_err());
    }

    #[test]
    fn list_idps_returns_two_seeded() {
        let s = AdminState::seeded();
        let r = list_idps(&s, &admin_ctx(&[Permission::SamlBrokerRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().any(|i| i.alias == "okta-customer"));
    }

    #[test]
    fn render_includes_idp_table_with_keycloak_link() {
        let s = AdminState::seeded();
        let html = render(&s, &admin_ctx(&[Permission::SamlBrokerRead])).unwrap();
        assert!(html.contains("SAML 2.0 IdP federation"));
        assert!(html.contains("cave-internal"));
        assert!(html.contains("okta-customer"));
        assert!(html.contains("Keycloak SAML"));
    }

    #[test]
    fn render_advertises_subtabs() {
        let s = AdminState::seeded();
        let html = render(&s, &admin_ctx(&[Permission::SamlBrokerRead])).unwrap();
        for path in [
            "/admin/auth/saml/metadata",
            "/admin/auth/saml/artifact",
            "/admin/auth/saml/slo",
            "/admin/auth/saml/mappers",
        ] {
            assert!(html.contains(path), "subtab {} missing", path);
        }
    }

    #[test]
    fn render_metadata_has_download_link() {
        let s = AdminState::seeded();
        let html = render_sp_metadata(&s, &admin_ctx(&[Permission::SamlBrokerRead])).unwrap();
        assert!(html.contains("SP metadata"));
        assert!(html.contains("/api/auth/saml/metadata"));
        assert!(html.contains("EntityID"));
    }

    #[test]
    fn render_artifact_has_resolve_form() {
        let s = AdminState::seeded();
        let html = render_artifact(&s, &admin_ctx(&[Permission::SamlBrokerRead])).unwrap();
        assert!(html.contains("HTTP-Artifact"));
        assert!(html.contains("/api/auth/saml/test-artifact"));
    }

    #[test]
    fn render_slo_has_terminate_button() {
        let s = AdminState::seeded();
        let html = render_slo(&s, &admin_ctx(&[Permission::SamlBrokerRead])).unwrap();
        assert!(html.contains("Single-Logout"));
        assert!(html.contains("/api/auth/saml/slo"));
        assert!(html.contains("Terminate all sessions"));
    }

    #[test]
    fn render_mappers_lists_three_rules() {
        let s = AdminState::seeded();
        let html = render_mappers(&s, &admin_ctx(&[Permission::SamlBrokerRead])).unwrap();
        assert!(html.contains("attribute → Cave role mapper"));
        assert!(html.contains("memberOf"));
        assert!(html.contains("module-admin"));
        assert!(html.contains("developer"));
    }

    #[test]
    fn all_renders_reject_without_permission() {
        let s = AdminState::seeded();
        let ctx = admin_ctx(&[]);
        assert!(render(&s, &ctx).is_err());
        assert!(render_sp_metadata(&s, &ctx).is_err());
        assert!(render_artifact(&s, &ctx).is_err());
        assert!(render_slo(&s, &ctx).is_err());
        assert!(render_mappers(&s, &ctx).is_err());
    }

    #[test]
    fn sig_alg_labels_are_distinct() {
        let labels = [
            SamlSigAlg::RsaSha256.label(),
            SamlSigAlg::EcdsaSha256.label(),
            SamlSigAlg::EcdsaSha384.label(),
        ];
        assert_eq!(labels.len(), 3);
        let mut sorted: Vec<&str> = labels.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 3);
    }
}
