// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! `/admin/auth/jwe` — JWE algorithm selector per client + decoder.
//!
//! Lets the operator pick the `alg`/`enc` pair used to encrypt ID tokens or
//! userinfo for a given client.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone)]
pub struct JweClientRow {
    pub realm: String,
    pub client_id: String,
    pub id_token_alg: &'static str,
    pub id_token_enc: &'static str,
    pub userinfo_alg: Option<&'static str>,
    pub userinfo_enc: Option<&'static str>,
}

pub fn seeded_clients(_state: &AdminState) -> Vec<JweClientRow> {
    vec![
        JweClientRow {
            realm: "main".into(),
            client_id: "web-portal".into(),
            id_token_alg: "RSA-OAEP-256",
            id_token_enc: "A256GCM",
            userinfo_alg: None,
            userinfo_enc: None,
        },
        JweClientRow {
            realm: "main".into(),
            client_id: "internal-api".into(),
            id_token_alg: "A256KW",
            id_token_enc: "A128CBC-HS256",
            userinfo_alg: Some("dir"),
            userinfo_enc: Some("A256GCM"),
        },
    ]
}

const SUPPORTED_ALGS: &[&str] = &["RSA-OAEP", "RSA-OAEP-256", "dir", "A128KW", "A256KW"];
const SUPPORTED_ENCS: &[&str] = &["A128GCM", "A256GCM", "A128CBC-HS256", "A256CBC-HS512"];

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, super::AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let rows = seeded_clients(state);
    let tbl_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.realm),
                escape(&r.client_id),
                escape(r.id_token_alg),
                escape(r.id_token_enc),
                escape(r.userinfo_alg.unwrap_or("—")),
                escape(r.userinfo_enc.unwrap_or("—")),
            ]
        })
        .collect();
    let alg_opts: String = SUPPORTED_ALGS
        .iter()
        .map(|a| format!(r#"<option value="{a}">{a}</option>"#))
        .collect();
    let enc_opts: String = SUPPORTED_ENCS
        .iter()
        .map(|e| format!(r#"<option value="{e}">{e}</option>"#))
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    RFC 7516 JSON Web Encryption (JWE). Pick the <code>alg</code>+<code>enc</code>
    pair used to encrypt ID tokens and userinfo responses for each client.
  </p>
  <h2 class="text-lg font-semibold mb-2">Per-client JWE selection</h2>
  {tbl}
  <h2 class="text-lg font-semibold mt-6 mb-2">Set algorithm</h2>
  <form method="post" action="/admin/auth/jwe/set" class="space-y-2">
    <label class="text-sm">client_id <input name="client_id" class="ml-1 text-xs border rounded p-1"/></label>
    <label class="text-sm ml-3">alg <select name="alg" class="ml-1 text-xs border rounded p-1">{alg_opts}</select></label>
    <label class="text-sm ml-3">enc <select name="enc" class="ml-1 text-xs border rounded p-1">{enc_opts}</select></label>
    <button type="submit" class="ml-3 px-3 py-1 rounded bg-blue-600 text-white text-sm">Apply</button>
  </form>
  <h2 class="text-lg font-semibold mt-6 mb-2">JWE decoder</h2>
  <form method="post" action="/admin/auth/jwe/decode">
    <textarea name="jwe" rows="3" class="w-full font-mono text-xs border rounded p-2" placeholder="eyJ…"></textarea>
    <button type="submit" class="mt-2 px-3 py-1 rounded bg-blue-600 text-white text-sm">Decode</button>
  </form>
</section>"#,
        tbl = table(
            &["realm", "client_id", "id-token alg", "id-token enc", "userinfo alg", "userinfo enc"],
            &tbl_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/jwe",
        "auth · jwe",
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
    fn render_lists_clients() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("web-portal"));
        assert!(html.contains("internal-api"));
    }

    #[test]
    fn render_requires_permission() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn supported_algs_present_in_dropdown() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        for a in SUPPORTED_ALGS {
            assert!(html.contains(a), "alg {a} missing");
        }
        for e in SUPPORTED_ENCS {
            assert!(html.contains(e), "enc {e} missing");
        }
    }

    #[test]
    fn render_contains_decoder_form() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("/admin/auth/jwe/decode"));
    }
}
