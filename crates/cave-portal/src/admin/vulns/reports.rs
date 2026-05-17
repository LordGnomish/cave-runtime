// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/vulns/reports` — executive summary.

use crate::admin::layout::shell::{shell_v2, ShellOptions};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::state::AdminState;
use crate::admin::vulns::VulnsViewError;

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, VulnsViewError> {
    ctx.authorise(Permission::VulnsRead)?;
    let body = r#"<section>
  <h2>Executive summary</h2>
  <p>The executive report aggregates: total / active findings, severity
  counts, SLA breaches (now + breaching-soon), and top vulnerable
  components.</p>
  <ul>
    <li><a href="/api/vulns/reports/executive">Download JSON</a></li>
    <li><a href="/api/vulns/reports/executive.html">Download HTML</a></li>
  </ul>
  <p><em>PDF export is a Phase 2 deliverable.</em></p>
</section>"#;
    Ok(shell_v2(ShellOptions {
        title: "vulns · reports",
        persona: ctx.persona,
        tenant_id: ctx.tenant.as_str(),
        current_path: "/admin/vulns/reports",
        body,
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_includes_download_links() {
        let ctx = RequestCtx::developer("acme", &[Permission::VulnsRead]);
        let html = render(&AdminState::seeded(), &ctx).unwrap();
        assert!(html.contains("/api/vulns/reports/executive"));
        assert!(html.contains("/api/vulns/reports/executive.html"));
    }
    #[test]
    fn render_refuses_without_perm() {
        assert!(render(&AdminState::seeded(), &RequestCtx::developer("acme", &[])).is_err());
    }
}
