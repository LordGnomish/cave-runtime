// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! `/admin/auth/dpop` — RFC 9449 DPoP admin view.
//!
//! Shows nonce policy + DPoP-bound client roster.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone)]
pub struct DpopClientRow {
    pub realm: String,
    pub client_id: String,
    pub key_alg: &'static str,
    pub jkt_prefix: String,
    pub nonce_required: bool,
}

pub fn seeded_clients(_state: &AdminState) -> Vec<DpopClientRow> {
    vec![
        DpopClientRow {
            realm: "main".into(),
            client_id: "web-spa".into(),
            key_alg: "ES256",
            jkt_prefix: "ZcL7xQrV5N…".into(),
            nonce_required: false,
        },
        DpopClientRow {
            realm: "main".into(),
            client_id: "mobile-app".into(),
            key_alg: "EdDSA",
            jkt_prefix: "f7Wn8sQpRk…".into(),
            nonce_required: true,
        },
    ]
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, super::AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let rows = seeded_clients(state);
    let tbl_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.realm),
                escape(&r.client_id),
                escape(r.key_alg),
                escape(&r.jkt_prefix),
                if r.nonce_required { "yes" } else { "no" }.into(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    RFC 9449 — Demonstration of Proof-of-Possession at the Application Layer.
    Clients listed here issue access tokens bound to their DPoP key via the
    <code class="px-1 bg-gray-100 rounded">cnf.jkt</code> claim.
  </p>
  <h2 class="text-lg font-semibold mb-2">DPoP-bound clients</h2>
  {tbl}
  <h2 class="text-lg font-semibold mt-6 mb-2">Nonce policy</h2>
  <form method="post" action="/admin/auth/dpop/nonce-policy">
    <label class="text-sm">
      Nonce TTL (seconds):
      <input name="ttl_secs" value="60" class="ml-2 w-24 text-xs border rounded p-1">
    </label>
    <label class="text-sm ml-4">
      <input type="checkbox" name="enforce_for_all" class="mr-1">
      Require DPoP-nonce for all clients
    </label>
    <button type="submit" class="ml-3 px-3 py-1 rounded bg-blue-600 text-white text-sm">Save</button>
  </form>
  <h2 class="text-lg font-semibold mt-6 mb-2">DPoP proof inspector</h2>
  <form method="post" action="/admin/auth/dpop/inspect">
    <textarea name="proof" rows="4" class="w-full font-mono text-xs border rounded p-2" placeholder="header.payload.signature"></textarea>
    <button type="submit" class="mt-2 px-3 py-1 rounded bg-blue-600 text-white text-sm">Inspect</button>
  </form>
</section>"#,
        tbl = table(
            &["realm", "client_id", "alg", "jkt", "nonce required"],
            &tbl_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/dpop",
        "auth · dpop",
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
    fn render_shows_clients() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("web-spa"));
        assert!(html.contains("mobile-app"));
    }

    #[test]
    fn render_requires_permission() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_nonce_policy_editor() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("/admin/auth/dpop/nonce-policy"));
        assert!(html.contains("Nonce TTL"));
    }

    #[test]
    fn render_includes_proof_inspector() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("/admin/auth/dpop/inspect"));
    }
}
