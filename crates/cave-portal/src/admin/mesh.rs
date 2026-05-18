// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/mesh` view — AuthorizationPolicy editor + flow log viewer.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
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

/// One unified service-mesh dashboard. 2026-05-14 consolidation:
/// `/admin/kiali` and `/admin/net` now 308-redirect into this page
/// with anchor hashes (`#kiali-topology`, `#net-flows`, etc.) so
/// existing deep-links still land on the right tab. The renderer
/// composes mesh's own AuthZ + Flows + Workloads + Services PLUS
/// kiali's exclusive Topology / Traffic / Validations PLUS net's
/// exclusive NetworkPolicies / Nodes / Identities sections.
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
    // 2026-05-14 consolidation: pull in kiali's exclusive sections
    // (Topology, Traffic, Validations) + net's exclusive sections
    // (NetworkPolicies, Nodes, Identities). Each is gated on the
    // appropriate permission; if a caller is missing one, the
    // corresponding render_section returns an Auth error which we
    // silently swallow so the page still renders the parts the
    // caller can see (matches the dashboard pattern elsewhere).
    let topology_html = crate::admin::kiali::topology::render_section(state, ctx)
        .unwrap_or_default();
    let traffic_html = crate::admin::kiali::traffic::render_section(state, ctx)
        .unwrap_or_default();
    let validations_html = crate::admin::kiali::validations::render_section(state, ctx)
        .unwrap_or_default();
    let net_flows_html = crate::admin::net::flows::render_section(state, ctx)
        .unwrap_or_default();
    let net_policies_html = crate::admin::net::policies::render_section(state, ctx)
        .unwrap_or_default();
    let net_nodes_html = crate::admin::net::nodes::render_section(state, ctx)
        .unwrap_or_default();
    let net_identities_html = crate::admin::net::identities::render_section(state, ctx)
        .unwrap_or_default();

    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Unified service-mesh dashboard (cave-mesh + cave-net + Kiali parity).
  Upstreams: <a class="text-blue-700 underline" href="https://istio.io/">istio.io</a>,
  <a class="text-blue-700 underline" href="https://kiali.io/">kiali.io</a>,
  <a class="text-blue-700 underline" href="https://docs.cilium.io/en/stable/observability/hubble/hubble-ui/">cilium hubble</a>.
  Note: <code>/admin/kiali</code> and <code>/admin/net</code> 308-redirect here.
</section>
<nav class="mb-4 flex gap-4 flex-wrap text-sm text-blue-700">
  <a href="#kiali-topology">Topology</a>
  <a href="#mesh-workloads">Workloads</a>
  <a href="#mesh-services">Services</a>
  <a href="#kiali-traffic">Traffic</a>
  <a href="#mesh-authz">AuthZ</a>
  <a href="#mesh-flows">Mesh Flows</a>
  <a href="#net-flows">Network Flows</a>
  <a href="#net-policies">NetworkPolicies</a>
  <a href="#kiali-validations">Validations</a>
  <a href="#net-nodes">Nodes</a>
  <a href="#net-identities">Identities</a>
</nav>
{topology}
<section id="mesh-workloads"><span id="kiali-workloads"></span><h2 class="text-lg font-semibold mb-2">Workloads ({n_w})</h2>{w_tbl}</section>
<section id="mesh-services" class="mt-6"><span id="kiali-services"></span><span id="net-services"></span><h2 class="text-lg font-semibold mb-2">Services ({n_s})</h2>{s_tbl}</section>
{traffic}
<section id="mesh-authz" class="mt-6"><h2 class="text-lg font-semibold mb-2">AuthZ ({n_p})</h2>{p_tbl}</section>
<section id="mesh-flows" class="mt-6"><h2 class="text-lg font-semibold mb-2">Mesh Flows ({n_f})</h2>{f_tbl}</section>
{net_flows}
{net_policies}
{validations}
{net_nodes}
{net_identities}"##,
        n_p = policies.len(),
        n_f = flows.len(),
        n_w = workloads.len(),
        n_s = services.len(),
        p_tbl = table(&["name", "action", "principal_glob"], &p_rows),
        f_tbl = table(&["src", "dst", "verdict", "bytes"], &f_rows),
        w_tbl = table(&["workload", "edges", "bytes out", "bytes dropped"], &w_rows),
        s_tbl = table(&["service", "edges", "bytes in", "bytes dropped", "health"], &s_rows),
        topology = topology_html,
        traffic = traffic_html,
        validations = validations_html,
        net_flows = net_flows_html,
        net_policies = net_policies_html,
        net_nodes = net_nodes_html,
        net_identities = net_identities_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/mesh",
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

    // ── 2026-05-14 consolidation ────────────────────────────────

    #[test]
    fn render_consolidated_includes_every_kiali_anchor() {
        // After /admin/kiali → /admin/mesh#kiali-topology 308 redirect,
        // every legacy kiali anchor must resolve on the mesh page.
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Topology.tsx",
            "KialiAnchorsOnMesh",
            "acme"
        );
        let html = render(
            &AdminState::seeded(),
            &ctx(&[
                Permission::MeshRead,
                Permission::KialiRead,
            ]),
        )
        .unwrap();
        for anchor in [
            "id=\"kiali-topology\"",
            "id=\"kiali-workloads\"",
            "id=\"kiali-services\"",
            "id=\"kiali-traffic\"",
            "id=\"kiali-validations\"",
        ] {
            assert!(html.contains(anchor), "missing kiali anchor: {anchor}");
        }
    }

    #[test]
    fn render_consolidated_includes_every_net_anchor() {
        // After /admin/net → /admin/mesh#net-flows 308 redirect,
        // every legacy net anchor must resolve on the mesh page.
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Network/NetworkPoliciesTab.tsx",
            "NetAnchorsOnMesh",
            "acme"
        );
        let html = render(
            &AdminState::seeded(),
            &ctx(&[
                Permission::MeshRead,
                Permission::NetRead,
            ]),
        )
        .unwrap();
        for anchor in [
            "id=\"net-flows\"",
            "id=\"net-policies\"",
            "id=\"net-services\"",
            "id=\"net-nodes\"",
            "id=\"net-identities\"",
        ] {
            assert!(html.contains(anchor), "missing net anchor: {anchor}");
        }
    }

    #[test]
    fn render_consolidated_mentions_redirect_note() {
        // Operators visiting /admin/mesh deserve to know that
        // /admin/kiali and /admin/net 308 here. The intro section
        // calls this out.
        let html = render(
            &AdminState::seeded(),
            &ctx(&[Permission::MeshRead]),
        )
        .unwrap();
        assert!(html.contains("/admin/kiali"));
        assert!(html.contains("/admin/net"));
        assert!(html.contains("308"));
    }

    #[test]
    fn render_consolidated_includes_cilium_upstream_reference() {
        // Net features (Cilium Hubble) keep their upstream attribution.
        let html = render(
            &AdminState::seeded(),
            &ctx(&[Permission::MeshRead, Permission::NetRead]),
        )
        .unwrap();
        assert!(html.contains("cilium"));
        assert!(html.contains("istio.io"));
    }

    #[test]
    fn render_consolidated_works_with_mesh_only_permission() {
        // MeshRead-only callers still get a working page — the
        // consolidated kiali/net sections silently drop their
        // permission-gated bodies. The mesh-native sections remain;
        // the kiali/net data sections (#kiali-topology body,
        // #net-flows body) are absent.
        let html = render(
            &AdminState::seeded(),
            &ctx(&[Permission::MeshRead]),
        )
        .unwrap();
        assert!(html.contains("Workloads"));
        assert!(html.contains("AuthZ"));
        // No KialiRead → topology section content absent. The
        // anchor link in the nav remains (`href="#kiali-topology"`)
        // but the body never renders.
        assert!(!html.contains("id=\"kiali-topology\""));
        // No NetRead → flows section content absent.
        assert!(!html.contains("id=\"net-flows\""));
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
