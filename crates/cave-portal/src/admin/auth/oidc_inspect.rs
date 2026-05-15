// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/oidc-inspect` — PAR + CIBA + revoked-token inspector.
//!
//! Two-tab debug surface: outstanding PAR `request_uri`s and outstanding
//! CIBA `auth_req_id`s. Useful for operators chasing protocol-level bugs.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParRow {
    pub request_uri: String,
    pub client_id: String,
    pub response_type: String,
    pub expires_in: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CibaRow {
    pub auth_req_id: String,
    pub client_id: String,
    pub username: String,
    pub binding_message: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokedRow {
    pub token: String,
    pub token_type_hint: String,
    pub client_id: String,
}

pub fn seeded_par() -> Vec<ParRow> {
    vec![ParRow {
        request_uri: "urn:ietf:params:oauth:request_uri:abcd1234".into(),
        client_id: "cli1".into(),
        response_type: "code".into(),
        expires_in: 60,
    }]
}

pub fn seeded_ciba() -> Vec<CibaRow> {
    vec![CibaRow {
        auth_req_id: "ciba-5678".into(),
        client_id: "cli-bank".into(),
        username: "alice".into(),
        binding_message: "Authorize $42 transfer".into(),
        status: "pending".into(),
    }]
}

pub fn seeded_revoked() -> Vec<RevokedRow> {
    vec![RevokedRow {
        token: "ey…ab12 (truncated)".into(),
        token_type_hint: "refresh_token".into(),
        client_id: "cli1".into(),
    }]
}

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, super::AuthViewError> {
    ctx.authorise(Permission::AuthOidcInspect)?;
    let par = seeded_par();
    let ciba = seeded_ciba();
    let rev = seeded_revoked();
    let par_rows: Vec<Vec<String>> = par.iter().map(|r| vec![
        escape(&r.request_uri), escape(&r.client_id),
        escape(&r.response_type), r.expires_in.to_string(),
    ]).collect();
    let ciba_rows: Vec<Vec<String>> = ciba.iter().map(|r| vec![
        escape(&r.auth_req_id), escape(&r.client_id),
        escape(&r.username), escape(&r.binding_message), escape(&r.status),
    ]).collect();
    let rev_rows: Vec<Vec<String>> = rev.iter().map(|r| vec![
        escape(&r.token), escape(&r.token_type_hint), escape(&r.client_id),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    PAR (RFC 9126) · CIBA (OIDC Core) · token revocation (RFC 7009) inspector.
    Backends: <code>par_endpoint · ciba_endpoint · revoke_endpoint</code>.
  </p>
  <h2 class="text-lg font-semibold mb-2">Outstanding PAR request_uri ({n_par})</h2>
  {tbl_par}
  <h2 class="text-lg font-semibold mt-6 mb-2">Outstanding CIBA auth_req_id ({n_ciba})</h2>
  {tbl_ciba}
  <h2 class="text-lg font-semibold mt-6 mb-2">Revoked tokens ({n_rev})</h2>
  {tbl_rev}
</section>"#,
        n_par = par.len(),
        n_ciba = ciba.len(),
        n_rev = rev.len(),
        tbl_par = table(&["request_uri", "client_id", "response_type", "expires_in"], &par_rows),
        tbl_ciba = table(&["auth_req_id", "client_id", "user", "binding_message", "status"], &ciba_rows),
        tbl_rev = table(&["token", "token_type_hint", "client_id"], &rev_rows),
    );
    Ok(page_shell_full(ctx, "/admin/auth/oidc-inspect", &format!("auth/oidc-inspect · {}", escape(ctx.tenant.as_str())), &body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_permission() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_has_three_tables() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthOidcInspect])).unwrap();
        assert!(html.contains("Outstanding PAR"));
        assert!(html.contains("Outstanding CIBA"));
        assert!(html.contains("Revoked tokens"));
    }

    #[test]
    fn par_seed_uses_rfc9126_urn() {
        let par = seeded_par();
        assert!(par[0].request_uri.starts_with("urn:ietf:params:oauth:request_uri:"));
    }

    #[test]
    fn ciba_seed_has_binding_message() {
        let ciba = seeded_ciba();
        assert!(!ciba[0].binding_message.is_empty());
    }
}
