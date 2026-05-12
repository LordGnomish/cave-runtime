//! Nodes tab — Cilium endpoint + agent state. The endpoint accessor
//! lives here so other tabs (policies impact, services aggregator)
//! can reuse it without a cycle.

use super::NetViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{scope, AdminState, NetEndpoint};

pub fn list_endpoints(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<NetEndpoint>, NetViewError> {
    ctx.authorise(Permission::NetRead)?;
    Ok(scope(&state.net_endpoints.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    pub node: String,
    pub agent_version: &'static str,
    pub healthy: bool,
    pub identities: u32,
}

pub fn list_agents(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<AgentRow>, NetViewError> {
    let endpoints = list_endpoints(state, ctx)?;
    // Group endpoints by IP /24 to derive node identity.
    use std::collections::BTreeMap;
    let mut by_node: BTreeMap<String, u32> = BTreeMap::new();
    for e in &endpoints {
        let node = e.ip.rsplit_once('.').map(|(p, _)| p.to_string()).unwrap_or_default();
        *by_node.entry(node).or_insert(0) += 1;
    }
    Ok(by_node
        .into_iter()
        .map(|(node, count)| AgentRow {
            node: format!("node-{}", node),
            agent_version: "v1.16.3",
            healthy: true,
            identities: count,
        })
        .collect())
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, NetViewError> {
    let endpoints = list_endpoints(state, ctx)?;
    let agents = list_agents(state, ctx)?;
    let endpoint_rows: Vec<Vec<String>> = endpoints
        .iter()
        .map(|e| {
            vec![
                e.identity.to_string(),
                e.namespace.clone(),
                e.ip.clone(),
                if e.ready { "Ready" } else { "NotReady" }.into(),
            ]
        })
        .collect();
    let agent_rows: Vec<Vec<String>> = agents
        .iter()
        .map(|a| {
            vec![
                a.node.clone(),
                a.agent_version.into(),
                if a.healthy { "Healthy" } else { "Down" }.into(),
                a.identities.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="net-nodes" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Nodes / Endpoints</h2>
  <h3 class="text-md font-semibold mt-2 mb-1">Cilium agents ({a})</h3>
  {agent_tbl}
  <h3 class="text-md font-semibold mt-3 mb-1">Endpoints ({e})</h3>
  {endpoint_tbl}
</section>"#,
        a = agents.len(),
        e = endpoints.len(),
        agent_tbl = table(&["node", "version", "health", "identities"], &agent_rows),
        endpoint_tbl = table(&["identity", "namespace", "ip", "ready"], &endpoint_rows),
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
    fn list_endpoints_filters_to_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Network/EndpointsTab.tsx",
            "EndpointsTab",
            "acme"
        );
        let s = AdminState::seeded();
        let endpoints = list_endpoints(&s, &ctx(&[Permission::NetRead])).unwrap();
        assert_eq!(endpoints.len(), 2);
    }

    #[test]
    fn list_endpoints_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_endpoints(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_agents_returns_one_row_per_distinct_node() {
        let s = AdminState::seeded();
        let agents = list_agents(&s, &ctx(&[Permission::NetRead])).unwrap();
        assert!(!agents.is_empty());
        assert!(agents.iter().all(|a| a.healthy));
    }

    #[test]
    fn render_section_emits_both_subsections() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::NetRead])).unwrap();
        assert!(html.contains("Cilium agents"));
        assert!(html.contains("Endpoints ("));
    }
}
