//! `/admin/vault` — Vault Web UI parity surface.
//!
//! Mirrors the **upstream Vault UI**'s top-level tabs:
//!
//! 1. **Secrets** — every mounted secrets engine (`kv`, `pki`,
//!    `transit`, `database`, …) and the metadata for each path under
//!    them. Vault never exposes plaintext values through this UI; the
//!    same invariant holds here ([`read_value`] returns a hard
//!    refusal).
//! 2. **Access** — auth methods (`token`, `userpass`, `kubernetes`,
//!    `approle`, `oidc`, …).
//! 3. **Policies** — ACL policy documents and the token count bound
//!    to each.
//! 4. **Audit** — append-only audit log.
//!
//! The module is split per tab so each surface owns its data
//! accessors, render helper, and tests. `mod.rs` re-exports the
//! legacy `list_secrets` / `list_audit` / `read_value` /
//! `VaultViewError` so any older caller keeps compiling, and provides
//! a single [`render`] entry point that composes all four tabs into
//! one page.
//!
//! Upstream UI: <https://developer.hashicorp.com/vault/docs/configuration/ui>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod audit;
pub mod auth_methods;
pub mod kv_browser;
pub mod policies;
pub mod secrets_engines;

pub use audit::list_audit;
pub use auth_methods::list_auth_methods;
pub use kv_browser::list_secrets;
pub use policies::list_policies;
pub use secrets_engines::list_secrets_engines;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VaultViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("vault values are never exposed through the admin view")]
    ValueAccessForbidden,
}

/// Hard rejection: the UI never exposes plaintext secret values, even
/// to a caller with [`Permission::VaultRead`]. Returns
/// [`VaultViewError::ValueAccessForbidden`] 100% of the time. Vault's
/// own UI follows the same rule — values are surfaced only by
/// targeted CLI / API calls, not by the browser-side admin view.
pub fn read_value(_path: &str) -> Result<String, VaultViewError> {
    Err(VaultViewError::ValueAccessForbidden)
}

/// Render the four-tab Vault UI: Secrets → Access → Policies → Audit.
/// Tabs are rendered as anchored sections rather than client-side
/// toggles so the page is still useful with JavaScript disabled.
pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, VaultViewError> {
    // Auth the whole composite up-front so a permission gate failure
    // returns a single error rather than half-rendering the page.
    ctx.authorise(Permission::VaultRead)?;
    let engines_html = secrets_engines::render_section(state, ctx)?;
    let kv_html = kv_browser::render_section(state, ctx)?;
    let access_html = auth_methods::render_section(state, ctx)?;
    let policies_html = policies::render_section(state, ctx)?;
    let audit_html = audit::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Vault Web UI parity surface. Plaintext secret values are NEVER
  exposed here — see <code>read_value</code> docs.
  Upstream:
  <a class="text-blue-700 underline" href="https://developer.hashicorp.com/vault/docs/configuration/ui">developer.hashicorp.com/vault/docs/configuration/ui</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#secrets-engines">Secrets</a>
  <a href="#kv-browser">KV browser</a>
  <a href="#access">Access</a>
  <a href="#policies">Policies</a>
  <a href="#audit">Audit</a>
</nav>
{engines}
{kv}
{access}
{policies}
{audit}"##,
        engines = engines_html,
        kv = kv_html,
        access = access_html,
        policies = policies_html,
        audit = audit_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/vault",
        &format!("vault · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/auth-react/src/components/UserSettings/AuthProviders/AuthProviders.tsx",
    "AuthProviders",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn read_value_always_returns_forbidden() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/SecretReveal.tsx",
            "RevealValue",
            "acme"
        );
        assert!(matches!(
            read_value("kv/db").unwrap_err(),
            VaultViewError::ValueAccessForbidden
        ));
    }

    #[test]
    fn render_requires_vault_read_permission() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_four_tabs() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/SecretsPage.tsx",
            "AllTabs",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::VaultRead])).unwrap();
        // Anchored nav links exist.
        assert!(html.contains("#secrets-engines"));
        assert!(html.contains("#kv-browser"));
        assert!(html.contains("#access"));
        assert!(html.contains("#policies"));
        assert!(html.contains("#audit"));
        // Surface heads.
        assert!(html.contains("Secrets engines"));
        assert!(html.contains("Auth methods"));
        assert!(html.contains("Policies"));
        // Plaintext-protection banner.
        assert!(html.contains("Plaintext secret values are NEVER"));
    }

    #[test]
    fn render_links_upstream_vault_ui_docs() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/UpstreamLink.tsx",
            "UpstreamLink",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::VaultRead])).unwrap();
        assert!(html.contains("developer.hashicorp.com/vault/docs/configuration/ui"));
    }

    #[test]
    fn render_excludes_foreign_tenant_data() {
        // The seed includes an `evil` tenant row in every collection;
        // none of it must reach the acme view.
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/TenantIsolation.tsx",
            "TenantIsolation",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::VaultRead])).unwrap();
        assert!(!html.contains("evil-default"));
        assert!(!html.contains("auth_token_evil"));
    }
}
