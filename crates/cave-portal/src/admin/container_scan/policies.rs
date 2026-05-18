// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/container_scan/policies` — Trivy "Policies" tab.
//! Surfaces the admission-gate rules cave-streams enforces
//! against scan output. cave-portal exposes the canonical set
//! (CRITICAL_CVE_THRESHOLD, MAX_AGE_DAYS, REGISTRY_ALLOWLIST)
//! along with the count of images that would fail each rule.
//!
//! Upstream: <https://aquasecurity.github.io/trivy/latest/docs/configuration/filtering/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use super::ContainerScanViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRow {
    pub policy_id: &'static str,
    pub description: &'static str,
    pub threshold: String,
    pub matching_images: usize,
}

pub fn list_policies(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<PolicyRow>, ContainerScanViewError> {
    let images = super::images::list_images(state, ctx)?;
    let critical_threshold = 1u32;
    let critical_match = images.iter().filter(|i| i.critical_cves >= critical_threshold).count();
    let age_threshold_unix = 1_000_000_i64;
    let stale_match = images.iter().filter(|i| i.last_scan_unix < age_threshold_unix).count();
    let allowlist = ["docker.io", "quay.io", "gcr.io"];
    let off_registry = images
        .iter()
        .filter(|i| !allowlist.iter().any(|r| i.image.contains(r)))
        .count();
    Ok(vec![
        PolicyRow {
            policy_id: "CRITICAL_CVE_THRESHOLD",
            description: "Block images with ≥1 CRITICAL CVE",
            threshold: critical_threshold.to_string(),
            matching_images: critical_match,
        },
        PolicyRow {
            policy_id: "MAX_AGE_UNIX",
            description: "Re-scan images older than threshold",
            threshold: age_threshold_unix.to_string(),
            matching_images: stale_match,
        },
        PolicyRow {
            policy_id: "REGISTRY_ALLOWLIST",
            description: "Allow only docker.io / quay.io / gcr.io",
            threshold: allowlist.join(","),
            matching_images: off_registry,
        },
    ])
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ContainerScanViewError> {
    let rows = list_policies(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.policy_id.to_string(),
                r.description.to_string(),
                escape(&r.threshold),
                r.matching_images.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Policies ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Admission-gate rules. Upstream:
    <a class="text-blue-700 underline" href="https://aquasecurity.github.io/trivy/latest/docs/configuration/filtering/">Trivy filtering</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["policy_id", "description", "threshold", "matching"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/container_scan/policies",
        &format!("container_scan/policies · {}", escape(ctx.tenant.as_str())),
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
    fn list_returns_three_canonical_policies() {
        let rows = list_policies(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn critical_policy_count_matches_image_filter() {
        let images = super::super::images::list_images(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let expected = images.iter().filter(|i| i.critical_cves >= 1).count();
        let rows = list_policies(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let critical = rows.iter().find(|r| r.policy_id == "CRITICAL_CVE_THRESHOLD").unwrap();
        assert_eq!(critical.matching_images, expected);
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_policies(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn registry_allowlist_contains_canonical_set() {
        let rows = list_policies(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        let reg = rows.iter().find(|r| r.policy_id == "REGISTRY_ALLOWLIST").unwrap();
        assert!(reg.threshold.contains("docker.io"));
        assert!(reg.threshold.contains("quay.io"));
        assert!(reg.threshold.contains("gcr.io"));
    }

    #[test]
    fn render_includes_policies_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ContainerScanRead])).unwrap();
        assert!(html.contains("Policies ("));
        assert!(html.contains("Trivy filtering"));
    }
}
