//! `/admin/sbom/vulnerabilities` — Dependency-Track "Vulnerabilities" tab.
//! Synthesises a CVE roster from the seeded component set (illustrative;
//! live wiring routes through cave-sbom REST `/api/v1/vulnerability`).
//!
//! Upstream: <https://dependencytrack.org/docs/glossary/#vulnerability>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, SbomComponent};
use super::SbomViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VulnRow {
    pub component: String,
    pub version: String,
    pub cve_id: String,
    pub severity: &'static str,
}

/// Pre-canned CVE catalogue keyed by component name. Mirrors the
/// Dependency-Track demo data set used by the upstream UI screenshots.
fn catalogue(component: &str) -> Option<(&'static str, &'static str)> {
    match component {
        "openssl" => Some(("CVE-2023-5363", "HIGH")),
        "tokio" => Some(("CVE-2024-3566", "MEDIUM")),
        "log4j" => Some(("CVE-2021-44228", "CRITICAL")),
        _ => None,
    }
}

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<VulnRow>, SbomViewError> {
    ctx.authorise(Permission::SbomRead)?;
    let rows: Vec<SbomComponent> = scope(
        &state.sbom_components.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    let mut out: Vec<VulnRow> = rows
        .iter()
        .filter_map(|c| {
            catalogue(c.package.as_str()).map(|(cve, sev)| VulnRow {
                component: c.package.clone(),
                version: c.version.clone(),
                cve_id: cve.to_string(),
                severity: sev,
            })
        })
        .collect();
    out.sort_by(|a, b| severity_rank(a.severity).cmp(&severity_rank(b.severity)).reverse());
    Ok(out)
}

fn severity_rank(s: &str) -> u8 {
    match s {
        "CRITICAL" => 4,
        "HIGH" => 3,
        "MEDIUM" => 2,
        "LOW" => 1,
        _ => 0,
    }
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SbomViewError> {
    let rows = list(state, ctx)?;
    let critical = rows.iter().filter(|r| r.severity == "CRITICAL").count();
    let high = rows.iter().filter(|r| r.severity == "HIGH").count();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![
            escape(&r.cve_id),
            escape(&r.component),
            escape(&r.version),
            r.severity.to_string(),
        ])
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">CVE roster derived from component-name lookup against a small Dependency-Track-style catalogue.</p>
  <div class="mb-4 grid grid-cols-3 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">TOTAL</div><div class="text-2xl font-bold">{n}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">CRITICAL</div><div class="text-2xl font-bold text-red-700">{c}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">HIGH</div><div class="text-2xl font-bold text-orange-700">{h}</div></div>
  </div>
  {tbl}
</section>"#,
        n = rows.len(), c = critical, h = high,
        tbl = table(&["cve", "component", "version", "severity"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/sbom/vulnerabilities",
        &format!("sbom/vulnerabilities · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_rejects_no_perm() {
        assert!(list(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_sorted_worst_first() {
        let rows = list(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        for w in rows.windows(2) {
            assert!(severity_rank(w[0].severity) >= severity_rank(w[1].severity));
        }
    }

    #[test]
    fn severity_rank_is_monotonic() {
        assert!(severity_rank("CRITICAL") > severity_rank("HIGH"));
        assert!(severity_rank("HIGH") > severity_rank("MEDIUM"));
    }

    #[test]
    fn render_includes_severity_cards() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert!(html.contains("CRITICAL"));
        assert!(html.contains("HIGH"));
    }
}
