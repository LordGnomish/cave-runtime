// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/container_scan/history` — Trivy "Scan History" tab.
//! Surfaces every scan execution in reverse-chronological order
//! — operators land here to answer "when was this image last
//! checked".
//!
//! Upstream: <https://aquasecurity.github.io/trivy>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, ContainerScanResult};
use super::ContainerScanViewError;

pub fn list_history(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ContainerScanResult>, ContainerScanViewError> {
    let mut rows = super::vulnerabilities::list_vulnerable_sorted(state, ctx)?;
    rows.sort_by(|a, b| b.scanned_at_unix.cmp(&a.scanned_at_unix));
    Ok(rows)
}

pub fn oldest_unix(rows: &[ContainerScanResult]) -> Option<i64> {
    rows.iter().map(|r| r.scanned_at_unix).min()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ContainerScanViewError> {
    let rows = list_history(state, ctx)?;
    let oldest = oldest_unix(&rows).unwrap_or(0);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.scanned_at_unix.to_string(),
                escape(&r.image),
                escape(&r.digest),
                r.critical_cves.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Scan History ({n}) · oldest {ts}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Reverse-chronological scan log. Upstream:
    <a class="text-blue-700 underline" href="https://aquasecurity.github.io/trivy">Trivy</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        ts = oldest,
        tbl = table(&["scanned_at", "image", "digest", "critical"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/container_scan/history",
        &format!("container_scan/history · {}", escape(ctx.tenant.as_str())),
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
    fn history_sorted_newest_first() {
        let rows = list_history(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].scanned_at_unix >= w[1].scanned_at_unix);
        }
    }

    #[test]
    fn oldest_unix_matches_min() {
        let rows = list_history(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let oldest = oldest_unix(&rows);
        let expected = rows.iter().map(|r| r.scanned_at_unix).min();
        assert_eq!(oldest, expected);
    }

    #[test]
    fn history_rejects_without_permission() {
        assert!(list_history(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn history_count_matches_seeded_scans() {
        let scans = super::super::vulnerabilities::list_vulnerable_sorted(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let rows = list_history(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert_eq!(rows.len(), scans.len());
    }

    #[test]
    fn render_includes_history_count() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(html.contains("Scan History ("));
    }
}
