// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Secrets-engines tab — mirrors Vault's `Secrets` page
//! (`GET /v1/sys/mounts`). Lists every mounted engine for the
//! caller's tenant.

use super::VaultViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{scope, AdminState, VaultSecretsEngine};

pub fn list_secrets_engines(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VaultSecretsEngine>, VaultViewError> {
    ctx.authorise(Permission::VaultRead)?;
    let mut rows: Vec<VaultSecretsEngine> = scope(
        &state.vault_engines.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(rows)
}

/// Look up a single engine by its mount path (without trailing slash
/// normalisation — pass the same path Vault uses). Returns `None` if
/// the caller's tenant does not own the path.
pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    path: &str,
) -> Result<Option<VaultSecretsEngine>, VaultViewError> {
    let rows = list_secrets_engines(state, ctx)?;
    Ok(rows.into_iter().find(|e| e.path == path))
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, VaultViewError> {
    let rows = list_secrets_engines(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|e| {
            vec![
                escape(&e.path),
                escape(&e.engine_type),
                format!("v{}", e.version),
                ttl_human(e.default_lease_ttl_s),
                if e.enabled { "enabled".into() } else { "disabled".into() },
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="secrets-engines" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Secrets engines ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["path", "type", "version", "default TTL", "state"],
            &table_rows
        ),
    ))
}

fn ttl_human(s: u64) -> String {
    match s {
        0 => "system".into(),
        s if s % 86400 == 0 => format!("{}d", s / 86400),
        s if s % 3600 == 0 => format!("{}h", s / 3600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_returns_only_owner_engines_sorted_by_path() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/Engines.tsx",
            "Engines",
            "acme"
        );
        let s = AdminState::seeded();
        let engines = list_secrets_engines(&s, &ctx(&[Permission::VaultRead])).unwrap();
        // Acme has 5 seeded engines (4 enabled + 1 disabled), evil
        // has its own kv that must not leak.
        assert_eq!(engines.len(), 5, "acme engines: {engines:?}");
        let paths: Vec<&str> = engines.iter().map(|e| e.path.as_str()).collect();
        // Sorted.
        assert!(paths.windows(2).all(|w| w[0] <= w[1]));
        // No foreign tenant.
        assert!(engines.iter().all(|e| e.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_vault_read() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_secrets_engines(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn detail_returns_engine_for_owned_path() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/EngineDetail.tsx",
            "Detail",
            "acme"
        );
        let s = AdminState::seeded();
        let e = detail(&s, &ctx(&[Permission::VaultRead]), "transit/")
            .unwrap()
            .expect("transit/ should exist");
        assert_eq!(e.engine_type, "transit");
    }

    #[test]
    fn detail_returns_none_for_missing_path() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/EngineDetail.tsx",
            "DetailMissing",
            "acme"
        );
        let s = AdminState::seeded();
        let e = detail(&s, &ctx(&[Permission::VaultRead]), "no-such/")
            .unwrap();
        assert!(e.is_none());
    }

    #[test]
    fn disabled_engines_are_visible_with_disabled_state() {
        // The seed has a `legacy-kv/` engine with enabled=false.
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/EngineState.tsx",
            "DisabledVisible",
            "acme"
        );
        let html = render_section(
            &AdminState::seeded(),
            &ctx(&[Permission::VaultRead]),
        )
        .unwrap();
        assert!(html.contains("legacy-kv/"));
        assert!(html.contains("disabled"));
    }

    #[test]
    fn ttl_human_renders_common_buckets() {
        assert_eq!(ttl_human(0), "system");
        assert_eq!(ttl_human(60), "1m");
        assert_eq!(ttl_human(3600), "1h");
        assert_eq!(ttl_human(86400), "1d");
        assert_eq!(ttl_human(90), "90s");
    }
}
