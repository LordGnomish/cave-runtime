// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/events` — Login + admin event log with filter form.
//! Visual port of `js/apps/admin-ui/src/events/EventsSection.tsx`.

use std::collections::BTreeMap;

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};
use super::{render_admin_nav, require_platform, AuthAdminError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub time_iso: String,
    pub kind: EventKind,
    pub realm: String,
    pub client: String,
    pub user: String,
    pub ip: String,
    pub error: Option<String>,
    pub details: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    LoginSuccess,
    LoginError,
    Logout,
    CodeToToken,
    RefreshToken,
    UserUpdate,
    ClientCreate,
    AdminAction,
}
impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LoginSuccess => "LOGIN",
            Self::LoginError => "LOGIN_ERROR",
            Self::Logout => "LOGOUT",
            Self::CodeToToken => "CODE_TO_TOKEN",
            Self::RefreshToken => "REFRESH_TOKEN",
            Self::UserUpdate => "UPDATE_PROFILE",
            Self::ClientCreate => "CREATE_CLIENT",
            Self::AdminAction => "ADMIN",
        }
    }
    pub fn badge_class(&self) -> &'static str {
        match self {
            Self::LoginError => "text-red-700",
            Self::AdminAction | Self::ClientCreate => "text-amber-700",
            _ => "text-green-700",
        }
    }
}

pub fn seeded_events() -> Vec<EventRow> {
    let mut details = BTreeMap::new();
    details.insert("auth_method".into(), "openid-connect".into());
    let mut details_err = BTreeMap::new();
    details_err.insert("auth_method".into(), "openid-connect".into());
    details_err.insert("username".into(), "mallory".into());
    vec![
        EventRow {
            time_iso: "2026-05-15T08:30:00Z".into(),
            kind: EventKind::LoginSuccess,
            realm: "cave".into(),
            client: "cave-portal".into(),
            user: "admin".into(),
            ip: "10.0.0.42".into(),
            error: None,
            details: details.clone(),
        },
        EventRow {
            time_iso: "2026-05-15T08:31:12Z".into(),
            kind: EventKind::LoginError,
            realm: "cave".into(),
            client: "cave-portal".into(),
            user: "mallory".into(),
            ip: "203.0.113.7".into(),
            error: Some("invalid_user_credentials".into()),
            details: details_err,
        },
        EventRow {
            time_iso: "2026-05-15T08:32:00Z".into(),
            kind: EventKind::CodeToToken,
            realm: "cave".into(),
            client: "cavectl".into(),
            user: "acme-dev".into(),
            ip: "10.0.0.43".into(),
            error: None,
            details,
        },
        EventRow {
            time_iso: "2026-05-15T08:35:01Z".into(),
            kind: EventKind::ClientCreate,
            realm: "cave".into(),
            client: "security-team".into(),
            user: "admin".into(),
            ip: "10.0.0.42".into(),
            error: None,
            details: BTreeMap::new(),
        },
    ]
}

pub fn filter_events<'a>(rows: &'a [EventRow], kind: Option<EventKind>, user: Option<&str>) -> Vec<&'a EventRow> {
    rows.iter()
        .filter(|e| kind.map_or(true, |k| e.kind == k))
        .filter(|e| user.map_or(true, |u| e.user.contains(u)))
        .collect()
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let events = seeded_events();
    let rows: Vec<Vec<String>> = events
        .iter()
        .map(|e| {
            let kind_cell = format!(
                r#"<span class="text-xs {cls} font-mono">{k}</span>"#,
                cls = e.kind.badge_class(),
                k = e.kind.as_str()
            );
            let err_cell = match &e.error {
                Some(err) => format!(r#"<code class="text-xs text-red-700">{}</code>"#, escape(err)),
                None => "".to_string(),
            };
            vec![
                escape(&e.time_iso),
                kind_cell,
                escape(&e.realm),
                escape(&e.client),
                escape(&e.user),
                format!(r#"<code class="text-xs">{}</code>"#, escape(&e.ip)),
                err_cell,
            ]
        })
        .collect();
    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-3">Events</h2>
  <form method="get" action="/admin/auth/events" class="flex flex-wrap gap-2 mb-3" role="search">
    <label class="text-sm">type
      <select class="ml-1 rounded border-gray-300 dark:bg-zinc-800" name="type">
        <option value="">any</option>
        <option value="LOGIN">LOGIN</option>
        <option value="LOGIN_ERROR">LOGIN_ERROR</option>
        <option value="CODE_TO_TOKEN">CODE_TO_TOKEN</option>
        <option value="REFRESH_TOKEN">REFRESH_TOKEN</option>
        <option value="LOGOUT">LOGOUT</option>
        <option value="ADMIN">ADMIN</option>
      </select>
    </label>
    <label class="text-sm">user
      <input class="ml-1 rounded border-gray-300 dark:bg-zinc-800" name="user" placeholder="username substring">
    </label>
    <label class="text-sm">client
      <input class="ml-1 rounded border-gray-300 dark:bg-zinc-800" name="client" placeholder="client_id">
    </label>
    <button type="submit" class="px-3 py-1 rounded bg-zinc-200 dark:bg-zinc-700">Filter</button>
    <a class="px-3 py-1 rounded bg-zinc-100 dark:bg-zinc-800" href="/admin/auth/events.csv">Export CSV</a>
  </form>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Login + admin events. Showing {n} most recent.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_events_resource">Keycloak Events</a>.
  </p>
  {tbl}
</section>"#,
        nav = render_admin_nav("/admin/auth/events"),
        n = events.len(),
        tbl = table_html(
            &["time", "type", "realm", "client", "user", "ip", "error"],
            &rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/events",
        &format!("auth/events · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn seeded_events_includes_success_error_and_admin_kinds() {
        let e = seeded_events();
        assert!(e.iter().any(|x| matches!(x.kind, EventKind::LoginSuccess)));
        assert!(e.iter().any(|x| matches!(x.kind, EventKind::LoginError)));
        assert!(e.iter().any(|x| matches!(x.kind, EventKind::ClientCreate)));
    }

    #[test]
    fn filter_events_by_kind_returns_only_matching() {
        let e = seeded_events();
        let filtered = filter_events(&e, Some(EventKind::LoginError), None);
        assert!(!filtered.is_empty());
        assert!(filtered.iter().all(|x| matches!(x.kind, EventKind::LoginError)));
    }

    #[test]
    fn filter_events_by_user_substring_matches() {
        let e = seeded_events();
        let filtered = filter_events(&e, None, Some("admin"));
        assert!(filtered.iter().all(|x| x.user.contains("admin")));
    }

    #[test]
    fn event_kind_badge_red_for_errors() {
        assert_eq!(EventKind::LoginError.badge_class(), "text-red-700");
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_filter_form_with_type_user_client_inputs() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains(r#"name="type""#));
        assert!(html.contains(r#"name="user""#));
        assert!(html.contains(r#"name="client""#));
        assert!(html.contains("Export CSV"));
    }

    #[test]
    fn render_shows_error_codes_in_error_column() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("invalid_user_credentials"));
    }
}
