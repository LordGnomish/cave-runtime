// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Node controller tab — node registration + InitializeProvider state.
//! Derived from scheduler_nodes; one row per node with provider + zone.

use super::CloudControllerViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{AdminState, scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeControllerRow {
    pub node: String,
    pub provider: &'static str, // "aws" | "gcp" | "hetzner" | "bare-metal"
    pub provider_id: String,
    pub zone: &'static str,
    pub initialized: bool,
}

pub fn list_nodes(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<NodeControllerRow>, CloudControllerViewError> {
    ctx.authorise(Permission::CloudControllerRead)?;
    let guard = state.scheduler_nodes.read().unwrap();
    let nodes = scope(&guard, &ctx.tenant, |r| &r.tenant);
    Ok(nodes
        .into_iter()
        .enumerate()
        .map(|(idx, n)| {
            let provider = match idx % 4 {
                0 => "aws",
                1 => "gcp",
                2 => "hetzner",
                _ => "bare-metal",
            };
            let zone = match provider {
                "aws" => "eu-west-1a",
                "gcp" => "europe-west1-b",
                "hetzner" => "nbg1-dc3",
                _ => "rack-01",
            };
            NodeControllerRow {
                provider,
                provider_id: format!("{}://i-{}", provider, &n.name),
                zone,
                initialized: n.ready,
                node: n.name.clone(),
            }
        })
        .collect())
}

pub fn count_initialized(rows: &[NodeControllerRow]) -> usize {
    rows.iter().filter(|r| r.initialized).count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CloudControllerViewError> {
    let rows = list_nodes(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.node.clone(),
                r.provider.into(),
                r.provider_id.clone(),
                r.zone.into(),
                if r.initialized {
                    "Initialized"
                } else {
                    "Pending"
                }
                .into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="ccm-nodes" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">NodeController ({n}, {init} Initialized)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        init = count_initialized(&rows),
        tbl = table(
            &["node", "provider", "providerID", "zone", "state"],
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
    fn list_nodes_uses_scheduler_node_set() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/CloudResources/Nodes.tsx",
            "Nodes",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_nodes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        assert!(!rows.is_empty());
    }

    #[test]
    fn list_nodes_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_nodes(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn provider_set_to_known_string() {
        let s = AdminState::seeded();
        let rows = list_nodes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        for r in &rows {
            assert!(["aws", "gcp", "hetzner", "bare-metal"].contains(&r.provider));
        }
    }

    #[test]
    fn render_section_emits_provider_column() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        for col in ["node", "provider", "providerID", "zone", "state"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
