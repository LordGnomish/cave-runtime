//! `/admin/vulns` view — CVE record browser by severity.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, VulnRecord};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VulnsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_vulns(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<VulnRecord>, VulnsViewError> {
    ctx.authorise(Permission::VulnsRead)?;
    Ok(scope(&state.vuln_records.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn list_by_severity(state: &AdminState, ctx: &RequestCtx, severity: &str) -> Result<Vec<VulnRecord>, VulnsViewError> {
    Ok(list_vulns(state, ctx)?.into_iter().filter(|v| v.severity == severity).collect())
}

pub fn unfixed_count(state: &AdminState, ctx: &RequestCtx) -> Result<usize, VulnsViewError> {
    Ok(list_vulns(state, ctx)?.iter().filter(|v| v.fixed_version.is_none()).count())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, VulnsViewError> {
    let v = list_vulns(state, ctx)?;
    let rows: Vec<Vec<String>> = v.iter().map(|x| vec![
        x.cve_id.clone(), x.package.clone(), x.installed_version.clone(),
        x.fixed_version.clone().unwrap_or_else(|| "—".into()),
        x.severity.into(),
    ]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">CVEs ({n})</h2>{tbl}</section>"#,
        n = v.len(),
        tbl = table(&["cve", "package", "installed", "fixed", "severity"], &rows),
    );
    Ok(page_shell(&format!("vulns · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/vulns/src/components/CveList.tsx", "CveList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/vulns/src/components/CveList.tsx", "CveList", "acme");
        let s = AdminState::seeded();
        let v = list_vulns(&s, &ctx(&[Permission::VulnsRead])).unwrap();
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_vulns(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_by_severity_filters() {
        let (_c, _t) = portal_test_ctx!("plugins/vulns/src/components/SeverityFilter.tsx", "SeverityFilter", "acme");
        let s = AdminState::seeded();
        let crit = list_by_severity(&s, &ctx(&[Permission::VulnsRead]), "Critical").unwrap();
        assert_eq!(crit.len(), 1);
        assert_eq!(crit[0].cve_id, "CVE-2025-0001");
    }

    #[test]
    fn unfixed_count_works() {
        let (_c, _t) = portal_test_ctx!("plugins/vulns/src/components/UnfixedBadge.tsx", "UnfixedBadge", "acme");
        let s = AdminState::seeded();
        assert_eq!(unfixed_count(&s, &ctx(&[Permission::VulnsRead])).unwrap(), 1);
    }

    #[test]
    fn render_excludes_evil_cve() {
        let (_c, _t) = portal_test_ctx!("plugins/vulns/src/components/CvePage.tsx", "CvePage", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::VulnsRead])).unwrap();
        assert!(html.contains("CVEs (2)"));
        assert!(html.contains("CVE-2025-0001"));
        assert!(!html.contains("CVE-2025-9999"));
    }
}
