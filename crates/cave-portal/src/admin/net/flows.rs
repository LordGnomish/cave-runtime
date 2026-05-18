// SPDX-License-Identifier: AGPL-3.0-or-later
//! Flows tab — Hubble-style L3/L4/L7 flow viewer.
//!
//! Sourced from the seeded `mesh_flows` collection; verdict pills
//! mirror Hubble's "Forwarded" (green) / "Dropped" (red) classification.

use super::NetViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{scope, AdminState, MeshFlow};

pub fn list_flows(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<MeshFlow>, NetViewError> {
    ctx.authorise(Permission::NetRead)?;
    Ok(scope(&state.mesh_flows.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn forwarded_count(rows: &[MeshFlow]) -> usize {
    rows.iter().filter(|f| f.verdict == "Forwarded").count()
}

pub fn dropped_count(rows: &[MeshFlow]) -> usize {
    rows.iter().filter(|f| f.verdict == "Dropped").count()
}

pub fn total_bytes(rows: &[MeshFlow]) -> u64 {
    rows.iter().map(|f| f.bytes).sum()
}

pub(crate) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, NetViewError> {
    let flows = list_flows(state, ctx)?;
    let rows: Vec<Vec<String>> = flows
        .iter()
        .map(|f| {
            vec![
                escape(&f.source),
                escape(&f.destination),
                f.verdict.into(),
                f.bytes.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="net-flows" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">Flows ({n}, {fw} Forwarded / {dr} Dropped, {b} B)</h2>
  {tbl}
</section>"#,
        n = flows.len(),
        fw = forwarded_count(&flows),
        dr = dropped_count(&flows),
        b = total_bytes(&flows),
        tbl = table(&["source", "destination", "verdict", "bytes"], &rows),
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
    fn list_flows_filters_to_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Network/FlowsTab.tsx",
            "FlowsTab",
            "acme"
        );
        let s = AdminState::seeded();
        let flows = list_flows(&s, &ctx(&[Permission::NetRead])).unwrap();
        assert!(flows.iter().all(|f| f.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_flows_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_flows(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn verdict_counts_partition_total() {
        let s = AdminState::seeded();
        let flows = list_flows(&s, &ctx(&[Permission::NetRead])).unwrap();
        assert_eq!(forwarded_count(&flows) + dropped_count(&flows), flows.len());
    }

    #[test]
    fn render_section_emits_verdict_summary() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::NetRead])).unwrap();
        assert!(html.contains("Forwarded"));
        for col in ["source", "destination", "verdict", "bytes"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
