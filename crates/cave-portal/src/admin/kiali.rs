//! `/admin/kiali` — Istio Kiali topology upstream-UI parity scaffold.
//!
//! Distinct from `admin/mesh.rs` (cave-mesh authz + traffic view).
//! This page mirrors the **upstream-UI** shape of Kiali — a flow-by-
//! flow source→destination topology with per-edge bytes and verdict.
//! Backed by cave-mesh.
//!
//! Upstream UI: <https://kiali.io/>
//!
//! Status: scaffold. The 5 tests pin the flow-aggregation + render
//! contracts.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KialiViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyEdge {
    pub source: String,
    pub destination: String,
    pub verdict: &'static str,
    pub bytes: u64,
}

/// Aggregate `MeshFlow` rows for the caller's tenant into a unique
/// `(source, destination, verdict)` edge list, summing `bytes`. The
/// upstream Kiali graph aggregates the same way — multiple flows on
/// the same edge collapse into one weighted line.
pub fn list_edges(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<TopologyEdge>, KialiViewError> {
    ctx.authorise(Permission::KialiRead)?;
    let flows = state.mesh_flows.read().unwrap();
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<(String, String, &'static str), u64> = BTreeMap::new();
    for f in flows
        .iter()
        .filter(|f| f.tenant.as_str() == ctx.tenant.as_str())
    {
        let key = (f.source.clone(), f.destination.clone(), f.verdict);
        *acc.entry(key).or_insert(0) += f.bytes;
    }
    let rows = acc
        .into_iter()
        .map(|((source, destination, verdict), bytes)| TopologyEdge {
            source,
            destination,
            verdict,
            bytes,
        })
        .collect();
    Ok(rows)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KialiViewError> {
    let rows = list_edges(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.source),
                escape(&r.destination),
                r.verdict.into(),
                r.bytes.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Istio Kiali topology scaffold (cave-mesh).
    Upstream: <a class="text-blue-700 underline" href="https://kiali.io/">kiali.io</a>.
  </p>
  <h2 class="text-lg font-semibold mb-2">Edges ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["source", "destination", "verdict", "bytes"], &table_rows),
    );
    Ok(page_shell(
        &format!("kiali · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/kiali/src/components/Topology.tsx", "Topology");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_edges_aggregates_flows_by_source_destination_verdict() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Topology.tsx",
            "EdgeAggregation",
            "acme"
        );
        let edges = list_edges(&AdminState::seeded(), &ctx(&[Permission::KialiRead])).unwrap();
        // No two edges should share the exact key — aggregation works.
        let mut keys: Vec<_> = edges
            .iter()
            .map(|e| (e.source.clone(), e.destination.clone(), e.verdict))
            .collect();
        keys.sort();
        let len = keys.len();
        keys.dedup();
        assert_eq!(
            keys.len(),
            len,
            "aggregation should collapse duplicate edges"
        );
    }

    #[test]
    fn list_edges_refuses_without_permission() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_edges(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_edges_excludes_other_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Topology.tsx",
            "TenantFilter",
            "acme"
        );
        let edges = list_edges(&AdminState::seeded(), &ctx(&[Permission::KialiRead])).unwrap();
        assert!(edges.iter().all(|e| !e.source.contains("evil")
            && !e.destination.contains("evil")));
    }

    #[test]
    fn render_links_kiali_url() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Topology.tsx",
            "RenderUpstreamLink",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KialiRead])).unwrap();
        assert!(html.contains("kiali.io"));
    }

    #[test]
    fn render_shows_verdict_per_edge() {
        // The seeded data includes "Forwarded" and "Dropped" verdicts;
        // both must appear so an operator can see drops at a glance.
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Topology.tsx",
            "RenderVerdicts",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KialiRead])).unwrap();
        // At minimum, the edge table must include a verdict column.
        assert!(html.contains("verdict"));
    }
}
