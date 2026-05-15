//! Services tab — Istio service entries with traffic split + endpoint count.

use super::KialiViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRow {
    pub name: String,
    pub namespace: String,
    pub endpoints: u32,
    pub virtual_service: bool,
    pub destination_rule: bool,
    pub traffic_rpm: u32,
}

pub fn list_services(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ServiceRow>, KialiViewError> {
    let edges = super::topology::list_edges(state, ctx)?;
    // Aggregate destination side of every edge into one service row.
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, (u32, u64)> = BTreeMap::new();
    for e in &edges {
        let entry = acc.entry(e.destination.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += e.bytes;
    }
    Ok(acc
        .into_iter()
        .map(|(name, (endpoints, bytes))| ServiceRow {
            name,
            namespace: "default".into(),
            endpoints,
            virtual_service: true,
            destination_rule: bytes > 5_000,
            traffic_rpm: (bytes as u32 / 1024).min(99_999),
        })
        .collect())
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, KialiViewError> {
    let rows = list_services(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.namespace.clone(),
                s.endpoints.to_string(),
                if s.virtual_service { "✓" } else { "✗" }.into(),
                if s.destination_rule { "✓" } else { "✗" }.into(),
                s.traffic_rpm.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kiali-services" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Services ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["name", "namespace", "endpoints", "VS", "DR", "rpm"],
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
    fn list_services_aggregates_destination_edges() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/ServiceList.tsx",
            "Services",
            "acme"
        );
        let s = AdminState::seeded();
        let services = list_services(&s, &ctx(&[Permission::KialiRead])).unwrap();
        // Every row's endpoints should be ≥1.
        assert!(services.iter().all(|s| s.endpoints >= 1));
    }

    #[test]
    fn list_services_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_services(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn destination_rule_set_for_busy_services_only() {
        let s = AdminState::seeded();
        let services = list_services(&s, &ctx(&[Permission::KialiRead])).unwrap();
        for s in &services {
            // DR flag tracks traffic_rpm threshold.
            assert_eq!(s.destination_rule, s.traffic_rpm * 1024 > 5_000);
        }
    }

    #[test]
    fn render_section_columns_match_kiali_legend() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KialiRead])).unwrap();
        for col in ["name", "namespace", "endpoints", "VS", "DR", "rpm"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
