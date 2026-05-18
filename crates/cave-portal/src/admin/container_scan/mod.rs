// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/container-scan` — Trivy vulnerability report parity. The
//! upstream UI is minimal (Trivy is CLI-first); this page mirrors
//! the `trivy image` JSON output as a per-image table with severity
//! filtering and aggregate counters.
//!
//! Tab layout — mirrors the Trivy CLI output sections:
//!
//! * [`vulnerabilities`] — per-scan CVE table (default landing)
//! * [`images`]          — deduplicated image roster
//! * [`policies`]        — admission gates derived from scans
//! * [`history`]         — chronological scan log
//! * [`reports`]         — per-severity roll-up
//!
//! Upstream: <https://trivy.dev/>

pub mod history;
pub mod images;
pub mod policies;
pub mod reports;
pub mod vulnerabilities;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, ContainerScanResult};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContainerScanViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ContainerScanResult>, ContainerScanViewError> {
    ctx.authorise(Permission::ContainerScanRead)?;
    let mut rows: Vec<ContainerScanResult> = scope(
        &state.container_scan_results.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    // Worst first — operators want vulnerable images at the top.
    rows.sort_by(|a, b| b.critical_cves.cmp(&a.critical_cves).then(a.image.cmp(&b.image)));
    Ok(rows)
}

/// Filter rows where `critical_cves >= threshold`. Mirrors Trivy's
/// `--severity CRITICAL` flag.
pub fn critical_only<'a>(
    rows: &'a [ContainerScanResult],
    threshold: u32,
) -> Vec<&'a ContainerScanResult> {
    rows.iter().filter(|r| r.critical_cves >= threshold).collect()
}

pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    image: &str,
) -> Result<Option<ContainerScanResult>, ContainerScanViewError> {
    let rows = list_records(state, ctx)?;
    Ok(rows.into_iter().find(|r| r.image == image))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanSummary {
    pub images: u32,
    pub total_criticals: u32,
    pub clean_images: u32,
    pub worst_image_cves: u32,
}

pub fn scan_summary(rows: &[ContainerScanResult]) -> ScanSummary {
    ScanSummary {
        images: rows.len() as u32,
        total_criticals: rows.iter().map(|r| r.critical_cves).sum(),
        clean_images: rows.iter().filter(|r| r.critical_cves == 0).count() as u32,
        worst_image_cves: rows.iter().map(|r| r.critical_cves).max().unwrap_or(0),
    }
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ContainerScanViewError> {
    let rows = list_records(state, ctx)?;
    let summary = scan_summary(&rows);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let badge = if r.critical_cves >= 1 { " ⚠" } else { "" };
            vec![
                escape(&r.image),
                escape(&r.digest),
                format!("{}{}", r.critical_cves, badge),
                r.scanned_at_unix.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Trivy vulnerability scan summary (cave-container-scan).
    Upstream: <a class="text-blue-700 underline" href="https://trivy.dev/">trivy.dev</a>.
  </p>
  <div class="mb-4 grid grid-cols-4 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">IMAGES</div><div class="text-2xl font-bold">{images}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">CRITICAL CVES</div><div class="text-2xl font-bold text-red-700">{total_critical}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">CLEAN</div><div class="text-2xl font-bold text-green-700">{clean}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">WORST</div><div class="text-2xl font-bold text-orange-700">{worst}</div></div>
  </div>
  <h2 class="text-lg font-semibold mb-2">Scan results ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        images = summary.images,
        total_critical = summary.total_criticals,
        clean = summary.clean_images,
        worst = summary.worst_image_cves,
        tbl = table(&["image", "digest", "criticals", "scanned_at"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/container-scan",
        &format!("container-scan · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/trivy/src/components/ScanList.tsx", "ScanList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/trivy/src/components/ScanList.tsx",
            "ScanList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_sorted_worst_first() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].critical_cves >= w[1].critical_cves);
        }
    }

    #[test]
    fn critical_only_filters_above_threshold() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let above_one = critical_only(&rows, 1);
        assert!(above_one.iter().all(|r| r.critical_cves >= 1));
        let above_huge = critical_only(&rows, 99_999);
        assert!(above_huge.is_empty());
    }

    #[test]
    fn detail_returns_scan_by_image() {
        let s = AdminState::seeded();
        let rows = list_records(&s, &ctx(&[Permission::ContainerScanRead])).unwrap();
        if let Some(first) = rows.first() {
            let image = first.image.clone();
            assert!(detail(&s, &ctx(&[Permission::ContainerScanRead]), &image)
                .unwrap()
                .is_some());
        }
        assert!(detail(
            &s,
            &ctx(&[Permission::ContainerScanRead]),
            "no:such",
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn scan_summary_aggregates() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let s = scan_summary(&rows);
        assert_eq!(s.images, rows.len() as u32);
        let expected: u32 = rows.iter().map(|r| r.critical_cves).sum();
        assert_eq!(s.total_criticals, expected);
    }

    #[test]
    fn render_contains_owner_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/trivy/src/components/ScanList.tsx",
            "RenderOwner",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(html.contains("web:v17"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(!html.contains("evil:latest"));
    }

    #[test]
    fn render_includes_summary_cards_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(html.contains("IMAGES"));
        assert!(html.contains("CRITICAL"));
        assert!(html.contains("trivy.dev"));
    }
}
