// SPDX-License-Identifier: AGPL-3.0-or-later
//! Status tab — service discovery + config + alertmanagers.
//!
//! Mirrors Prometheus `/status` page (build info, runtime info,
//! command-line flags, configuration, rules, targets, service
//! discovery, alertmanagers). The TSDB / Targets / Flags / Rules tabs
//! each cover one corner; this tab covers the cross-cutting bits.

use super::PrometheusViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDiscoveryRow {
    pub name: &'static str,
    pub kind: &'static str, // "kubernetes_sd" | "static_config" | "consul_sd" | ...
    pub target_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertmanagerRow {
    pub url: &'static str,
    pub api_version: &'static str,
    pub state: &'static str, // "active" | "dropped"
}

pub fn list_service_discovery(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ServiceDiscoveryRow>, PrometheusViewError> {
    let targets = super::targets::list_targets(state, ctx)?;
    let mut counts: std::collections::BTreeMap<&'static str, u32> = std::collections::BTreeMap::new();
    for t in &targets {
        let kind = if t.scraper.contains("kube") {
            "kubernetes_sd"
        } else if t.scraper.starts_with("static") {
            "static_config"
        } else {
            "consul_sd"
        };
        *counts.entry(kind).or_insert(0) += 1;
    }
    Ok(counts
        .into_iter()
        .map(|(kind, n)| ServiceDiscoveryRow {
            name: kind,
            kind,
            target_count: n,
        })
        .collect())
}

pub fn list_alertmanagers(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<AlertmanagerRow>, PrometheusViewError> {
    ctx.authorise(Permission::PrometheusRead)?;
    Ok(vec![
        AlertmanagerRow {
            url: "http://alertmanager-0.observability.svc:9093",
            api_version: "v2",
            state: "active",
        },
        AlertmanagerRow {
            url: "http://alertmanager-1.observability.svc:9093",
            api_version: "v2",
            state: "active",
        },
    ])
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, PrometheusViewError> {
    let sd = list_service_discovery(state, ctx)?;
    let am = list_alertmanagers(state, ctx)?;
    let sd_rows: Vec<Vec<String>> = sd
        .iter()
        .map(|r| vec![r.kind.into(), r.target_count.to_string()])
        .collect();
    let am_rows: Vec<Vec<String>> = am
        .iter()
        .map(|r| {
            vec![
                r.url.into(),
                r.api_version.into(),
                r.state.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="prometheus-status" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Status</h2>
  <h3 class="text-md font-semibold mt-3 mb-1">Service discovery</h3>
  {sd_tbl}
  <h3 class="text-md font-semibold mt-3 mb-1">Alertmanagers</h3>
  {am_tbl}
</section>"#,
        sd_tbl = table(&["kind", "targets"], &sd_rows),
        am_tbl = table(&["url", "api version", "state"], &am_rows),
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
    fn list_service_discovery_bins_targets_by_kind() {
        use super::super::targets;
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/StatusPanel.tsx",
            "ServiceDiscovery",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_service_discovery(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        let total: u32 = rows.iter().map(|r| r.target_count).sum();
        let targets =
            targets::list_targets(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert_eq!(total, targets.len() as u32);
    }

    #[test]
    fn list_alertmanagers_returns_active_endpoints() {
        let s = AdminState::seeded();
        let rows = list_alertmanagers(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.state == "active"));
    }

    #[test]
    fn list_alertmanagers_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_alertmanagers(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_emits_both_subsections() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert!(html.contains("Service discovery"));
        assert!(html.contains("Alertmanagers"));
    }
}
