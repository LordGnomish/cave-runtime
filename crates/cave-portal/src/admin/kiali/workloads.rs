//! Workloads tab — service-mesh-attached workload list.
//!
//! Kiali shows: workload name, namespace, type (Deployment, StatefulSet),
//! sidecar status, health (Healthy / Degraded / Failure), traffic
//! (requests/min in, requests/min out). We derive from mesh flows.

use super::KialiViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadRow {
    pub name: String,
    pub namespace: String,
    pub sidecar: &'static str, // "✓" | "✗"
    pub health: &'static str,
    pub in_rpm: u32,
    pub out_rpm: u32,
}

pub fn list_workloads(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<WorkloadRow>, KialiViewError> {
    let edges = super::topology::list_edges(state, ctx)?;
    let nodes = super::topology::list_nodes(&edges);
    Ok(nodes
        .into_iter()
        .map(|n| WorkloadRow {
            health: classify_health(&n),
            sidecar: if n.bytes_total > 0 { "✓" } else { "✗" },
            // Synthetic RPM from byte volume (1 RPM ≈ 1 KiB/s).
            in_rpm: (n.bytes_total as u32 / 1024).min(99_999),
            out_rpm: (n.outgoing * 60).min(99_999),
            namespace: "default".into(),
            name: n.name,
        })
        .collect())
}

fn classify_health(n: &super::topology::GraphNode) -> &'static str {
    if n.bytes_total == 0 {
        "Idle"
    } else if n.bytes_total < 1_000 {
        "Degraded"
    } else {
        "Healthy"
    }
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, KialiViewError> {
    let rows = list_workloads(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|w| {
            vec![
                w.name.clone(),
                w.namespace.clone(),
                w.sidecar.into(),
                w.health.into(),
                w.in_rpm.to_string(),
                w.out_rpm.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kiali-workloads" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Workloads ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["name", "namespace", "sidecar", "health", "in rpm", "out rpm"],
            &table_rows
        ),
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
    fn list_workloads_emits_one_row_per_graph_node() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/WorkloadList.tsx",
            "Workloads",
            "acme"
        );
        let s = AdminState::seeded();
        let workloads = list_workloads(&s, &ctx(&[Permission::KialiRead])).unwrap();
        let edges = super::super::topology::list_edges(&s, &ctx(&[Permission::KialiRead])).unwrap();
        let nodes = super::super::topology::list_nodes(&edges);
        assert_eq!(workloads.len(), nodes.len());
    }

    #[test]
    fn list_workloads_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_workloads(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn health_classification_uses_traffic_thresholds() {
        use super::super::topology::GraphNode;
        assert_eq!(
            classify_health(&GraphNode {
                name: "x".into(),
                incoming: 0,
                outgoing: 0,
                bytes_total: 0,
            }),
            "Idle"
        );
        assert_eq!(
            classify_health(&GraphNode {
                name: "x".into(),
                incoming: 0,
                outgoing: 0,
                bytes_total: 500,
            }),
            "Degraded"
        );
        assert_eq!(
            classify_health(&GraphNode {
                name: "x".into(),
                incoming: 0,
                outgoing: 0,
                bytes_total: 100_000,
            }),
            "Healthy"
        );
    }

    #[test]
    fn render_section_emits_kiali_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KialiRead])).unwrap();
        for col in ["name", "namespace", "sidecar", "health", "in rpm", "out rpm"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
