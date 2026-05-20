// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/karpenter` — Karpenter NodePool + NodeClaim parity. The
//! upstream surface is CRD-only; the page mirrors what `karpenter`'s
//! Grafana dashboard would show — pool utilisation, capacity, and a
//! "near-cap" badge for pools approaching their max.
//!
//! Upstream: <https://karpenter.sh/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, NodePool, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KarpenterViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<NodePool>, KarpenterViewError> {
    ctx.authorise(Permission::KarpenterRead)?;
    let mut rows: Vec<NodePool> = scope(&state.node_pools.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

/// Pool utilisation as `active / max`, in `[0.0, 1.0]`. Returns
/// `1.0` if `max == 0` so the dashboard surfaces such pools as "at
/// capacity" rather than dividing by zero.
pub fn utilisation(pool: &NodePool) -> f64 {
    if pool.max_nodes == 0 {
        1.0
    } else {
        (pool.active_nodes as f64 / pool.max_nodes as f64).clamp(0.0, 1.0)
    }
}

/// Threshold above which a pool earns the "near-cap" badge in the
/// dashboard. Matches Karpenter's recommended scale-out trigger.
pub const NEAR_CAP_THRESHOLD: f64 = 0.8;

/// Filter pools to those at or above [`NEAR_CAP_THRESHOLD`] — the
/// dashboard's "needs attention" list.
pub fn near_cap(pools: &[NodePool]) -> Vec<&NodePool> {
    pools
        .iter()
        .filter(|p| utilisation(p) >= NEAR_CAP_THRESHOLD)
        .collect()
}

/// Aggregate metrics for the header card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PoolSummary {
    pub pools: u32,
    pub active_nodes: u32,
    pub max_nodes: u32,
    pub near_cap_count: u32,
}

pub fn pool_summary(pools: &[NodePool]) -> PoolSummary {
    PoolSummary {
        pools: pools.len() as u32,
        active_nodes: pools.iter().map(|p| p.active_nodes).sum(),
        max_nodes: pools.iter().map(|p| p.max_nodes).sum(),
        near_cap_count: near_cap(pools).len() as u32,
    }
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KarpenterViewError> {
    let rows = list_records(state, ctx)?;
    let summary = pool_summary(&rows);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let util_pct = (utilisation(r) * 100.0).round() as u32;
            let badge = if util_pct >= 80 { " ⚠" } else { "" };
            vec![
                escape(&r.name),
                escape(&r.instance_class),
                r.max_nodes.to_string(),
                r.active_nodes.to_string(),
                format!("{util_pct}%{badge}"),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Karpenter NodePool capacity (cave-karpenter).
    Upstream: <a class="text-blue-700 underline" href="https://karpenter.sh/">karpenter.sh</a>.
  </p>
  <div class="mb-4 grid grid-cols-4 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">POOLS</div><div class="text-2xl font-bold">{pools}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">ACTIVE</div><div class="text-2xl font-bold">{active}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">CAPACITY</div><div class="text-2xl font-bold">{max}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">NEAR-CAP</div><div class="text-2xl font-bold text-orange-700">{near_cap}</div></div>
  </div>
  <h2 class="text-lg font-semibold mb-2">NodePools ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        pools = summary.pools,
        active = summary.active_nodes,
        max = summary.max_nodes,
        near_cap = summary.near_cap_count,
        tbl = table(&["name", "class", "max", "active", "util"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/karpenter",
        &format!("karpenter · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/karpenter/src/components/NodePoolsList.tsx",
    "NodePoolsList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/karpenter/src/components/NodePoolsList.tsx",
            "NodePoolsList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::KarpenterRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_owner_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/karpenter/src/components/NodePoolsList.tsx",
            "RenderOwner",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KarpenterRead])).unwrap();
        assert!(html.contains("default"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/karpenter/src/components/NodePoolsList.tsx",
            "RenderEvil",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KarpenterRead])).unwrap();
        assert!(!html.contains("evil-pool"));
    }

    #[test]
    fn utilisation_caps_at_one_and_handles_zero_max() {
        use cave_kernel::ns::TenantId;
        let t = TenantId::new("t").unwrap();
        let full = NodePool {
            tenant: t.clone(),
            name: "f".into(),
            instance_class: "m5.large".into(),
            max_nodes: 4,
            active_nodes: 4,
        };
        assert!((utilisation(&full) - 1.0).abs() < 1e-9);
        let over = NodePool {
            tenant: t.clone(),
            name: "o".into(),
            instance_class: "m5.large".into(),
            max_nodes: 4,
            active_nodes: 10,
        };
        assert!((utilisation(&over) - 1.0).abs() < 1e-9);
        let zero = NodePool {
            tenant: t.clone(),
            name: "z".into(),
            instance_class: "m5.large".into(),
            max_nodes: 0,
            active_nodes: 0,
        };
        assert!((utilisation(&zero) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn near_cap_filters_above_threshold() {
        use cave_kernel::ns::TenantId;
        let t = TenantId::new("t").unwrap();
        let pools = vec![
            NodePool {
                tenant: t.clone(),
                name: "a".into(),
                instance_class: "m5.large".into(),
                max_nodes: 10,
                active_nodes: 9,
            },
            NodePool {
                tenant: t.clone(),
                name: "b".into(),
                instance_class: "m5.large".into(),
                max_nodes: 10,
                active_nodes: 3,
            },
        ];
        let hot = near_cap(&pools);
        assert_eq!(hot.len(), 1);
        assert_eq!(hot[0].name, "a");
    }

    #[test]
    fn pool_summary_aggregates_totals() {
        let pools =
            list_records(&AdminState::seeded(), &ctx(&[Permission::KarpenterRead])).unwrap();
        let s = pool_summary(&pools);
        assert_eq!(s.pools, pools.len() as u32);
        let expected: u32 = pools.iter().map(|p| p.active_nodes).sum();
        assert_eq!(s.active_nodes, expected);
    }

    #[test]
    fn render_includes_capacity_cards() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KarpenterRead])).unwrap();
        assert!(html.contains("POOLS"));
        assert!(html.contains("CAPACITY"));
        assert!(html.contains("NEAR-CAP"));
        assert!(html.contains("karpenter.sh"));
    }
}
