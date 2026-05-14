// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/apiserver` view — RBAC-filtered Kubernetes resource browser.
//!
//! Mirrors the `kubernetes-resources` panes Backstage's Kubernetes plugin
//! exposes, gated through cave's permission model so a viewer cannot see
//! resources they don't have read access to.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, K8sResource};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ApiserverViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

/// List resources, optionally filtered by `kind`.
pub fn list_resources(
    state: &AdminState,
    ctx: &RequestCtx,
    kind_filter: Option<&str>,
) -> Result<Vec<K8sResource>, ApiserverViewError> {
    ctx.authorise(Permission::ApiserverRead)?;
    let all = state.k8s_resources.read().unwrap();
    let mut rows: Vec<K8sResource> = scope(&all, &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .filter(|r| match kind_filter {
            Some(k) => r.kind == k,
            None => true,
        })
        .cloned()
        .collect();
    rows.sort_by(|a, b| (a.kind.as_str(), a.name.as_str()).cmp(&(b.kind.as_str(), b.name.as_str())));
    Ok(rows)
}

/// Distinct resource kinds visible to this caller — drives the filter chips.
pub fn distinct_kinds(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<String>, ApiserverViewError> {
    let rows = list_resources(state, ctx, None)?;
    let mut kinds: Vec<String> = rows.iter().map(|r| r.kind.clone()).collect();
    kinds.sort();
    kinds.dedup();
    Ok(kinds)
}

pub fn render(state: &AdminState, ctx: &RequestCtx, kind: Option<&str>) -> Result<String, ApiserverViewError> {
    let rows = list_resources(state, ctx, kind)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![r.kind.clone(), r.name.clone(), r.namespace.clone()])
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Resources ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["kind", "name", "namespace"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/apiserver",
        &format!("apiserver · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/EmptyResponses/PodsEmptyState.tsx",
    "ResourceList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_resources_returns_only_owner_rows_sorted() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/Resources.tsx",
            "ResourceList",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = list_resources(&state, &ctx(&[Permission::ApiserverRead]), None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "Deployment");
        assert_eq!(rows[1].kind, "Service");
        assert!(rows.iter().all(|r| r.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_resources_kind_filter_is_applied() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/Resources.tsx",
            "filterByKind",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = list_resources(&state, &ctx(&[Permission::ApiserverRead]), Some("Service")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "web");
    }

    #[test]
    fn list_resources_refuses_without_apiserver_read() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let state = AdminState::seeded();
        assert!(list_resources(&state, &ctx(&[]), None).is_err());
    }

    #[test]
    fn distinct_kinds_dedups_and_sorts() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/KindFilter.tsx",
            "KindFilter",
            "acme"
        );
        let state = AdminState::seeded();
        let kinds = distinct_kinds(&state, &ctx(&[Permission::ApiserverRead])).unwrap();
        assert_eq!(kinds, vec!["Deployment".to_string(), "Service".to_string()]);
    }

    #[test]
    fn render_page_omits_foreign_tenant_rows() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/ResourcesPage.tsx",
            "ResourcesPage",
            "acme"
        );
        let state = AdminState::seeded();
        let html = render(&state, &ctx(&[Permission::ApiserverRead]), None).unwrap();
        assert!(html.contains("Resources (2)"));
        assert!(!html.contains("evil-web"));
    }
}
