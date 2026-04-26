//! `/admin/vault` view — secret browser (metadata only) + audit log.
//!
//! No secret *values* ever leave Vault through this view; we only show
//! metadata (path, version, creation time) and the audit log. Mirrors
//! Backstage's `auth-react` plugin in spirit (read-only meta, never the
//! plaintext credential).

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, VaultAuditEntry, VaultSecretMeta};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VaultViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("vault values are never exposed through the admin view")]
    ValueAccessForbidden,
}

pub fn list_secrets(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VaultSecretMeta>, VaultViewError> {
    ctx.authorise(Permission::VaultRead)?;
    let mut rows: Vec<VaultSecretMeta> =
        scope(&state.vault_secrets.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(rows)
}

pub fn list_audit(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VaultAuditEntry>, VaultViewError> {
    ctx.authorise(Permission::VaultRead)?;
    let mut rows: Vec<VaultAuditEntry> =
        scope(&state.vault_audit.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| b.time_unix.cmp(&a.time_unix)); // newest first
    Ok(rows)
}

/// Hard rejection: never exposes secret values, even to a privileged caller.
/// Returns an error 100% of the time.
pub fn read_value(_path: &str) -> Result<String, VaultViewError> {
    Err(VaultViewError::ValueAccessForbidden)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, VaultViewError> {
    let secrets = list_secrets(state, ctx)?;
    let audit = list_audit(state, ctx)?;
    let s_rows: Vec<Vec<String>> = secrets
        .iter()
        .map(|s| vec![s.path.clone(), s.version.to_string(), s.created_unix.to_string()])
        .collect();
    let a_rows: Vec<Vec<String>> = audit
        .iter()
        .map(|a| vec![a.time_unix.to_string(), a.principal.clone(), a.op.into(), a.path.clone()])
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Secrets ({n_s}) <small class="text-gray-500">(metadata only)</small></h2>{s_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Audit ({n_a})</h2>{a_tbl}</section>"#,
        n_s = secrets.len(),
        n_a = audit.len(),
        s_tbl = table(&["path", "version", "created"], &s_rows),
        a_tbl = table(&["time", "principal", "op", "path"], &a_rows),
    );
    Ok(page_shell(
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
    fn list_secrets_returns_only_owner_metadata_sorted() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/SecretList.tsx",
            "SecretList",
            "acme"
        );
        let state = AdminState::seeded();
        let s = list_secrets(&state, &ctx(&[Permission::VaultRead])).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].path, "kv/api");
        assert_eq!(s[1].path, "kv/db");
    }

    #[test]
    fn list_audit_returns_newest_first() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/AuditLog.tsx",
            "AuditLog",
            "acme"
        );
        let state = AdminState::seeded();
        let a = list_audit(&state, &ctx(&[Permission::VaultRead])).unwrap();
        assert_eq!(a.len(), 2);
        assert!(a[0].time_unix >= a[1].time_unix);
    }

    #[test]
    fn read_value_always_returns_forbidden() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/SecretReveal.tsx",
            "RevealValue",
            "acme"
        );
        assert!(matches!(read_value("kv/db").unwrap_err(), VaultViewError::ValueAccessForbidden));
    }

    #[test]
    fn list_secrets_refuses_without_vault_read() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let state = AdminState::seeded();
        assert!(list_secrets(&state, &ctx(&[])).is_err());
    }

    #[test]
    fn render_page_advertises_metadata_only_and_omits_other_tenants() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/SecretsPage.tsx",
            "SecretsPage",
            "acme"
        );
        let state = AdminState::seeded();
        let html = render(&state, &ctx(&[Permission::VaultRead])).unwrap();
        assert!(html.contains("metadata only"));
        assert!(html.contains("kv/db"));
        assert!(!html.contains("kv/secret")); // foreign tenant
    }
}
