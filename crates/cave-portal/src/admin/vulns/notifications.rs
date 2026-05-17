// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/vulns/notifications` — sink registry + event types.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/notifications/helper.py

use crate::admin::layout::shell::{shell_v2, ShellOptions};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;
use crate::admin::vulns::VulnsViewError;

pub const SINKS: &[(&str, &str, &str)] = &[
    ("InMemorySink", "memory", "Used by tests + Portal recent-events feed"),
    ("WebhookSink (SlackCompatible)", "https POST", "{\"text\": \"…\"} — Slack / Teams"),
    ("WebhookSink (Raw)", "https POST", "Full event JSON for custom consumers"),
    ("LogSink", "tracing::info!", "Structured logs only"),
    ("JiraSink", "trait-shaped (Phase 2)", "Requires cave-vault credential plumbing"),
    ("EmailSink", "trait-shaped (Phase 2)", "Requires cave-vault SMTP plumbing"),
];

pub const EVENT_KINDS: &[&str] = &[
    "finding_created",
    "severity_increased",
    "sla_breach_imminent",
    "sla_breached",
    "risk_acceptance_expired",
    "scan_failed",
    "scan_completed",
];

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, VulnsViewError> {
    ctx.authorise(Permission::VulnsRead)?;
    let sink_rows: Vec<Vec<String>> = SINKS.iter()
        .map(|(n, t, d)| vec![n.to_string(), t.to_string(), d.to_string()]).collect();
    let kind_rows: Vec<Vec<String>> = EVENT_KINDS.iter()
        .map(|k| vec![k.to_string()]).collect();
    let body = format!(
        r#"<section>
  <h2>Notification sinks ({n_s})</h2>
  {sinks}
  <h2 style="margin-top:1.5rem">Event kinds ({n_k})</h2>
  {kinds}
</section>"#,
        n_s = SINKS.len(),
        n_k = EVENT_KINDS.len(),
        sinks = table(&["sink", "transport", "notes"], &sink_rows),
        kinds = table(&["event_kind"], &kind_rows),
    );
    Ok(shell_v2(ShellOptions {
        title: "vulns · notifications",
        persona: ctx.persona,
        tenant_id: ctx.tenant.as_str(),
        current_path: "/admin/vulns/notifications",
        body: &body,
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn at_least_three_sinks_documented() {
        assert!(SINKS.len() >= 4);
    }
    #[test]
    fn seven_event_kinds() {
        assert_eq!(EVENT_KINDS.len(), 7);
    }
    #[test]
    fn render_includes_sinks() {
        let ctx = RequestCtx::developer("acme", &[Permission::VulnsRead]);
        let html = render(&AdminState::seeded(), &ctx).unwrap();
        assert!(html.contains("InMemorySink"));
        assert!(html.contains("LogSink"));
    }
}
