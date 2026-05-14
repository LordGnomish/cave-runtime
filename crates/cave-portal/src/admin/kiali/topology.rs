//! Topology tab — Istio Kiali service-graph parity.

use super::KialiViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyEdge {
    pub source: String,
    pub destination: String,
    pub verdict: &'static str,
    pub bytes: u64,
}

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
    Ok(acc
        .into_iter()
        .map(|((source, destination, verdict), bytes)| TopologyEdge {
            source,
            destination,
            verdict,
            bytes,
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub name: String,
    pub incoming: u32,
    pub outgoing: u32,
    pub bytes_total: u64,
}

pub fn list_nodes(edges: &[TopologyEdge]) -> Vec<GraphNode> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, GraphNode> = BTreeMap::new();
    for e in edges {
        let s = acc.entry(e.source.clone()).or_insert(GraphNode {
            name: e.source.clone(),
            incoming: 0,
            outgoing: 0,
            bytes_total: 0,
        });
        s.outgoing += 1;
        s.bytes_total += e.bytes;
        let d = acc.entry(e.destination.clone()).or_insert(GraphNode {
            name: e.destination.clone(),
            incoming: 0,
            outgoing: 0,
            bytes_total: 0,
        });
        d.incoming += 1;
        d.bytes_total += e.bytes;
    }
    acc.into_values().collect()
}

pub fn edge_health(edge: &TopologyEdge) -> &'static str {
    if edge.verdict == "Forwarded" {
        "Healthy"
    } else {
        "Failing"
    }
}

pub(crate) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, KialiViewError> {
    let edges = list_edges(state, ctx)?;
    let nodes = list_nodes(&edges);
    let edge_rows: Vec<Vec<String>> = edges
        .iter()
        .map(|r| {
            vec![
                escape(&r.source),
                escape(&r.destination),
                r.verdict.into(),
                r.bytes.to_string(),
                edge_health(r).into(),
            ]
        })
        .collect();
    let node_rows: Vec<Vec<String>> = nodes
        .iter()
        .map(|n| {
            vec![
                escape(&n.name),
                n.incoming.to_string(),
                n.outgoing.to_string(),
                n.bytes_total.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kiali-topology" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">Topology — Edges ({e})</h2>
  {edge_tbl}
  <h3 class="text-md font-semibold mt-3 mb-1">Nodes ({n})</h3>
  {node_tbl}
</section>"#,
        e = edges.len(),
        n = nodes.len(),
        edge_tbl = table(
            &["source", "destination", "verdict", "bytes", "health"],
            &edge_rows
        ),
        node_tbl = table(&["name", "in", "out", "bytes total"], &node_rows),
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
    fn list_edges_filters_to_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Topology.tsx",
            "Edges",
            "acme"
        );
        let s = AdminState::seeded();
        let edges = list_edges(&s, &ctx(&[Permission::KialiRead])).unwrap();
        assert!(!edges.is_empty());
    }

    #[test]
    fn list_edges_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_edges(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_nodes_derives_in_out_counts() {
        let s = AdminState::seeded();
        let edges = list_edges(&s, &ctx(&[Permission::KialiRead])).unwrap();
        let nodes = list_nodes(&edges);
        let total_io: u32 = nodes.iter().map(|n| n.incoming + n.outgoing).sum();
        assert_eq!(total_io as usize, edges.len() * 2);
    }

    #[test]
    fn edge_health_maps_forwarded_to_healthy() {
        let e = TopologyEdge {
            source: "a".into(),
            destination: "b".into(),
            verdict: "Forwarded",
            bytes: 1,
        };
        assert_eq!(edge_health(&e), "Healthy");
        let e2 = TopologyEdge {
            source: "a".into(),
            destination: "b".into(),
            verdict: "Dropped",
            bytes: 1,
        };
        assert_eq!(edge_health(&e2), "Failing");
    }

    #[test]
    fn render_section_emits_edges_and_nodes() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KialiRead])).unwrap();
        assert!(html.contains("Edges"));
        assert!(html.contains("Nodes"));
    }
}
