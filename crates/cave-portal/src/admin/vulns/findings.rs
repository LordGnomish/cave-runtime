// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/vulns/findings` — Finding triage list.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 templates/dojo/findings_list.html

use crate::admin::layout::shell::{ShellOptions, shell_v2};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{AdminState, VulnRecord, scope};
use crate::admin::vulns::VulnsViewError;

pub fn list_findings(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VulnRecord>, VulnsViewError> {
    ctx.authorise(Permission::VulnsRead)?;
    Ok(
        scope(&state.vuln_records.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect(),
    )
}

pub fn list_by_state(
    state: &AdminState,
    ctx: &RequestCtx,
    only_unfixed: bool,
) -> Result<Vec<VulnRecord>, VulnsViewError> {
    let mut all = list_findings(state, ctx)?;
    if only_unfixed {
        all.retain(|f| f.fixed_version.is_none());
    }
    Ok(all)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, VulnsViewError> {
    let findings = list_findings(state, ctx)?;
    let rows: Vec<Vec<String>> = findings
        .iter()
        .map(|f| {
            vec![
                escape(&f.cve_id),
                escape(&f.package),
                escape(&f.installed_version),
                f.fixed_version
                    .as_deref()
                    .map(escape)
                    .unwrap_or_else(|| "—".to_string()),
                f.severity.to_string(),
                if f.fixed_version.is_some() {
                    "fixed_available".into()
                } else {
                    "active".into()
                },
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2>Findings ({n})</h2>
  <p>Triage view with state hints. Source-of-truth: <code>cave-vulns</code> <code>Finding</code> model (DefectDojo-parity).</p>
  {tbl}
</section>"#,
        n = findings.len(),
        tbl = table(
            &["cve", "package", "installed", "fixed", "severity", "state"],
            &rows
        ),
    );
    Ok(shell_v2(ShellOptions {
        title: "vulns · findings",
        persona: ctx.persona,
        tenant_id: ctx.tenant.as_str(),
        current_path: "/admin/vulns/findings",
        body: &body,
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_tenant() {
        let s = AdminState::seeded();
        let v = list_findings(&s, &ctx(&[Permission::VulnsRead])).unwrap();
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_findings(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn unfixed_filter_excludes_resolved() {
        let v = list_by_state(&AdminState::seeded(), &ctx(&[Permission::VulnsRead]), true).unwrap();
        assert_eq!(v.len(), 1, "1 unfixed in acme tenant seed");
    }

    #[test]
    fn render_includes_h2_count() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::VulnsRead])).unwrap();
        assert!(html.contains("Findings (2)"));
    }
}
