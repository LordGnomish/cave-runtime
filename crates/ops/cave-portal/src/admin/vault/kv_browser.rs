// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! KV-v2 browser — the legacy `list_secrets` surface, kept under the
//! folder split so older callers keep compiling and the new
//! folder-shaped module owns every Vault concern in one place.

use super::VaultViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{AdminState, VaultSecretMeta, scope};

pub fn list_secrets(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VaultSecretMeta>, VaultViewError> {
    ctx.authorise(Permission::VaultRead)?;
    let mut rows: Vec<VaultSecretMeta> =
        scope(&state.vault_secrets.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(rows)
}

/// Filter secrets to those whose path begins with `prefix`. Mirrors
/// Vault's `vault kv list <prefix>` shape — useful for browsing one
/// engine mount at a time (e.g. `kv/`).
pub fn list_under(
    state: &AdminState,
    ctx: &RequestCtx,
    prefix: &str,
) -> Result<Vec<VaultSecretMeta>, VaultViewError> {
    let all = list_secrets(state, ctx)?;
    Ok(all
        .into_iter()
        .filter(|s| s.path.starts_with(prefix))
        .collect())
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, VaultViewError> {
    let rows = list_secrets(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|s| {
            vec![
                escape(&s.path),
                format!("v{}", s.version),
                s.created_unix.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kv-browser" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">KV browser ({n}) <small class="text-gray-500">(metadata only)</small></h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["path", "version", "created"], &table_rows),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_returns_only_owner_metadata_sorted() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/KvList.tsx",
            "KvList",
            "acme"
        );
        let s = AdminState::seeded();
        let secrets = list_secrets(&s, &ctx(&[Permission::VaultRead])).unwrap();
        assert_eq!(secrets.len(), 2);
        assert_eq!(secrets[0].path, "kv/api");
        assert_eq!(secrets[1].path, "kv/db");
    }

    #[test]
    fn list_refuses_without_vault_read() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_secrets(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_under_prefix_narrows_results() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/KvUnder.tsx",
            "Under",
            "acme"
        );
        let s = AdminState::seeded();
        let kv = list_under(&s, &ctx(&[Permission::VaultRead]), "kv/").unwrap();
        assert_eq!(kv.len(), 2);
        let foo = list_under(&s, &ctx(&[Permission::VaultRead]), "foo/").unwrap();
        assert!(foo.is_empty());
    }

    #[test]
    fn list_under_does_not_leak_other_tenant() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/KvTenant.tsx",
            "Tenant",
            "acme"
        );
        let s = AdminState::seeded();
        let any = list_under(&s, &ctx(&[Permission::VaultRead]), "kv/").unwrap();
        assert!(any.iter().all(|x| !x.path.contains("secret")));
    }
}
