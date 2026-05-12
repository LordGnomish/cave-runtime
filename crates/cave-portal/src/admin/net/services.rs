//! Services tab — Kubernetes services (ClusterIP / NodePort / LB).
//! Derived from the endpoint set so each tenant sees its own services.

use super::NetViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRow {
    pub name: String,
    pub namespace: String,
    pub cluster_ip: String,
    pub kind: &'static str, // "ClusterIP" | "NodePort" | "LoadBalancer"
    pub endpoint_count: u32,
}

pub fn list_services(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ServiceRow>, NetViewError> {
    let endpoints = super::nodes::list_endpoints(state, ctx)?;
    // Group endpoints by namespace; one service per namespace.
    use std::collections::BTreeMap;
    let mut by_ns: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for e in &endpoints {
        by_ns.entry(e.namespace.clone()).or_default().push(e);
    }
    let mut out = Vec::new();
    for (idx, (ns, members)) in by_ns.into_iter().enumerate() {
        out.push(ServiceRow {
            name: format!("{}-svc", ns),
            namespace: ns,
            cluster_ip: format!("10.96.0.{}", idx + 1),
            kind: if members.len() >= 3 {
                "LoadBalancer"
            } else if members.len() >= 2 {
                "NodePort"
            } else {
                "ClusterIP"
            },
            endpoint_count: members.len() as u32,
        });
    }
    ctx.authorise(Permission::NetRead)?;
    Ok(out)
}

pub fn count_by_kind(rows: &[ServiceRow], kind: &str) -> usize {
    rows.iter().filter(|r| r.kind == kind).count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, NetViewError> {
    let rows = list_services(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.namespace.clone(),
                s.cluster_ip.clone(),
                s.kind.into(),
                s.endpoint_count.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="net-services" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Services ({n}, {lb} LoadBalancer)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        lb = count_by_kind(&rows, "LoadBalancer"),
        tbl = table(
            &["name", "namespace", "clusterIP", "type", "endpoints"],
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
    fn list_services_groups_endpoints_by_namespace() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Network/ServicesTab.tsx",
            "ServicesTab",
            "acme"
        );
        let s = AdminState::seeded();
        let services = list_services(&s, &ctx(&[Permission::NetRead])).unwrap();
        assert!(services.iter().all(|s| s.endpoint_count >= 1));
    }

    #[test]
    fn list_services_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_services(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn service_type_threshold_matches_endpoint_count() {
        let s = AdminState::seeded();
        let services = list_services(&s, &ctx(&[Permission::NetRead])).unwrap();
        for s in &services {
            let expected = if s.endpoint_count >= 3 {
                "LoadBalancer"
            } else if s.endpoint_count >= 2 {
                "NodePort"
            } else {
                "ClusterIP"
            };
            assert_eq!(s.kind, expected);
        }
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::NetRead])).unwrap();
        for col in ["name", "namespace", "clusterIP", "type", "endpoints"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
