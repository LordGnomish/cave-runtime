// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/token-exchange` — Token Exchange policies. Calls A4's
//! cave-auth surfaces. Visual port of
//! `js/apps/admin-ui/src/clients/scopes/PermissionsTab.tsx`.

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};
use super::{render_admin_nav, require_platform, AuthAdminError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenExchangePolicy {
    pub id: String,
    pub source_client: String,
    pub target_client: String,
    pub requested_subject: SubjectKind,
    pub allowed_audiences: Vec<String>,
    pub allowed_actors: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    SameSubject,
    Impersonation,
    Delegation,
}
impl SubjectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SameSubject => "same-subject",
            Self::Impersonation => "impersonation",
            Self::Delegation => "delegation (act+sub)",
        }
    }
}

pub fn seeded_policies() -> Vec<TokenExchangePolicy> {
    vec![
        TokenExchangePolicy {
            id: "te-portal-to-apiserver".into(),
            source_client: "cave-portal".into(),
            target_client: "cave-apiserver".into(),
            requested_subject: SubjectKind::Impersonation,
            allowed_audiences: vec!["cave-apiserver".into()],
            allowed_actors: vec!["cave-portal".into()],
            enabled: true,
        },
        TokenExchangePolicy {
            id: "te-gateway-to-svc".into(),
            source_client: "cave-gateway".into(),
            target_client: "cave-llm-gateway".into(),
            requested_subject: SubjectKind::Delegation,
            allowed_audiences: vec!["cave-llm-gateway".into()],
            allowed_actors: vec!["cave-gateway".into()],
            enabled: true,
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let pols = seeded_policies();
    let rows: Vec<Vec<String>> = pols
        .iter()
        .map(|p| {
            vec![
                escape(&p.source_client),
                escape(&p.target_client),
                escape(p.requested_subject.as_str()),
                escape(&p.allowed_audiences.join(", ")),
                escape(&p.allowed_actors.join(", ")),
                if p.enabled {
                    r#"<span class="text-green-700">enabled</span>"#.into()
                } else {
                    r#"<span class="text-zinc-500">disabled</span>"#.into()
                },
                format!(
                    r#"<a class="text-blue-700 underline" href="/admin/auth/token-exchange/{id}">edit</a>"#,
                    id = escape(&p.id)
                ),
            ]
        })
        .collect();
    let body = format!(
        r#"{nav}
<section>
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-lg font-semibold">Token Exchange policies ({n})</h2>
    <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/token-exchange/new">Create policy</a>
  </div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    RFC 8693 Token Exchange — controls which clients may swap which
    subject tokens for which audiences. Upstream: cave-auth A4
    token-exchange.
  </p>
  {tbl}
</section>"#,
        nav = render_admin_nav("/admin/auth/token-exchange"),
        n = pols.len(),
        tbl = table_html(
            &["source client", "target client", "subject kind", "audiences", "actors", "status", "action"],
            &rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/token-exchange",
        &format!("auth/token-exchange · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn subject_kind_covers_same_impersonation_delegation() {
        assert_eq!(SubjectKind::SameSubject.as_str(), "same-subject");
        assert_eq!(SubjectKind::Impersonation.as_str(), "impersonation");
        assert!(SubjectKind::Delegation.as_str().starts_with("delegation"));
    }

    #[test]
    fn seeded_policies_target_apiserver_and_llm_gateway() {
        let p = seeded_policies();
        assert!(p.iter().any(|x| x.target_client == "cave-apiserver"));
        assert!(p.iter().any(|x| x.target_client == "cave-llm-gateway"));
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_create_policy_button_and_lists_audiences() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Create policy"));
        assert!(html.contains("cave-apiserver"));
        assert!(html.contains("impersonation"));
    }
}
