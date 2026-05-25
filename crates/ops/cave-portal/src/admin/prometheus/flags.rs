// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Flags tab — runtime flags Prometheus was launched with.
//!
//! Mirrors `/flags`. Today we expose a static set keyed to cave-metrics
//! defaults; a real port reads `/api/v1/status/flags`.

use super::PrometheusViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagRow {
    pub name: &'static str,
    pub value: String,
    pub description: &'static str,
}

pub fn list_flags(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<FlagRow>, PrometheusViewError> {
    ctx.authorise(Permission::PrometheusRead)?;
    // Pull retention out of the seeded targets so the value reflects
    // the live cave-metrics state.
    let targets = super::targets::list_targets(state, ctx)?;
    let max_retention = targets.iter().map(|t| t.retention_days).max().unwrap_or(15);
    Ok(vec![
        FlagRow {
            name: "--storage.tsdb.retention.time",
            value: format!("{}d", max_retention),
            description: "How long to retain samples in storage.",
        },
        FlagRow {
            name: "--storage.tsdb.path",
            value: "/var/lib/prometheus".into(),
            description: "Base path for metrics storage.",
        },
        FlagRow {
            name: "--web.enable-lifecycle",
            value: "true".into(),
            description: "Enable shutdown and reload via HTTP request.",
        },
        FlagRow {
            name: "--web.enable-admin-api",
            value: "true".into(),
            description: "Enable admin control actions.",
        },
        FlagRow {
            name: "--query.max-concurrency",
            value: "20".into(),
            description: "Maximum number of queries executed concurrently.",
        },
        FlagRow {
            name: "--query.timeout",
            value: "2m".into(),
            description: "Maximum time a query may take before being aborted.",
        },
        FlagRow {
            name: "--config.file",
            value: "/etc/prometheus/prometheus.yml".into(),
            description: "Prometheus configuration file path.",
        },
    ])
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, PrometheusViewError> {
    let rows = list_flags(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|f| vec![f.name.into(), f.value.clone(), f.description.into()])
        .collect();
    Ok(format!(
        r#"<section id="prometheus-flags" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Flags ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["flag", "value", "description"], &table_rows),
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
    fn list_flags_includes_well_known_prometheus_flags() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/FlagsList.tsx",
            "Flags",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_flags(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        let names: Vec<_> = rows.iter().map(|r| r.name).collect();
        for f in [
            "--storage.tsdb.retention.time",
            "--web.enable-lifecycle",
            "--query.timeout",
            "--config.file",
        ] {
            assert!(names.contains(&f), "missing flag {}", f);
        }
    }

    #[test]
    fn list_flags_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_flags(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn retention_flag_uses_target_max() {
        let s = AdminState::seeded();
        let rows = list_flags(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        let retention = rows
            .iter()
            .find(|r| r.name == "--storage.tsdb.retention.time")
            .unwrap();
        assert!(retention.value.ends_with('d'));
    }

    #[test]
    fn render_section_emits_table_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        for col in ["flag", "value", "description"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
