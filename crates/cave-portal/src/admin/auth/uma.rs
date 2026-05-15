// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! `/admin/auth/uma` — UMA 2.0 admin browser.
//!
//! Read-only view of:
//!   - Registered resources per realm (cave-auth::uma::resource::ResourceStore)
//!   - Active permission tickets (one-shot, TTL-bounded)
//!   - A small RPT decoder (paste an RPT → see permissions)
//!
//! Operator-only — gated behind `Permission::AuthSessionsRead`. Live data
//! sources are injected by the runtime; tests use seeded fixtures.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone)]
pub struct UmaResourceRow {
    pub realm: String,
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub owner: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UmaTicketRow {
    pub realm: String,
    pub ticket: String,
    pub permissions: Vec<(String, Vec<String>)>, // rsid, scopes
    pub audience: Option<String>,
    pub ttl_secs: i64,
    pub created_at: i64,
}

/// Synthesised demo state until the runtime is wired up. The shape mirrors
/// what `cave_auth::uma::resource::ResourceStore::list()` will deliver.
pub fn seeded_resources(_state: &AdminState) -> Vec<UmaResourceRow> {
    vec![
        UmaResourceRow {
            realm: "main".into(),
            id: "rs-001".into(),
            name: "Photo Album · spring 2026".into(),
            scopes: vec!["view".into(), "edit".into(), "share".into()],
            owner: Some("alice".into()),
        },
        UmaResourceRow {
            realm: "main".into(),
            id: "rs-002".into(),
            name: "Confidential Memo".into(),
            scopes: vec!["read".into()],
            owner: Some("bob".into()),
        },
    ]
}

pub fn seeded_tickets(_state: &AdminState) -> Vec<UmaTicketRow> {
    vec![UmaTicketRow {
        realm: "main".into(),
        ticket: "PT-7ffa…b3a1".into(),
        permissions: vec![("rs-001".into(), vec!["view".into()])],
        audience: Some("rs-client".into()),
        ttl_secs: 60,
        created_at: 1_700_000_000,
    }]
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, super::AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let resources = seeded_resources(state);
    let tickets = seeded_tickets(state);

    let res_rows: Vec<Vec<String>> = resources
        .iter()
        .map(|r| {
            vec![
                escape(&r.realm),
                escape(&r.id),
                escape(&r.name),
                escape(&r.scopes.join(" ")),
                escape(r.owner.as_deref().unwrap_or("")),
            ]
        })
        .collect();
    let tic_rows: Vec<Vec<String>> = tickets
        .iter()
        .map(|t| {
            vec![
                escape(&t.realm),
                escape(&t.ticket),
                escape(
                    &t.permissions
                        .iter()
                        .map(|(r, s)| format!("{}:{}", r, s.join(",")))
                        .collect::<Vec<_>>()
                        .join("; "),
                ),
                escape(t.audience.as_deref().unwrap_or("")),
                t.ttl_secs.to_string(),
            ]
        })
        .collect();

    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">UMA 2.0 (User-Managed Access) — resources + permission tickets + RPT decoder.</p>
  <h2 class="text-lg font-semibold mb-2">Registered resources</h2>
  {res}
  <h2 class="text-lg font-semibold mt-6 mb-2">Active permission tickets</h2>
  {tic}
  <h2 class="text-lg font-semibold mt-6 mb-2">RPT decoder</h2>
  <form method="post" action="/admin/auth/uma/decode">
    <textarea name="rpt" rows="3" class="w-full font-mono text-xs border rounded p-2" placeholder="eyJhbGciOi…"></textarea>
    <button type="submit" class="mt-2 px-3 py-1 rounded bg-blue-600 text-white text-sm">Decode</button>
  </form>
</section>"#,
        res = table(&["realm", "id", "name", "scopes", "owner"], &res_rows),
        tic = table(
            &["realm", "ticket", "permissions", "audience", "ttl"],
            &tic_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/uma",
        "auth · uma",
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
    fn render_lists_seeded_resources() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("Photo Album"));
        assert!(html.contains("rs-001"));
    }

    #[test]
    fn render_requires_permission() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_rpt_decoder() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("RPT decoder"));
        assert!(html.contains("/admin/auth/uma/decode"));
    }

    #[test]
    fn seeded_tickets_have_permissions() {
        let t = seeded_tickets(&AdminState::seeded());
        assert!(!t.is_empty());
        assert!(!t[0].permissions.is_empty());
    }
}
