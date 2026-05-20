// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Auth-methods tab — mirrors Vault's `Access` page
//! (`GET /v1/sys/auth`). Lists every mounted auth method
//! (`token`, `userpass`, `kubernetes`, `approle`, `oidc`, …).

use super::VaultViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{AdminState, VaultAuthMethod, scope};

pub fn list_auth_methods(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VaultAuthMethod>, VaultViewError> {
    ctx.authorise(Permission::VaultRead)?;
    let mut rows: Vec<VaultAuthMethod> = scope(
        &state.vault_auth_methods.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(rows)
}

/// Group methods by their `method_type` so the UI can show "you have
/// 3 OIDC mounts" at a glance. Returns `(method_type → count)` pairs
/// sorted by count desc.
pub fn group_by_type(methods: &[VaultAuthMethod]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for m in methods {
        *acc.entry(m.method_type.clone()).or_insert(0) += 1;
    }
    let mut out: Vec<(String, usize)> = acc.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, VaultViewError> {
    let rows = list_auth_methods(state, ctx)?;
    let groups = group_by_type(&rows);
    let group_html = groups
        .iter()
        .map(|(t, n)| {
            format!(
                r#"<span class="inline-block mr-3 px-2 py-1 rounded bg-gray-200">{t} <small>×{n}</small></span>"#,
                t = escape(t),
                n = n,
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|m| {
            vec![
                escape(&m.path),
                escape(&m.method_type),
                escape(&m.accessor),
                ttl_human(m.default_lease_ttl_s),
                if m.enabled {
                    "enabled".into()
                } else {
                    "disabled".into()
                },
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="access" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Auth methods ({n})</h2>
  <div class="mb-3 text-sm">{group_html}</div>
  {tbl}
</section>"#,
        n = rows.len(),
        group_html = group_html,
        tbl = table(
            &["path", "type", "accessor", "default TTL", "state"],
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
    use cave_kernel::ns::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_returns_only_owner_methods_sorted() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/AuthMethods.tsx",
            "AuthMethods",
            "acme"
        );
        let methods =
            list_auth_methods(&AdminState::seeded(), &ctx(&[Permission::VaultRead])).unwrap();
        assert_eq!(methods.len(), 5);
        let paths: Vec<&str> = methods.iter().map(|m| m.path.as_str()).collect();
        assert!(paths.windows(2).all(|w| w[0] <= w[1]));
        assert!(methods.iter().all(|m| m.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_permission() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_auth_methods(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_type_orders_by_count_descending_then_name() {
        let tenant = TenantId::new("t").unwrap();
        let methods = vec![
            VaultAuthMethod {
                tenant: tenant.clone(),
                path: "userpass-a/".into(),
                method_type: "userpass".into(),
                accessor: "a".into(),
                default_lease_ttl_s: 0,
                enabled: true,
            },
            VaultAuthMethod {
                tenant: tenant.clone(),
                path: "userpass-b/".into(),
                method_type: "userpass".into(),
                accessor: "b".into(),
                default_lease_ttl_s: 0,
                enabled: true,
            },
            VaultAuthMethod {
                tenant: tenant.clone(),
                path: "token/".into(),
                method_type: "token".into(),
                accessor: "t".into(),
                default_lease_ttl_s: 0,
                enabled: true,
            },
        ];
        let groups = group_by_type(&methods);
        assert_eq!(groups[0], ("userpass".into(), 2));
        assert_eq!(groups[1], ("token".into(), 1));
    }

    #[test]
    fn group_summary_renders_per_type_pill() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/AuthMethods.tsx",
            "GroupPills",
            "acme"
        );
        let html = render_section(&AdminState::seeded(), &ctx(&[Permission::VaultRead])).unwrap();
        for t in ["token", "userpass", "kubernetes", "approle", "oidc"] {
            assert!(html.contains(t), "missing pill for {t}");
        }
    }
}
