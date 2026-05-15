//! `/admin/container_scan/images` — Trivy "Images" tab. Per-image
//! roster with the latest scan timestamp. Operators reach this
//! tab when they need to see "which images do we even have on
//! the cluster".
//!
//! Upstream: <https://aquasecurity.github.io/trivy>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use super::ContainerScanViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRow {
    pub image: String,
    pub digest: String,
    pub last_scan_unix: i64,
    pub critical_cves: u32,
}

pub fn list_images(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ImageRow>, ContainerScanViewError> {
    let rows = super::vulnerabilities::list_vulnerable_sorted(state, ctx)?;
    let mut acc: std::collections::BTreeMap<String, ImageRow> = std::collections::BTreeMap::new();
    for r in rows {
        let entry = acc.entry(r.image.clone()).or_insert(ImageRow {
            image: r.image.clone(),
            digest: r.digest.clone(),
            last_scan_unix: r.scanned_at_unix,
            critical_cves: r.critical_cves,
        });
        if r.scanned_at_unix > entry.last_scan_unix {
            entry.last_scan_unix = r.scanned_at_unix;
            entry.digest = r.digest.clone();
            entry.critical_cves = r.critical_cves;
        }
    }
    Ok(acc.into_values().collect())
}

pub fn newest_scan_unix(rows: &[ImageRow]) -> Option<i64> {
    rows.iter().map(|r| r.last_scan_unix).max()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ContainerScanViewError> {
    let rows = list_images(state, ctx)?;
    let newest = newest_scan_unix(&rows).unwrap_or(0);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.image),
                escape(&r.digest),
                r.last_scan_unix.to_string(),
                r.critical_cves.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Images ({n}) · newest scan {ts}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-image roster. Upstream:
    <a class="text-blue-700 underline" href="https://aquasecurity.github.io/trivy">Trivy Images</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        ts = newest,
        tbl = table(
            &["image", "digest", "last_scan_unix", "critical_cves"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/container_scan/images",
        &format!("container_scan/images · {}", escape(ctx.tenant.as_str())),
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
    fn list_deduplicates_per_image() {
        let rows = list_images(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let mut seen = std::collections::HashSet::new();
        for r in &rows {
            assert!(seen.insert(r.image.clone()));
        }
    }

    #[test]
    fn newest_scan_unix_returns_max() {
        let rows = list_images(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let max = newest_scan_unix(&rows);
        let expected = rows.iter().map(|r| r.last_scan_unix).max();
        assert_eq!(max, expected);
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_images(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn image_count_matches_distinct_scans() {
        let scans = super::super::vulnerabilities::list_vulnerable_sorted(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let distinct: std::collections::HashSet<_> = scans.iter().map(|s| s.image.clone()).collect();
        let rows = list_images(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert_eq!(rows.len(), distinct.len());
    }

    #[test]
    fn render_includes_image_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(html.contains("Images ("));
        assert!(html.contains("Trivy Images"));
    }
}
