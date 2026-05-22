// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/vulns/sla` — SLA window config + per-severity breach rollup.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py:999

use crate::admin::layout::shell::{ShellOptions, shell_v2};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;
use crate::admin::vulns::VulnsViewError;

/// Default SLA window in days per severity. Source: ADR-035.
pub const DEFAULT_SLA: &[(&str, u32)] =
    &[("Critical", 7), ("High", 30), ("Medium", 90), ("Low", 180)];

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, VulnsViewError> {
    ctx.authorise(Permission::VulnsRead)?;
    let rows: Vec<Vec<String>> = DEFAULT_SLA
        .iter()
        .map(|(s, d)| vec![s.to_string(), format!("{d}d")])
        .collect();
    let body = format!(
        r#"<section>
  <h2>SLA configuration</h2>
  <p>Each Finding's SLA deadline is anchored to its <code>date</code>
  (DefectDojo's <code>sla_start_date</code> falls back to
  <code>date</code>). A Finding is <em>breached</em> when
  <code>now &gt; date + days_for(severity)</code>.</p>
  <h3>Default windows (charter ADR-035)</h3>
  {tbl}
  <p>Info severity is intentionally untracked.</p>
  <h3>Live rollup</h3>
  <p>Endpoint: <code>GET /api/vulns/sla</code> — returns config +
  total / breached / breaching-soon (≤7d) counts by severity.</p>
</section>"#,
        tbl = table(&["severity", "window"], &rows),
    );
    Ok(shell_v2(ShellOptions {
        title: "vulns · sla",
        persona: ctx.persona,
        tenant_id: ctx.tenant.as_str(),
        current_path: "/admin/vulns/sla",
        body: &body,
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_sla_matches_charter() {
        assert_eq!(
            DEFAULT_SLA,
            &[("Critical", 7), ("High", 30), ("Medium", 90), ("Low", 180)]
        );
    }
    #[test]
    fn render_includes_breach_definition() {
        let ctx = RequestCtx::developer("acme", &[Permission::VulnsRead]);
        let html = render(&AdminState::seeded(), &ctx).unwrap();
        assert!(html.contains("breached"));
        assert!(html.contains("/api/vulns/sla"));
    }
}
