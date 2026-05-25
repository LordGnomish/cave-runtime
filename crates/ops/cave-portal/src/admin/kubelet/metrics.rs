// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Metrics tab — per-node CPU / memory / disk IO derived numbers.
//!
//! Mirrors the upstream Kubernetes Dashboard's per-node metrics chart
//! (header utilisation bars). Numbers are derived from the node's
//! capacity + the count of Running pods on it — a real cluster would
//! query metrics-server's `/apis/metrics.k8s.io/v1beta1/nodes`.

use super::KubeletViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq)]
pub struct NodeMetricsRow {
    pub node: String,
    pub cpu_used_milli: u64,
    pub cpu_capacity_milli: u64,
    pub mem_used_mib: u64,
    pub mem_capacity_mib: u64,
    pub disk_read_kib_s: u32,
    pub disk_write_kib_s: u32,
}

impl NodeMetricsRow {
    pub fn cpu_pct(&self) -> u32 {
        pct(self.cpu_used_milli, self.cpu_capacity_milli)
    }
    pub fn mem_pct(&self) -> u32 {
        pct(self.mem_used_mib, self.mem_capacity_mib)
    }
}

fn pct(used: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    ((used * 100 / total).min(100)) as u32
}

pub fn list_metrics(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<NodeMetricsRow>, KubeletViewError> {
    let nodes = super::nodes::list_nodes(state, ctx)?;
    Ok(nodes
        .into_iter()
        .map(|n| {
            // Synthetic but stable: each Running pod consumes 250m CPU + 512Mi.
            let running = n.pods_running as u64;
            let pending = n.pods_pending as u64;
            NodeMetricsRow {
                cpu_used_milli: (running * 250 + pending * 50).min(n.cpu_milli_total),
                cpu_capacity_milli: n.cpu_milli_total,
                mem_used_mib: (running * 512 + pending * 64).min(n.mem_mib_total),
                mem_capacity_mib: n.mem_mib_total,
                disk_read_kib_s: (running as u32 * 1024).min(8192),
                disk_write_kib_s: (running as u32 * 512).min(4096),
                node: n.name,
            }
        })
        .collect())
}

pub fn render_section(state: &AdminState, ctx: &RequestCtx) -> Result<String, KubeletViewError> {
    let rows = list_metrics(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|m| {
            vec![
                escape(&m.node),
                format!(
                    "{}m / {}m ({}%)",
                    m.cpu_used_milli,
                    m.cpu_capacity_milli,
                    m.cpu_pct()
                ),
                format!(
                    "{}Mi / {}Mi ({}%)",
                    m.mem_used_mib,
                    m.mem_capacity_mib,
                    m.mem_pct()
                ),
                format!("{} KiB/s", m.disk_read_kib_s),
                format!("{} KiB/s", m.disk_write_kib_s),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kubelet-metrics" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Per-node metrics ({n})</h2>
  <p class="text-xs text-gray-500 mb-2">Derived numbers (250m + 512Mi per Running pod). A live cluster would query metrics-server <code>/apis/metrics.k8s.io/v1beta1/nodes</code>.</p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["node", "cpu", "memory", "disk read", "disk write"],
            &table_rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn pct_caps_at_100_and_handles_zero_total() {
        assert_eq!(pct(0, 0), 0);
        assert_eq!(pct(50, 200), 25);
        assert_eq!(pct(300, 100), 100);
    }

    #[test]
    fn list_metrics_emits_one_row_per_node() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Metrics/MetricsList.tsx",
            "MetricsList",
            "acme"
        );
        let s = AdminState::seeded();
        let metrics = list_metrics(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        let nodes = super::super::nodes::list_nodes(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        assert_eq!(metrics.len(), nodes.len());
    }

    #[test]
    fn cpu_pct_never_exceeds_capacity() {
        let s = AdminState::seeded();
        let metrics = list_metrics(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        for m in &metrics {
            assert!(m.cpu_used_milli <= m.cpu_capacity_milli);
            assert!(m.cpu_pct() <= 100);
            assert!(m.mem_pct() <= 100);
        }
    }

    #[test]
    fn list_metrics_requires_kubelet_read() {
        let s = AdminState::seeded();
        assert!(list_metrics(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_emits_metrics_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        for col in ["node", "cpu", "memory", "disk read", "disk write"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
        assert!(html.contains("metrics.k8s.io"));
    }
}
