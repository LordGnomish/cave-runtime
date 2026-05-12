//! Policies tab — mirrors Vault's `Policies` page
//! (`GET /v1/sys/policies/acl/<name>`). Lists every named policy for
//! the tenant; each row shows the bound-token count so the operator
//! can spot dead policies and avoid orphaning tokens by deleting an
//! in-use policy.

use super::VaultViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{scope, AdminState, VaultPolicy};

pub fn list_policies(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VaultPolicy>, VaultViewError> {
    ctx.authorise(Permission::VaultRead)?;
    let mut rows: Vec<VaultPolicy> = scope(
        &state.vault_policies.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

/// Find one policy by name. Returns `None` if the caller's tenant
/// does not own a policy by that name.
pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<Option<VaultPolicy>, VaultViewError> {
    let rows = list_policies(state, ctx)?;
    Ok(rows.into_iter().find(|p| p.name == name))
}

/// Policies with `bound_token_count == 0` are candidates for deletion
/// without orphaning access. The UI surfaces these as a separate
/// "unused" callout (defensive — same pattern as Vault's "0 tokens"
/// warning on the policies page).
pub fn unused(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<VaultPolicy>, VaultViewError> {
    let rows = list_policies(state, ctx)?;
    Ok(rows.into_iter().filter(|p| p.bound_token_count == 0).collect())
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, VaultViewError> {
    let rows = list_policies(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|p| {
            vec![
                escape(&p.name),
                p.bound_token_count.to_string(),
                // Truncate rules to a single line for the table; the
                // detail drill-down will show full HCL.
                escape(&truncate_rules(&p.rules, 80)),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="policies" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Policies ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["name", "bound tokens", "rules (truncated)"], &table_rows),
    ))
}

fn truncate_rules(rules: &str, max: usize) -> String {
    let one_line = rules.replace('\n', " ").trim().to_string();
    if one_line.chars().count() <= max {
        one_line
    } else {
        let cut: String = one_line.chars().take(max).collect();
        format!("{cut}…")
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
    fn list_returns_owner_policies_sorted_by_name() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/Policies.tsx",
            "Policies",
            "acme"
        );
        let s = AdminState::seeded();
        let policies = list_policies(&s, &ctx(&[Permission::VaultRead])).unwrap();
        let names: Vec<&str> = policies.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["db-admin", "default", "pki-ca"]);
        assert!(policies.iter().all(|p| p.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_permission() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_policies(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn detail_returns_policy_by_name() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/PolicyDetail.tsx",
            "Detail",
            "acme"
        );
        let s = AdminState::seeded();
        let p = detail(&s, &ctx(&[Permission::VaultRead]), "default")
            .unwrap()
            .expect("default should exist");
        assert!(p.rules.contains("kv/data/*"));
    }

    #[test]
    fn detail_returns_none_for_missing() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/PolicyDetail.tsx",
            "DetailMissing",
            "acme"
        );
        assert!(detail(&AdminState::seeded(), &ctx(&[Permission::VaultRead]), "no-such")
            .unwrap()
            .is_none());
    }

    #[test]
    fn unused_filters_zero_token_policies() {
        // Seed: every acme policy has bound_token_count > 0, so
        // `unused` is empty in the default state.
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/PolicyUnused.tsx",
            "UnusedEmpty",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(unused(&s, &ctx(&[Permission::VaultRead])).unwrap().is_empty());
    }

    #[test]
    fn truncate_rules_caps_long_strings() {
        let long = "path \"kv/*\" { capabilities = [\"read\", \"write\", \"create\", \"update\", \"delete\", \"list\"] }";
        let truncated = truncate_rules(long, 30);
        assert!(truncated.ends_with('…'));
        // chars count <= max + 1 (for the ellipsis).
        assert!(truncated.chars().count() <= 31);
    }
}
