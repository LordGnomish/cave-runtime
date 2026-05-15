// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/idp` — Keycloak Admin Console "Identity Providers" pane.
//!
//! Surfaces the IdP catalogue + currently-configured instances. Backed by
//! `cave_auth::keycloak::admin::idp::IdpService` once the portal is wired
//! to the runtime; for now we render the supported factory list and a
//! placeholder for live instances (seeded fixtures).
//!
//! Upstream UI: <https://www.keycloak.org/docs-api/latest/rest-api/index.html#_identity_providers>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

/// A factory row mirrored from `cave_auth::keycloak::admin::idp::FACTORIES`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactoryRow {
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
}

pub fn factories() -> Vec<FactoryRow> {
    vec![
        FactoryRow { id: "oidc", name: "OpenID Connect v1.0", group: "Custom" },
        FactoryRow { id: "saml", name: "SAML v2.0", group: "Custom" },
        FactoryRow { id: "google", name: "Google", group: "Social" },
        FactoryRow { id: "github", name: "GitHub", group: "Social" },
        FactoryRow { id: "okta", name: "Okta", group: "Social" },
        FactoryRow { id: "facebook", name: "Facebook", group: "Social" },
        FactoryRow { id: "microsoft", name: "Microsoft", group: "Social" },
        FactoryRow { id: "apple", name: "Apple", group: "Social" },
        FactoryRow { id: "gitlab", name: "GitLab", group: "Social" },
        FactoryRow { id: "linkedin", name: "LinkedIn", group: "Social" },
        FactoryRow { id: "twitter", name: "Twitter / X", group: "Social" },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdpInstance {
    pub alias: String,
    pub provider_id: String,
    pub display_name: String,
    pub enabled: bool,
    pub trust_email: bool,
}

/// Seeded fixture — replaced by a live `IdpService` call when the portal is wired in.
pub fn seeded_instances() -> Vec<IdpInstance> {
    vec![
        IdpInstance {
            alias: "google".into(), provider_id: "google".into(),
            display_name: "Google".into(), enabled: true, trust_email: true,
        },
        IdpInstance {
            alias: "okta".into(), provider_id: "okta".into(),
            display_name: "Okta".into(), enabled: true, trust_email: true,
        },
    ]
}

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, super::AuthViewError> {
    ctx.authorise(Permission::AuthIdpRead)?;
    let facs = factories();
    let instances = seeded_instances();
    let fac_rows: Vec<Vec<String>> = facs.iter()
        .map(|f| vec![escape(f.id), escape(f.name), escape(f.group)])
        .collect();
    let inst_rows: Vec<Vec<String>> = instances.iter()
        .map(|i| vec![
            escape(&i.alias),
            escape(&i.provider_id),
            escape(&i.display_name),
            (if i.enabled { "enabled" } else { "disabled" }).to_string(),
            (if i.trust_email { "yes" } else { "no" }).to_string(),
        ])
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Keycloak Admin Console parity — Identity Providers.
    Backend: <code>cave_auth::keycloak::admin::idp</code>.
  </p>
  <h2 class="text-lg font-semibold mb-2">Configured instances ({n_inst})</h2>
  {tbl_inst}
  <h2 class="text-lg font-semibold mt-6 mb-2">Available provider factories ({n_fac})</h2>
  {tbl_fac}
  <details class="mt-4">
    <summary class="cursor-pointer text-blue-700">Add new provider</summary>
    <p class="text-sm text-gray-600 mt-2">
      cavectl: <code>cavectl auth idp create --alias &lt;a&gt; --provider-id &lt;id&gt;</code><br>
      OIDC discovery import: <code>cavectl auth idp import-config --from-url &lt;url&gt;</code>
    </p>
  </details>
</section>"#,
        n_inst = instances.len(),
        n_fac = facs.len(),
        tbl_inst = table(&["alias", "providerId", "displayName", "enabled", "trustEmail"], &inst_rows),
        tbl_fac = table(&["id", "name", "group"], &fac_rows),
    );
    Ok(page_shell_full(ctx, "/admin/auth/idp", &format!("auth/idp · {}", escape(ctx.tenant.as_str())), &body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn factories_include_oidc_saml_okta() {
        let f = factories();
        assert!(f.iter().any(|x| x.id == "oidc"));
        assert!(f.iter().any(|x| x.id == "saml"));
        assert!(f.iter().any(|x| x.id == "okta"));
    }

    #[test]
    fn render_requires_permission() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_emits_instance_and_factory_tables() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthIdpRead])).unwrap();
        assert!(html.contains("Identity Providers"));
        assert!(html.contains("Google"));
        assert!(html.contains("provider factories"));
    }

    #[test]
    fn render_shows_cavectl_hint() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthIdpRead])).unwrap();
        assert!(html.contains("cavectl auth idp"));
    }

    #[test]
    fn seeded_instances_have_two_entries() {
        let inst = seeded_instances();
        assert_eq!(inst.len(), 2);
        assert!(inst.iter().any(|i| i.alias == "google"));
    }
}
