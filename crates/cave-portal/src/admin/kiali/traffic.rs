//! Traffic tab — VirtualService / DestinationRule / Gateway summary.
//!
//! Today the rules are synthesised one-per-service so the page has
//! the right shape; a live deployment resolves via Istio's
//! `/api/v1alpha3/virtualservices` / `destinationrules` / `gateways`.

use super::KialiViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficRule {
    pub kind: &'static str, // "VirtualService" | "DestinationRule" | "Gateway"
    pub name: String,
    pub host: String,
    pub subsets: u32,
}

pub fn list_traffic_rules(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<TrafficRule>, KialiViewError> {
    let services = super::services::list_services(state, ctx)?;
    let mut out = Vec::new();
    for s in &services {
        out.push(TrafficRule {
            kind: "VirtualService",
            name: format!("{}-vs", s.name),
            host: s.name.clone(),
            subsets: 1,
        });
        if s.destination_rule {
            out.push(TrafficRule {
                kind: "DestinationRule",
                name: format!("{}-dr", s.name),
                host: s.name.clone(),
                subsets: 2,
            });
        }
    }
    // Single mesh gateway.
    out.push(TrafficRule {
        kind: "Gateway",
        name: "mesh-gateway".into(),
        host: "*".into(),
        subsets: 0,
    });
    Ok(out)
}

pub fn count_by_kind(rules: &[TrafficRule], kind: &str) -> usize {
    rules.iter().filter(|r| r.kind == kind).count()
}

pub(crate) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, KialiViewError> {
    let rules = list_traffic_rules(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rules
        .iter()
        .map(|r| {
            vec![
                r.kind.into(),
                r.name.clone(),
                r.host.clone(),
                r.subsets.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kiali-traffic" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Traffic management ({n} rules)</h2>
  <div class="text-xs text-gray-500 mb-2">
    {vs} VirtualService · {dr} DestinationRule · {gw} Gateway
  </div>
  {tbl}
</section>"#,
        n = rules.len(),
        vs = count_by_kind(&rules, "VirtualService"),
        dr = count_by_kind(&rules, "DestinationRule"),
        gw = count_by_kind(&rules, "Gateway"),
        tbl = table(&["kind", "name", "host", "subsets"], &table_rows),
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
    fn list_traffic_rules_includes_gateway_kind() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Traffic.tsx",
            "Traffic",
            "acme"
        );
        let s = AdminState::seeded();
        let rules = list_traffic_rules(&s, &ctx(&[Permission::KialiRead])).unwrap();
        assert!(rules.iter().any(|r| r.kind == "Gateway"));
    }

    #[test]
    fn list_traffic_rules_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_traffic_rules(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn count_by_kind_filters() {
        let s = AdminState::seeded();
        let rules = list_traffic_rules(&s, &ctx(&[Permission::KialiRead])).unwrap();
        let total = count_by_kind(&rules, "VirtualService")
            + count_by_kind(&rules, "DestinationRule")
            + count_by_kind(&rules, "Gateway");
        assert_eq!(total, rules.len());
    }

    #[test]
    fn render_section_emits_legend() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KialiRead])).unwrap();
        assert!(html.contains("VirtualService"));
        assert!(html.contains("DestinationRule"));
        assert!(html.contains("Gateway"));
    }
}
