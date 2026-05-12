//! Identities tab — Cilium security identity catalog.

use super::NetViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityRow {
    pub id: u64,
    pub namespace: String,
    pub labels: Vec<String>,
    pub endpoint_count: u32,
}

pub fn list_identities(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<IdentityRow>, NetViewError> {
    let endpoints = super::nodes::list_endpoints(state, ctx)?;
    use std::collections::BTreeMap;
    let mut by_id: BTreeMap<u64, (String, u32)> = BTreeMap::new();
    for e in &endpoints {
        let slot = by_id.entry(e.identity).or_insert((e.namespace.clone(), 0));
        slot.1 += 1;
    }
    Ok(by_id
        .into_iter()
        .map(|(id, (ns, count))| IdentityRow {
            id,
            labels: vec![
                format!("k8s:io.kubernetes.pod.namespace={}", ns),
                format!("identity={}", id),
            ],
            namespace: ns,
            endpoint_count: count,
        })
        .collect())
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, NetViewError> {
    let rows = list_identities(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|i| {
            vec![
                i.id.to_string(),
                i.namespace.clone(),
                i.labels.join(", "),
                i.endpoint_count.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="net-identities" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Security identities ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["id", "namespace", "labels", "endpoints"],
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
    fn list_identities_groups_endpoints_by_identity() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Network/Identities.tsx",
            "Identities",
            "acme"
        );
        let s = AdminState::seeded();
        let identities = list_identities(&s, &ctx(&[Permission::NetRead])).unwrap();
        let total_endpoints: u32 = identities.iter().map(|i| i.endpoint_count).sum();
        // Acme has 2 endpoints.
        assert_eq!(total_endpoints, 2);
    }

    #[test]
    fn list_identities_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_identities(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn identity_labels_include_namespace() {
        let s = AdminState::seeded();
        let identities = list_identities(&s, &ctx(&[Permission::NetRead])).unwrap();
        for i in &identities {
            assert!(i.labels.iter().any(|l| l.contains(&i.namespace)));
        }
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::NetRead])).unwrap();
        for col in ["id", "namespace", "labels", "endpoints"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
