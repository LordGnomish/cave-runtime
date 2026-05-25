// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/vulns/dedup` — dedup algorithm + scanner mapping reference.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 settings.dist.py:978-1135
//!         (HASHCODE_FIELDS_PER_SCANNER) + dojo/finding/deduplication.py

use crate::admin::layout::shell::{ShellOptions, shell_v2};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;
use crate::admin::vulns::VulnsViewError;

/// The four canonical DefectDojo dedup algorithms.
pub const ALGORITHMS: &[(&str, &str)] = &[
    ("legacy", "title + cwe + line + file_path + description"),
    ("hash_code", "SHA-256 of scanner-specific fields"),
    (
        "unique_id_from_tool",
        "stable scanner-generated fingerprint",
    ),
    (
        "unique_id_from_tool_or_hash_code",
        "uid first, hash_code fallback",
    ),
];

/// Per-scanner field tuples — port of HASHCODE_FIELDS_PER_SCANNER.
pub const SCANNERS: &[(&str, &str)] = &[
    ("Bandit Scan", "file_path / line / vuln_id_from_tool"),
    ("ZAP Scan", "title / cwe / severity"),
    (
        "Trivy Scan",
        "title / severity / vulnerability_ids / cwe / description",
    ),
    (
        "Semgrep JSON Report",
        "title / cwe / line / file_path / description",
    ),
    ("SARIF", "title / cwe / line / file_path / description"),
    (
        "Snyk Scan",
        "vuln_id_from_tool / file_path / component_name / component_version",
    ),
    ("Nuclei Scan", "title / severity / vuln_id_from_tool"),
    (
        "Anchore Grype",
        "title / severity / component_name / component_version",
    ),
    (
        "Aqua Scan",
        "severity / vulnerability_ids / component_name / component_version",
    ),
    ("Burp Scan", "title / severity / vuln_id_from_tool"),
    (
        "CargoAudit Scan",
        "vulnerability_ids / severity / component_name / component_version / vuln_id_from_tool",
    ),
    ("Checkmarx Scan", "cwe / severity / file_path"),
    ("SonarQube Scan", "cwe / severity / file_path"),
    ("Dependency Check Scan", "title / cwe / file_path"),
    (
        "NPM Audit Scan",
        "title / severity / file_path / vulnerability_ids / cwe",
    ),
    (
        "Yarn Audit Scan",
        "title / severity / file_path / vulnerability_ids / cwe",
    ),
    (
        "GitLab Dependency Scanning Report",
        "title / vulnerability_ids / file_path / component_name / component_version",
    ),
    (
        "Github SAST Scan",
        "vuln_id_from_tool / severity / file_path / line",
    ),
    (
        "TFSec Scan",
        "severity / vuln_id_from_tool / file_path / line",
    ),
    (
        "Tenable Scan",
        "title / severity / vulnerability_ids / cwe / description",
    ),
];

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, VulnsViewError> {
    ctx.authorise(Permission::VulnsRead)?;
    let algo_rows: Vec<Vec<String>> = ALGORITHMS
        .iter()
        .map(|(a, d)| vec![a.to_string(), d.to_string()])
        .collect();
    let scan_rows: Vec<Vec<String>> = SCANNERS
        .iter()
        .map(|(s, f)| vec![s.to_string(), f.to_string()])
        .collect();
    let body = format!(
        r#"<section>
  <h2>Algorithms ({n_a})</h2>
  {algos}
  <h2 style="margin-top:1.5rem">Per-scanner field tuples ({n_s})</h2>
  <p>Hash_code dedup hashes the listed fields (plus the always-included <code>service</code>) with SHA-256.</p>
  {scans}
</section>"#,
        n_a = ALGORITHMS.len(),
        n_s = SCANNERS.len(),
        algos = table(&["algorithm", "fields / behaviour"], &algo_rows),
        scans = table(&["scanner", "hash_code fields"], &scan_rows),
    );
    Ok(shell_v2(ShellOptions {
        title: "vulns · dedup",
        persona: ctx.persona,
        tenant_id: ctx.tenant.as_str(),
        current_path: "/admin/vulns/dedup",
        body: &body,
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_algorithms_documented() {
        assert_eq!(ALGORITHMS.len(), 4);
        assert!(ALGORITHMS.iter().any(|(a, _)| *a == "legacy"));
        assert!(ALGORITHMS.iter().any(|(a, _)| *a == "hash_code"));
    }

    #[test]
    fn all_seven_shipped_parsers_in_scanner_table() {
        for s in [
            "Bandit Scan",
            "ZAP Scan",
            "Trivy Scan",
            "Semgrep JSON Report",
            "SARIF",
            "Snyk Scan",
            "Nuclei Scan",
        ] {
            assert!(SCANNERS.iter().any(|(n, _)| *n == s), "missing {s}");
        }
    }

    #[test]
    fn render_returns_html() {
        let ctx = RequestCtx::developer("acme", &[Permission::VulnsRead]);
        let html = render(&AdminState::seeded(), &ctx).unwrap();
        assert!(html.contains("Algorithms"));
        assert!(html.contains("Bandit Scan"));
    }

    #[test]
    fn render_refuses_without_perm() {
        let ctx = RequestCtx::developer("acme", &[]);
        assert!(render(&AdminState::seeded(), &ctx).is_err());
    }
}
