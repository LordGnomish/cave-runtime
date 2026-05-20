// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/vulns/engagements` — Engagement + Test runs.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py:1535,2163

use crate::admin::layout::shell::{ShellOptions, shell_v2};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;
use crate::admin::vulns::VulnsViewError;

pub const ENGAGEMENT_TYPES: &[&str] = &["Interactive", "CICD"];

pub const ENGAGEMENT_STATUS: &[&str] = &[
    "NotStarted",
    "Blocked",
    "Cancelled",
    "Completed",
    "InProgress",
    "OnHold",
    "WaitingForResource",
];

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, VulnsViewError> {
    ctx.authorise(Permission::VulnsRead)?;
    let body = format!(
        r#"<section>
  <h2>Engagements</h2>
  <p>An Engagement is a bounded testing window against a Product
  (e.g. "Q3 2026 pentest"). Each Test is a single scan run within it.</p>
  <h3>engagement_type ({n_t})</h3>
  {types}
  <h3>status ({n_s})</h3>
  {status}
  <h3>API</h3>
  <p>CRUD: <code>GET/POST /api/vulns/engagements</code>. Default
  target window: 30 days from creation; default status: InProgress.</p>
</section>"#,
        n_t = ENGAGEMENT_TYPES.len(),
        n_s = ENGAGEMENT_STATUS.len(),
        types = table(
            &["type"],
            &ENGAGEMENT_TYPES
                .iter()
                .map(|s| vec![s.to_string()])
                .collect::<Vec<_>>()
        ),
        status = table(
            &["status"],
            &ENGAGEMENT_STATUS
                .iter()
                .map(|s| vec![s.to_string()])
                .collect::<Vec<_>>()
        ),
    );
    Ok(shell_v2(ShellOptions {
        title: "vulns · engagements",
        persona: ctx.persona,
        tenant_id: ctx.tenant.as_str(),
        current_path: "/admin/vulns/engagements",
        body: &body,
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engagement_types_match_defectdojo() {
        assert_eq!(ENGAGEMENT_TYPES, &["Interactive", "CICD"]);
    }
    #[test]
    fn engagement_status_has_seven_values() {
        assert_eq!(ENGAGEMENT_STATUS.len(), 7);
    }
    #[test]
    fn render_passes_with_perm() {
        let ctx = RequestCtx::developer("acme", &[Permission::VulnsRead]);
        let html = render(&AdminState::seeded(), &ctx).unwrap();
        assert!(html.contains("Engagement"));
    }
}
