// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/sbom/notifications` — Dependency-Track "Notifications" panel.
//! Lists the default sinks (Slack / Teams / Email / Jira / Webhook) and
//! the matched event-group set.
//!
//! Upstream: <https://dependencytrack.org/docs/integrations/notifications/>

use super::SbomViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq)]
pub struct NotificationRow {
    pub publisher: &'static str,
    pub enabled: bool,
    pub events: &'static [&'static str],
    pub min_level: &'static str,
}

pub fn list() -> Vec<NotificationRow> {
    vec![
        NotificationRow {
            publisher: "Slack",
            enabled: true,
            events: &["NEW_VULNERABILITY", "POLICY_VIOLATION"],
            min_level: "WARNING",
        },
        NotificationRow {
            publisher: "Teams",
            enabled: false,
            events: &["NEW_VULNERABILITY"],
            min_level: "ERROR",
        },
        NotificationRow {
            publisher: "Email",
            enabled: true,
            events: &["NEW_VULNERABILITY", "POLICY_VIOLATION", "BOM_PROCESSED"],
            min_level: "INFORMATIONAL",
        },
        NotificationRow {
            publisher: "Jira",
            enabled: false,
            events: &["POLICY_VIOLATION"],
            min_level: "ERROR",
        },
        NotificationRow {
            publisher: "Webhook",
            enabled: true,
            events: &["BOM_CONSUMED", "BOM_PROCESSED"],
            min_level: "INFORMATIONAL",
        },
    ]
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SbomViewError> {
    ctx.authorise(Permission::SbomRead)?;
    let _ = state;
    let rows = list();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.publisher.to_string(),
                if r.enabled { "yes" } else { "no" }.to_string(),
                r.events.join(", "),
                r.min_level.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Notification sinks ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">Pluggable sinks; each rule filters by event-group + minimum level.</p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["publisher", "enabled", "events", "min_level"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/sbom/notifications",
        &format!("sbom/notifications · {}", escape(ctx.tenant.as_str())),
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
    fn list_includes_five_publishers() {
        let r = list();
        assert_eq!(r.len(), 5);
        let names: Vec<&str> = r.iter().map(|x| x.publisher).collect();
        assert!(names.contains(&"Slack"));
        assert!(names.contains(&"Jira"));
        assert!(names.contains(&"Email"));
        assert!(names.contains(&"Webhook"));
        assert!(names.contains(&"Teams"));
    }

    #[test]
    fn render_requires_perm() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_publisher_column() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert!(html.contains("publisher"));
        assert!(html.contains("Slack"));
    }
}
