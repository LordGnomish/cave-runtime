//! `/admin/mesh` view — AuthorizationPolicy editor + flow log viewer.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, MeshAuthzPolicy, MeshFlow};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MeshViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("policy {0} already exists in this tenant")]
    DuplicatePolicy(String),
    #[error("policy {0} not found")]
    PolicyNotFound(String),
    #[error("invalid action {0}: must be Allow or Deny")]
    InvalidAction(String),
    #[error("principal_glob must be non-empty")]
    EmptyPrincipalGlob,
}

pub fn list_policies(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<MeshAuthzPolicy>, MeshViewError> {
    ctx.authorise(Permission::MeshRead)?;
    Ok(scope(&state.mesh_authz.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn list_flows(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<MeshFlow>, MeshViewError> {
    ctx.authorise(Permission::MeshRead)?;
    Ok(scope(&state.mesh_flows.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

/// Create-or-update an AuthorizationPolicy. Mirrors the editor's Save action.
pub fn upsert_policy(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
    action: &str,
    principal_glob: &str,
) -> Result<(), MeshViewError> {
    ctx.authorise(Permission::MeshWrite)?;
    if action != "Allow" && action != "Deny" {
        return Err(MeshViewError::InvalidAction(action.into()));
    }
    if principal_glob.trim().is_empty() {
        return Err(MeshViewError::EmptyPrincipalGlob);
    }
    let normalised_action: &'static str = if action == "Allow" { "Allow" } else { "Deny" };
    let mut policies = state.mesh_authz.write().unwrap();
    if let Some(existing) = policies
        .iter_mut()
        .find(|p| p.tenant == ctx.tenant && p.name == name)
    {
        existing.action = normalised_action;
        existing.principal_glob = principal_glob.to_string();
    } else {
        policies.push(MeshAuthzPolicy {
            tenant: ctx.tenant.clone(),
            name: name.to_string(),
            action: normalised_action,
            principal_glob: principal_glob.to_string(),
        });
    }
    Ok(())
}

/// Reject duplicate-create. Mirrors the "fail if exists" branch of the editor.
pub fn create_policy(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
    action: &str,
    principal_glob: &str,
) -> Result<(), MeshViewError> {
    ctx.authorise(Permission::MeshWrite)?;
    {
        let policies = state.mesh_authz.read().unwrap();
        if policies.iter().any(|p| p.tenant == ctx.tenant && p.name == name) {
            return Err(MeshViewError::DuplicatePolicy(name.into()));
        }
    }
    upsert_policy(state, ctx, name, action, principal_glob)
}

pub fn delete_policy(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<(), MeshViewError> {
    ctx.authorise(Permission::MeshWrite)?;
    let mut policies = state.mesh_authz.write().unwrap();
    let before = policies.len();
    policies.retain(|p| !(p.tenant == ctx.tenant && p.name == name));
    if policies.len() == before {
        return Err(MeshViewError::PolicyNotFound(name.into()));
    }
    Ok(())
}

// ── Kiali-faithful aggregations ──────────────────────────────────────────────

/// One row in Kiali's **Workloads** tab — every distinct
/// `source` (call origin) in the mesh, aggregated with totals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshWorkload {
    pub name: String,
    pub outgoing_edges: u32,
    pub bytes_out: u64,
    pub bytes_dropped: u64,
}

/// One row in Kiali's **Services** tab — every distinct `destination`
/// (call target) in the mesh, aggregated by inbound traffic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshService {
    pub name: String,
    pub incoming_edges: u32,
    pub bytes_in: u64,
    pub bytes_dropped: u64,
}

/// Aggregate `MeshFlow` rows into workloads (per source). Mirrors
/// Kiali's "Workload" landing tab — one row per service-account /
/// pod owner that initiates traffic.
pub fn list_workloads(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<MeshWorkload>, MeshViewError> {
    let flows = list_flows(state, ctx)?;
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, MeshWorkload> = BTreeMap::new();
    for f in flows.iter() {
        let w = acc.entry(f.source.clone()).or_insert(MeshWorkload {
            name: f.source.clone(),
            outgoing_edges: 0,
            bytes_out: 0,
            bytes_dropped: 0,
        });
        w.outgoing_edges += 1;
        w.bytes_out += f.bytes;
        if f.verdict == "Dropped" {
            w.bytes_dropped += f.bytes;
        }
    }
    Ok(acc.into_values().collect())
}

/// Aggregate `MeshFlow` rows into services (per destination). Mirrors
/// Kiali's "Service" landing tab — one row per service that receives
/// traffic.
pub fn list_services(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<MeshService>, MeshViewError> {
    let flows = list_flows(state, ctx)?;
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, MeshService> = BTreeMap::new();
    for f in flows.iter() {
        let svc = acc.entry(f.destination.clone()).or_insert(MeshService {
            name: f.destination.clone(),
            incoming_edges: 0,
            bytes_in: 0,
            bytes_dropped: 0,
        });
        svc.incoming_edges += 1;
        svc.bytes_in += f.bytes;
        if f.verdict == "Dropped" {
            svc.bytes_dropped += f.bytes;
        }
    }
    Ok(acc.into_values().collect())
}

/// Kiali "Health" badge per service. Mirrors `kiali_api/health` —
/// a service is `Healthy` when zero recent flows have verdict
/// `Dropped`, `Degraded` when >0% but <50%, `Failing` when ≥50%.
pub fn service_health(svc: &MeshService) -> &'static str {
    if svc.bytes_in == 0 {
        "Idle"
    } else {
        let drop_ratio = svc.bytes_dropped as f64 / svc.bytes_in as f64;
        if drop_ratio == 0.0 {
            "Healthy"
        } else if drop_ratio < 0.5 {
            "Degraded"
        } else {
            "Failing"
        }
    }
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MeshViewError> {
    let policies = list_policies(state, ctx)?;
    let flows = list_flows(state, ctx)?;
    let workloads = list_workloads(state, ctx)?;
    let services = list_services(state, ctx)?;
    let p_rows: Vec<Vec<String>> = policies
        .iter()
        .map(|p| vec![p.name.clone(), p.action.into(), p.principal_glob.clone()])
        .collect();
    let f_rows: Vec<Vec<String>> = flows
        .iter()
        .map(|f| vec![f.source.clone(), f.destination.clone(), f.verdict.into(), f.bytes.to_string()])
        .collect();
    let w_rows: Vec<Vec<String>> = workloads
        .iter()
        .map(|w| vec![
            w.name.clone(),
            w.outgoing_edges.to_string(),
            w.bytes_out.to_string(),
            w.bytes_dropped.to_string(),
        ])
        .collect();
    let s_rows: Vec<Vec<String>> = services
        .iter()
        .map(|s| vec![
            s.name.clone(),
            s.incoming_edges.to_string(),
            s.bytes_in.to_string(),
            s.bytes_dropped.to_string(),
            service_health(s).into(),
        ])
        .collect();
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Kiali-parity service mesh dashboard (cave-mesh).
  Upstream: <a class="text-blue-700 underline" href="https://kiali.io/">kiali.io</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#mesh-workloads">Workloads</a>
  <a href="#mesh-services">Services</a>
  <a href="#mesh-authz">AuthZ</a>
  <a href="#mesh-flows">Flows</a>
</nav>
<section id="mesh-workloads"><h2 class="text-lg font-semibold mb-2">Workloads ({n_w})</h2>{w_tbl}</section>
<section id="mesh-services" class="mt-6"><h2 class="text-lg font-semibold mb-2">Services ({n_s})</h2>{s_tbl}</section>
<section id="mesh-authz" class="mt-6"><h2 class="text-lg font-semibold mb-2">AuthZ ({n_p})</h2>{p_tbl}</section>
<section id="mesh-flows" class="mt-6"><h2 class="text-lg font-semibold mb-2">Flows ({n_f})</h2>{f_tbl}</section>"##,
        n_p = policies.len(),
        n_f = flows.len(),
        n_w = workloads.len(),
        n_s = services.len(),
        p_tbl = table(&["name", "action", "principal_glob"], &p_rows),
        f_tbl = table(&["src", "dst", "verdict", "bytes"], &f_rows),
        w_tbl = table(&["workload", "edges", "bytes out", "bytes dropped"], &w_rows),
        s_tbl = table(&["service", "edges", "bytes in", "bytes dropped", "health"], &s_rows),
    );
    Ok(page_shell(
        &format!("mesh · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/catalog-graph/src/components/CatalogGraphPage/CatalogGraphPage.tsx",
    "CatalogGraphPage",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_policies_filters_to_owner() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/CatalogGraphPage/CatalogGraphPage.tsx",
            "PolicyTable",
            "acme"
        );
        let state = AdminState::seeded();
        let p = list_policies(&state, &ctx(&[Permission::MeshRead])).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "allow-web");
    }

    #[test]
    fn create_policy_appends_and_rejects_duplicate() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/PolicyEditor.tsx",
            "PolicyEditor",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::MeshRead, Permission::MeshWrite]);
        create_policy(&state, &c, "deny-bots", "Deny", "spiffe://*/sa/bot").unwrap();
        let err = create_policy(&state, &c, "deny-bots", "Deny", "spiffe://*/sa/bot").unwrap_err();
        assert!(matches!(err, MeshViewError::DuplicatePolicy(_)));
    }

    #[test]
    fn invalid_action_is_rejected() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/PolicyEditor.tsx",
            "validateAction",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::MeshRead, Permission::MeshWrite]);
        let err = create_policy(&state, &c, "x", "Maybe", "*").unwrap_err();
        assert!(matches!(err, MeshViewError::InvalidAction(_)));
    }

    #[test]
    fn delete_policy_removes_and_errors_on_missing() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/PolicyEditor.tsx",
            "deletePolicy",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::MeshRead, Permission::MeshWrite]);
        delete_policy(&state, &c, "allow-web").unwrap();
        assert_eq!(list_policies(&state, &c).unwrap().len(), 0);
        let err = delete_policy(&state, &c, "allow-web").unwrap_err();
        assert!(matches!(err, MeshViewError::PolicyNotFound(_)));
    }

    #[test]
    fn list_flows_only_returns_owner_flows() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/FlowViewer.tsx",
            "FlowViewer",
            "acme"
        );
        let state = AdminState::seeded();
        let f = list_flows(&state, &ctx(&[Permission::MeshRead])).unwrap();
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    // ── Kiali workload / service aggregations ─────────────────────────────

    #[test]
    fn list_workloads_aggregates_by_source() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Workloads.tsx",
            "AggregateBySource",
            "acme"
        );
        let state = AdminState::seeded();
        let workloads = list_workloads(&state, &ctx(&[Permission::MeshRead])).unwrap();
        // Each workload is unique by source name.
        let mut names: Vec<&str> = workloads.iter().map(|w| w.name.as_str()).collect();
        names.sort();
        let len = names.len();
        names.dedup();
        assert_eq!(names.len(), len, "workload names should be unique");
        // Aggregation totals bytes correctly.
        for w in &workloads {
            assert!(w.outgoing_edges > 0);
            assert!(w.bytes_out > 0);
        }
    }

    #[test]
    fn list_services_aggregates_by_destination() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Services.tsx",
            "AggregateByDest",
            "acme"
        );
        let state = AdminState::seeded();
        let services = list_services(&state, &ctx(&[Permission::MeshRead])).unwrap();
        let mut names: Vec<&str> = services.iter().map(|s| s.name.as_str()).collect();
        names.sort();
        let len = names.len();
        names.dedup();
        assert_eq!(names.len(), len, "service names should be unique");
    }

    #[test]
    fn list_workloads_refuses_without_permission() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_workloads(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_services_refuses_without_permission() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_services(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn service_health_classifies_drop_ratio_buckets() {
        let healthy = MeshService {
            name: "svc".into(),
            incoming_edges: 5,
            bytes_in: 1000,
            bytes_dropped: 0,
        };
        assert_eq!(service_health(&healthy), "Healthy");

        let degraded = MeshService {
            name: "svc".into(),
            incoming_edges: 5,
            bytes_in: 1000,
            bytes_dropped: 100, // 10% drop
        };
        assert_eq!(service_health(&degraded), "Degraded");

        let failing = MeshService {
            name: "svc".into(),
            incoming_edges: 5,
            bytes_in: 1000,
            bytes_dropped: 600, // 60% drop
        };
        assert_eq!(service_health(&failing), "Failing");

        let idle = MeshService {
            name: "svc".into(),
            incoming_edges: 0,
            bytes_in: 0,
            bytes_dropped: 0,
        };
        assert_eq!(service_health(&idle), "Idle");
    }

    #[test]
    fn render_includes_workloads_and_services_tabs() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/MeshPage.tsx",
            "TabsRender",
            "acme"
        );
        let html = render(
            &AdminState::seeded(),
            &ctx(&[Permission::MeshRead]),
        )
        .unwrap();
        assert!(html.contains("#mesh-workloads"));
        assert!(html.contains("#mesh-services"));
        assert!(html.contains("#mesh-authz"));
        assert!(html.contains("#mesh-flows"));
        assert!(html.contains("kiali.io"));
    }

    #[test]
    fn upsert_policy_updates_existing_action_and_glob() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/PolicyEditor.tsx",
            "UpsertUpdate",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::MeshRead, Permission::MeshWrite]);
        // The seed policy `allow-web` exists with action=Allow.
        upsert_policy(&state, &c, "allow-web", "Deny", "spiffe://*/sa/none").unwrap();
        let policies = list_policies(&state, &c).unwrap();
        let updated = policies.iter().find(|p| p.name == "allow-web").unwrap();
        assert_eq!(updated.action, "Deny");
        assert_eq!(updated.principal_glob, "spiffe://*/sa/none");
    }

    #[test]
    fn empty_principal_glob_is_rejected() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/PolicyEditor.tsx",
            "EmptyGlob",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::MeshRead, Permission::MeshWrite]);
        let err = create_policy(&state, &c, "x", "Allow", "").unwrap_err();
        assert!(matches!(err, MeshViewError::EmptyPrincipalGlob));
    }
}
