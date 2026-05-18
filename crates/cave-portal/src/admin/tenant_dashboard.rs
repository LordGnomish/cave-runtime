// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/t/{tenant}/dashboard` overview — resource summary + recent activity.
//!
//! Pulls counts from every other admin view's data source, plus the
//! activity feed. Mirrors Backstage's `ExplorePage` shape.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, tally_by_kind, ActivityEntry, AdminState};
use crate::admin::types::{Cite, TenantId};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DashboardError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("dashboard tenant {requested} does not match request tenant {actual}")]
    PathTenantMismatch { requested: TenantId, actual: TenantId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSummary {
    pub kv_count: usize,
    pub sandbox_count: usize,
    pub k8s_count: usize,
    pub user_count: usize,
    pub policy_count: usize,
    pub table_count: usize,
    pub secret_count: usize,
}

/// Compute the per-tenant resource summary by scoping every collection.
pub fn resource_summary(state: &AdminState, ctx: &RequestCtx) -> Result<ResourceSummary, DashboardError> {
    ctx.authorise(Permission::DashboardRead)?;
    let kv_count = scope(&state.etcd_kv.read().unwrap(), &ctx.tenant, |r| &r.tenant).len();
    let sandbox_count = scope(&state.cri_sandboxes.read().unwrap(), &ctx.tenant, |r| &r.tenant).len();
    let k8s_count = scope(&state.k8s_resources.read().unwrap(), &ctx.tenant, |r| &r.tenant).len();
    let user_count = scope(&state.iam_users.read().unwrap(), &ctx.tenant, |r| &r.tenant).len();
    let policy_count = scope(&state.mesh_authz.read().unwrap(), &ctx.tenant, |r| &r.tenant).len();
    let table_count = scope(&state.pg_tables.read().unwrap(), &ctx.tenant, |r| &r.tenant).len();
    let secret_count = scope(&state.vault_secrets.read().unwrap(), &ctx.tenant, |r| &r.tenant).len();
    Ok(ResourceSummary {
        kv_count,
        sandbox_count,
        k8s_count,
        user_count,
        policy_count,
        table_count,
        secret_count,
    })
}

/// Recent activity (most recent first), capped to `limit`.
pub fn recent_activity(
    state: &AdminState,
    ctx: &RequestCtx,
    limit: usize,
) -> Result<Vec<ActivityEntry>, DashboardError> {
    ctx.authorise(Permission::DashboardRead)?;
    let log = state.recent_activity.read().unwrap();
    let mut rows: Vec<ActivityEntry> =
        scope(&log, &ctx.tenant, |r| &r.tenant).into_iter().cloned().collect();
    rows.sort_by(|a, b| b.when_unix.cmp(&a.when_unix));
    rows.truncate(limit);
    Ok(rows)
}

/// Recent activity tally — useful for the small "by kind" badge row.
pub fn activity_by_kind(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<BTreeMap<&'static str, u64>, DashboardError> {
    ctx.authorise(Permission::DashboardRead)?;
    let log = state.recent_activity.read().unwrap();
    Ok(tally_by_kind(&log, &ctx.tenant))
}

/// Render the full dashboard. `path_tenant` is what was in the URL —
/// must match `ctx.tenant` to defeat URL spoofing.
pub fn render(
    state: &AdminState,
    ctx: &RequestCtx,
    path_tenant: &TenantId,
) -> Result<String, DashboardError> {
    if path_tenant != &ctx.tenant {
        return Err(DashboardError::PathTenantMismatch {
            requested: path_tenant.clone(),
            actual: ctx.tenant.clone(),
        });
    }
    let summary = resource_summary(state, ctx)?;
    let recent = recent_activity(state, ctx, 20)?;
    let summary_rows = vec![
        vec!["etcd KV".into(), summary.kv_count.to_string()],
        vec!["CRI sandboxes".into(), summary.sandbox_count.to_string()],
        vec!["k8s resources".into(), summary.k8s_count.to_string()],
        vec!["IAM users".into(), summary.user_count.to_string()],
        vec!["AuthZ policies".into(), summary.policy_count.to_string()],
        vec!["PG tables".into(), summary.table_count.to_string()],
        vec!["Vault secrets".into(), summary.secret_count.to_string()],
    ];
    let activity_rows: Vec<Vec<String>> = recent
        .iter()
        .map(|a| vec![a.when_unix.to_string(), a.kind.into(), a.summary.clone()])
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Summary</h2>{s_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Recent activity ({n})</h2>{a_tbl}</section>"#,
        s_tbl = table(&["resource", "count"], &summary_rows),
        n = recent.len(),
        a_tbl = table(&["time", "kind", "summary"], &activity_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/dashboard",
        &format!("dashboard · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/explore/src/components/ExplorePage.tsx",
    "ExplorePage",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn resource_summary_counts_only_owner_rows() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let state = AdminState::seeded();
        let s = resource_summary(&state, &ctx(&[Permission::DashboardRead])).unwrap();
        assert_eq!(s.kv_count, 2);
        assert_eq!(s.sandbox_count, 2);
        assert_eq!(s.k8s_count, 2);
        assert_eq!(s.user_count, 2);
        assert_eq!(s.policy_count, 1);
        assert_eq!(s.table_count, 2);
        assert_eq!(s.secret_count, 2);
    }

    #[test]
    fn recent_activity_returns_sorted_and_truncated() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ActivityFeed.tsx",
            "ActivityFeed",
            "acme"
        );
        let state = AdminState::seeded();
        let r = recent_activity(&state, &ctx(&[Permission::DashboardRead]), 1).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].when_unix, 1_000_200);
    }

    #[test]
    fn activity_by_kind_tallies_per_tenant() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ActivityFeed.tsx",
            "tally",
            "acme"
        );
        let state = AdminState::seeded();
        let m = activity_by_kind(&state, &ctx(&[Permission::DashboardRead])).unwrap();
        assert_eq!(m.get("deploy").copied(), Some(1));
        assert_eq!(m.get("policy").copied(), Some(1));
        // `evil` deploy must not leak in.
        assert_eq!(m.values().copied().sum::<u64>(), 2);
    }

    #[test]
    fn render_refuses_path_tenant_that_differs_from_request_tenant() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "tenantUrlGuard",
            "acme"
        );
        let state = AdminState::seeded();
        let err = render(
            &state,
            &ctx(&[Permission::DashboardRead]),
            &TenantId::new("evil").expect("test fixture"),
        )
        .unwrap_err();
        assert!(matches!(err, DashboardError::PathTenantMismatch { .. }));
    }

    #[test]
    fn render_includes_summary_and_activity_sections() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "render",
            "acme"
        );
        let state = AdminState::seeded();
        let html = render(
            &state,
            &ctx(&[Permission::DashboardRead]),
            &TenantId::new("acme").expect("test fixture"),
        )
        .unwrap();
        assert!(html.contains("Summary"));
        assert!(html.contains("Recent activity"));
        assert!(html.contains("AuthZ policies"));
    }
}
