// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/vulns/scans` — registered parsers + import surface.

use crate::admin::layout::shell::{shell_v2, ShellOptions};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;
use crate::admin::vulns::VulnsViewError;

pub const PARSERS: &[(&str, &str, &str)] = &[
    ("Bandit Scan", "JSON", "Python SAST (bandit -f json)"),
    ("Trivy Scan", "JSON", "Container / IaC / secret scanner"),
    ("ZAP Scan", "XML", "OWASP ZAP DAST report"),
    ("Semgrep JSON Report", "JSON", "semgrep --json"),
    ("SARIF", "JSON", "OASIS SARIF v2.1.0 (CodeQL, ESLint, ...)"),
    ("Snyk Scan", "JSON", "snyk test --json"),
    ("Nuclei Scan", "JSONL/JSON", "ProjectDiscovery nuclei"),
];

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, VulnsViewError> {
    ctx.authorise(Permission::VulnsRead)?;
    let rows: Vec<Vec<String>> = PARSERS.iter()
        .map(|(s, f, d)| vec![s.to_string(), f.to_string(), d.to_string()]).collect();
    let body = format!(
        r#"<section>
  <h2>Registered parsers ({n})</h2>
  <p>Import surface: <code>POST /api/vulns/import-scan</code> with body
  <code>{{"scan_type": "...", "content": "..."}}</code>. Optional
  <code>dedup</code> override (legacy / hash_code / unique_id_from_tool /
  unique_id_from_tool_or_hash_code).</p>
  {tbl}
</section>"#,
        n = PARSERS.len(),
        tbl = table(&["scan_type", "format", "description"], &rows),
    );
    Ok(shell_v2(ShellOptions {
        title: "vulns · scans",
        persona: ctx.persona,
        tenant_id: ctx.tenant.as_str(),
        current_path: "/admin/vulns/scans",
        body: &body,
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_list_has_seven_entries() {
        assert_eq!(PARSERS.len(), 7);
    }
    #[test]
    fn render_includes_import_endpoint() {
        let ctx = RequestCtx::developer("acme", &[Permission::VulnsRead]);
        let html = render(&AdminState::seeded(), &ctx).unwrap();
        assert!(html.contains("/api/vulns/import-scan"));
    }
    #[test]
    fn render_refuses_without_perm() {
        assert!(render(&AdminState::seeded(), &RequestCtx::developer("acme", &[])).is_err());
    }
}
