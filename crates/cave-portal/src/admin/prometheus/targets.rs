//! Targets tab — Prometheus `/targets` parity. One row per scraper +
//! series visible to the caller's tenant.

use super::PrometheusViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRow {
    pub series: String,
    pub scraper: String,
    pub sample_count: u64,
    pub retention_days: u32,
}

pub fn list_targets(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<TargetRow>, PrometheusViewError> {
    ctx.authorise(Permission::PrometheusRead)?;
    let series = state.metric_series.read().unwrap();
    let mut rows: Vec<TargetRow> = series
        .iter()
        .filter(|r| r.tenant.as_str() == ctx.tenant.as_str())
        .map(|r| TargetRow {
            series: r.name.clone(),
            scraper: r.scraper.clone(),
            sample_count: r.sample_count,
            retention_days: r.retention_days,
        })
        .collect();
    rows.sort_by(|a, b| a.scraper.cmp(&b.scraper).then(a.series.cmp(&b.series)));
    Ok(rows)
}

pub fn group_by_scraper(rows: &[TargetRow]) -> Vec<(String, Vec<TargetRow>)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, Vec<TargetRow>> = BTreeMap::new();
    for r in rows {
        acc.entry(r.scraper.clone()).or_default().push(r.clone());
    }
    acc.into_iter().collect()
}

pub fn total_samples(rows: &[TargetRow]) -> u64 {
    rows.iter().map(|r| r.sample_count).sum()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, PrometheusViewError> {
    let rows = list_targets(state, ctx)?;
    let groups = group_by_scraper(&rows);
    let group_html: String = groups
        .iter()
        .map(|(scraper, items)| {
            let item_rows: Vec<Vec<String>> = items
                .iter()
                .map(|r| {
                    vec![
                        r.series.clone(),
                        r.sample_count.to_string(),
                        format!("{}d", r.retention_days),
                    ]
                })
                .collect();
            format!(
                r#"<details open class="mb-2 p-2 bg-white rounded shadow-sm">
  <summary class="cursor-pointer font-semibold">🎯 {s} <small class="text-gray-500">({n})</small></summary>
  {tbl}
</details>"#,
                s = scraper,
                n = items.len(),
                tbl = table(&["series", "samples", "retention"], &item_rows),
            )
        })
        .collect();
    Ok(format!(
        r#"<section id="prometheus-targets" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">Targets ({n} series across {g} scrapers, {total} samples)</h2>
  {group_html}
</section>"#,
        n = rows.len(),
        g = groups.len(),
        total = total_samples(&rows),
        group_html = group_html,
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
    fn list_targets_filters_to_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/TargetList.tsx",
            "TargetList",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_targets(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert!(!rows.is_empty());
    }

    #[test]
    fn list_targets_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_targets(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_scraper_sorts_alphabetically() {
        let s = AdminState::seeded();
        let rows = list_targets(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        let groups = group_by_scraper(&rows);
        for w in groups.windows(2) {
            assert!(w[0].0 <= w[1].0);
        }
    }

    #[test]
    fn total_samples_sums_all_rows() {
        let s = AdminState::seeded();
        let rows = list_targets(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        let manual: u64 = rows.iter().map(|r| r.sample_count).sum();
        assert_eq!(total_samples(&rows), manual);
    }

    #[test]
    fn render_section_emits_grouped_tables() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert!(html.contains("🎯"));
        assert!(html.contains("Targets"));
    }
}
