//! `/admin/container_scan/vulnerabilities` — Trivy "Vulnerabilities"
//! tab. Surfaces the per-image CVE count. Operators sort by
//! critical descending to find images that need re-baked first.
//!
//! Upstream: <https://aquasecurity.github.io/trivy>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, ContainerScanResult};
use super::ContainerScanViewError;

pub fn list_vulnerable_sorted(
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
    rows.sort_by(|a, b| b.critical_cves.cmp(&a.critical_cves).then(a.image.cmp(&b.image)));
    Ok(rows)
}

pub fn total_critical(rows: &[ContainerScanResult]) -> u64 {
    rows.iter().map(|r| r.critical_cves as u64).sum()
}

pub fn images_with_zero_critical<'a>(rows: &'a [ContainerScanResult]) -> Vec<&'a ContainerScanResult> {
    rows.iter().filter(|r| r.critical_cves == 0).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ContainerScanViewError> {
    let rows = list_vulnerable_sorted(state, ctx)?;
    let total = total_critical(&rows);
    let clean = images_with_zero_critical(&rows).len();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.image),
                escape(&r.digest),
                r.critical_cves.to_string(),
                r.scanned_at_unix.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Vulnerabilities — {total} critical · {clean} clean image(s)</h2>
  <p class="text-sm text-gray-600 mb-3">
    Sorted by critical CVE count desc. Upstream:
    <a class="text-blue-700 underline" href="https://aquasecurity.github.io/trivy">Trivy</a>.
  </p>
  {tbl}
</section>"#,
        total = total,
        clean = clean,
        tbl = table(&["image", "digest", "critical_cves", "scanned_at"], &table_rows),
    );
    Ok(page_shell(
        &format!("container_scan/vulnerabilities · {}", escape(ctx.tenant.as_str())),
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
    fn list_returns_scans_for_tenant() {
        let rows = list_vulnerable_sorted(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(rows.iter().all(|r| r.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_sorted_by_critical_desc() {
        let rows = list_vulnerable_sorted(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].critical_cves >= w[1].critical_cves);
        }
    }

    #[test]
    fn total_critical_sums_cve_field() {
        let rows = list_vulnerable_sorted(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let total = total_critical(&rows);
        let expected: u64 = rows.iter().map(|r| r.critical_cves as u64).sum();
        assert_eq!(total, expected);
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_vulnerable_sorted(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_critical_label() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(html.contains("Vulnerabilities"));
        assert!(html.contains("Trivy"));
    }
}
