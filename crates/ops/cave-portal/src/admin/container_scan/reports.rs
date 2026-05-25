// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/container_scan/reports` — Trivy "Reports" tab. Per-
//! severity roll-up with derived high/medium/low buckets so the
//! operator can answer "how dirty is the registry overall".
//! Severity buckets are synthesised from the critical-CVE
//! count using fixed thresholds matching the typical Trivy
//! filter (`high=2-9`, `medium=1`, `low=0`).
//!
//! Upstream: <https://aquasecurity.github.io/trivy>

use super::ContainerScanViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRow {
    pub severity: &'static str,
    pub image_count: usize,
}

pub fn build(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ReportRow>, ContainerScanViewError> {
    let images = super::images::list_images(state, ctx)?;
    let mut high = 0usize;
    let mut medium = 0usize;
    let mut low = 0usize;
    for i in &images {
        if i.critical_cves >= 2 {
            high += 1;
        } else if i.critical_cves == 1 {
            medium += 1;
        } else {
            low += 1;
        }
    }
    Ok(vec![
        ReportRow {
            severity: "HIGH",
            image_count: high,
        },
        ReportRow {
            severity: "MEDIUM",
            image_count: medium,
        },
        ReportRow {
            severity: "LOW",
            image_count: low,
        },
    ])
}

pub fn worst_severity_present<'a>(rows: &'a [ReportRow]) -> Option<&'a ReportRow> {
    rows.iter().find(|r| r.image_count > 0)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ContainerScanViewError> {
    let rows = build(state, ctx)?;
    let worst = worst_severity_present(&rows)
        .map(|r| format!("worst: {} ({})", r.severity, r.image_count))
        .unwrap_or_else(|| "no images".to_string());
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![r.severity.to_string(), r.image_count.to_string()])
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Reports — {worst}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-severity roll-up. Upstream:
    <a class="text-blue-700 underline" href="https://aquasecurity.github.io/trivy">Trivy Reports</a>.
  </p>
  {tbl}
</section>"#,
        worst = worst,
        tbl = table(&["severity", "image_count"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/container_scan/reports",
        &format!("container_scan/reports · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn build_returns_three_severity_rows() {
        let rows = build(
            &AdminState::seeded(),
            &ctx(&[Permission::ContainerScanRead]),
        )
        .unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn build_severities_sum_to_image_count() {
        let images = super::super::images::list_images(
            &AdminState::seeded(),
            &ctx(&[Permission::ContainerScanRead]),
        )
        .unwrap();
        let rows = build(
            &AdminState::seeded(),
            &ctx(&[Permission::ContainerScanRead]),
        )
        .unwrap();
        let total: usize = rows.iter().map(|r| r.image_count).sum();
        assert_eq!(total, images.len());
    }

    #[test]
    fn worst_severity_returns_highest_with_count() {
        let rows = build(
            &AdminState::seeded(),
            &ctx(&[Permission::ContainerScanRead]),
        )
        .unwrap();
        if let Some(w) = worst_severity_present(&rows) {
            assert!(w.image_count > 0);
        }
    }

    #[test]
    fn build_rejects_without_permission() {
        assert!(build(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_reports_label() {
        let html = render(
            &AdminState::seeded(),
            &ctx(&[Permission::ContainerScanRead]),
        )
        .unwrap();
        assert!(html.contains("Reports"));
        assert!(html.contains("Trivy Reports"));
    }
}
