// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! `/admin/auth/token-exchange` — RFC 8693 token-exchange browser.
//!
//! Visualises recent exchanges (impersonation vs delegation) and provides a
//! free-form decoder.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone)]
pub struct ExchangeRow {
    pub at: i64,
    pub subject: String,
    pub actor: Option<String>,
    pub audience: String,
    pub mode: &'static str, // "impersonation" | "delegation"
    pub scope: Option<String>,
}

pub fn seeded_exchanges(_state: &AdminState) -> Vec<ExchangeRow> {
    vec![
        ExchangeRow {
            at: 1_700_000_010,
            subject: "alice".into(),
            actor: None,
            audience: "api://orders".into(),
            mode: "impersonation",
            scope: Some("read:foo".into()),
        },
        ExchangeRow {
            at: 1_700_000_020,
            subject: "alice".into(),
            actor: Some("svc-orders".into()),
            audience: "api://payments".into(),
            mode: "delegation",
            scope: Some("write:payment".into()),
        },
    ]
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, super::AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let rows = seeded_exchanges(state);
    let tbl_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.at.to_string(),
                escape(&r.subject),
                escape(r.actor.as_deref().unwrap_or("—")),
                escape(&r.audience),
                escape(r.mode),
                escape(r.scope.as_deref().unwrap_or("")),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    RFC 8693 — OAuth 2.0 Token Exchange.
    <span class="px-2 py-0.5 rounded bg-blue-100 text-blue-800 ml-2">grant_type=urn:ietf:params:oauth:grant-type:token-exchange</span>
  </p>
  <h2 class="text-lg font-semibold mb-2">Recent exchanges</h2>
  {tbl}
  <h2 class="text-lg font-semibold mt-6 mb-2">Try it</h2>
  <form method="post" action="/admin/auth/token-exchange/try">
    <label class="block text-sm">subject_token<textarea name="subject_token" rows="2" class="w-full font-mono text-xs border rounded p-2"></textarea></label>
    <label class="block text-sm mt-2">actor_token (optional)<textarea name="actor_token" rows="2" class="w-full font-mono text-xs border rounded p-2"></textarea></label>
    <label class="block text-sm mt-2">audience<input name="audience" class="w-full text-xs border rounded p-2"></label>
    <button type="submit" class="mt-2 px-3 py-1 rounded bg-blue-600 text-white text-sm">Exchange</button>
  </form>
</section>"#,
        tbl = table(
            &["at", "subject", "actor", "audience", "mode", "scope"],
            &tbl_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/token-exchange",
        "auth · token-exchange",
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
    fn render_shows_both_modes() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("impersonation"));
        assert!(html.contains("delegation"));
    }

    #[test]
    fn render_requires_permission() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_try_form() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("/admin/auth/token-exchange/try"));
        assert!(html.contains("subject_token"));
    }
}
