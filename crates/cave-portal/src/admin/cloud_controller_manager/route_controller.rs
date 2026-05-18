// SPDX-License-Identifier: AGPL-3.0-or-later
//! Route controller tab — per-node pod CIDR routes (k8s.io node-routes).

use super::CloudControllerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRow {
    pub node: String,
    pub pod_cidr: String,
    pub next_hop: String,
    pub state: &'static str, // "Active" | "Stale"
}

pub fn list_routes(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<RouteRow>, CloudControllerViewError> {
    let nodes = super::node_controller::list_nodes(state, ctx)?;
    Ok(nodes
        .into_iter()
        .enumerate()
        .map(|(idx, n)| RouteRow {
            pod_cidr: format!("10.244.{}.0/24", idx),
            next_hop: format!("10.0.0.{}", idx + 10),
            state: if n.initialized { "Active" } else { "Stale" },
            node: n.node,
        })
        .collect())
}

pub fn active_count(rows: &[RouteRow]) -> usize {
    rows.iter().filter(|r| r.state == "Active").count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CloudControllerViewError> {
    let rows = list_routes(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![r.node.clone(), r.pod_cidr.clone(), r.next_hop.clone(), r.state.into()])
        .collect();
    Ok(format!(
        r#"<section id="ccm-routes" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">RouteController ({n}, {act} Active)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        act = active_count(&rows),
        tbl = table(&["node", "podCIDR", "nextHop", "state"], &table_rows),
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
    fn list_routes_one_per_node() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/CloudResources/Routes.tsx",
            "Routes",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_routes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        let nodes = super::super::node_controller::list_nodes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        assert_eq!(rows.len(), nodes.len());
    }

    #[test]
    fn list_routes_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_routes(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn active_routes_only_count_initialized_nodes() {
        let s = AdminState::seeded();
        let rows = list_routes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        let nodes = super::super::node_controller::list_nodes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        let active = active_count(&rows);
        let expected = super::super::node_controller::count_initialized(&nodes);
        assert_eq!(active, expected);
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        for col in ["node", "podCIDR", "nextHop", "state"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
