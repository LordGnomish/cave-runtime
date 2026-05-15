//! Services tab — Services + Endpoints + Ingresses.
//!
//! Derives Service rows per node (one ClusterIP-style service per
//! Running pod's node) plus a synthesised Endpoints + Ingress count.

use super::K8sDashboardViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRow {
    pub name: String,
    pub namespace: String,
    pub cluster_ip: String,
    pub service_type: &'static str, // "ClusterIP" | "NodePort" | "LoadBalancer"
    pub endpoints: u32,
}

pub fn list_services(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ServiceRow>, K8sDashboardViewError> {
    let workloads = super::workloads::list_workloads(state, ctx)?;
    use std::collections::BTreeMap;
    let mut endpoints: BTreeMap<String, u32> = BTreeMap::new();
    for w in &workloads {
        if w.pod_name.is_empty() {
            continue;
        }
        let key = w.pod_name.split('-').next().unwrap_or(&w.pod_name).to_string();
        *endpoints.entry(key).or_insert(0) += 1;
    }
    Ok(endpoints
        .into_iter()
        .map(|(name, eps)| ServiceRow {
            cluster_ip: format!("10.96.{}.{}", (name.len() % 250) + 1, 100),
            service_type: if eps >= 3 {
                "LoadBalancer"
            } else if eps >= 2 {
                "NodePort"
            } else {
                "ClusterIP"
            },
            endpoints: eps,
            namespace: "default".into(),
            name,
        })
        .collect())
}

pub fn ingress_count(rows: &[ServiceRow]) -> usize {
    rows.iter().filter(|s| s.service_type == "LoadBalancer").count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, K8sDashboardViewError> {
    let rows = list_services(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.namespace.clone(),
                s.cluster_ip.clone(),
                s.service_type.into(),
                s.endpoints.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="k8s-dashboard-services" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Services ({n}, {lb} LoadBalancer / Ingress)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        lb = ingress_count(&rows),
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
    fn list_services_clusters_pods_by_name_prefix() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Services.tsx",
            "Services",
            "acme"
        );
        let s = AdminState::seeded();
        let services = list_services(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        assert!(services.iter().all(|s| s.endpoints >= 1));
    }

    #[test]
    fn list_services_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_services(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn service_type_threshold_is_endpoint_count() {
        let s = AdminState::seeded();
        let services = list_services(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        for s in &services {
            let expected = if s.endpoints >= 3 {
                "LoadBalancer"
            } else if s.endpoints >= 2 {
                "NodePort"
            } else {
                "ClusterIP"
            };
            assert_eq!(s.service_type, expected);
        }
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        for col in ["name", "namespace", "clusterIP", "type", "endpoints"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
