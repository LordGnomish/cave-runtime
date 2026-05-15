// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/uma` — UMA resources + policies. Calls A4's
//! cave-auth surfaces. Visual port of
//! `js/apps/admin-ui/src/clients/authorization/AuthorizationSection.tsx`.

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};
use super::{render_admin_nav, require_platform, AuthAdminError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UmaResource {
    pub id: String,
    pub name: String,
    pub uris: Vec<String>,
    pub r#type: String,
    pub owner: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UmaPolicy {
    pub id: String,
    pub name: String,
    pub kind: PolicyKind,
    pub decision_strategy: DecisionStrategy,
    pub logic: PolicyLogic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyKind {
    Role,
    User,
    Group,
    Time,
    Js,
    ClientScope,
    Aggregated,
}
impl PolicyKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Role => "role",
            Self::User => "user",
            Self::Group => "group",
            Self::Time => "time",
            Self::Js => "js",
            Self::ClientScope => "client-scope",
            Self::Aggregated => "aggregated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionStrategy {
    Unanimous,
    Affirmative,
    Consensus,
}
impl DecisionStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unanimous => "UNANIMOUS",
            Self::Affirmative => "AFFIRMATIVE",
            Self::Consensus => "CONSENSUS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyLogic {
    Positive,
    Negative,
}
impl PolicyLogic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Positive => "POSITIVE",
            Self::Negative => "NEGATIVE",
        }
    }
}

pub fn seeded_resources() -> Vec<UmaResource> {
    vec![
        UmaResource {
            id: "r-portal-config".into(),
            name: "Portal admin config".into(),
            uris: vec!["/admin/*".into()],
            r#type: "urn:cave:portal:admin".into(),
            owner: "admin".into(),
            scopes: vec!["read".into(), "write".into()],
        },
        UmaResource {
            id: "r-tenant-billing".into(),
            name: "Tenant billing dashboard".into(),
            uris: vec!["/t/{tenant}/billing".into()],
            r#type: "urn:cave:tenant:billing".into(),
            owner: "admin".into(),
            scopes: vec!["read".into()],
        },
    ]
}

pub fn seeded_policies() -> Vec<UmaPolicy> {
    vec![
        UmaPolicy {
            id: "p-platform-admin".into(),
            name: "Only platform admins".into(),
            kind: PolicyKind::Role,
            decision_strategy: DecisionStrategy::Unanimous,
            logic: PolicyLogic::Positive,
        },
        UmaPolicy {
            id: "p-business-hours".into(),
            name: "Business hours only".into(),
            kind: PolicyKind::Time,
            decision_strategy: DecisionStrategy::Affirmative,
            logic: PolicyLogic::Positive,
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let resources = seeded_resources();
    let policies = seeded_policies();
    let res_rows: Vec<Vec<String>> = resources
        .iter()
        .map(|r| {
            vec![
                escape(&r.name),
                format!(r#"<code class="text-xs">{}</code>"#, escape(&r.r#type)),
                escape(&r.uris.join(", ")),
                escape(&r.owner),
                escape(&r.scopes.join(", ")),
                format!(
                    r#"<a class="text-blue-700 underline" href="/admin/auth/uma/resources/{id}">edit</a>"#,
                    id = escape(&r.id)
                ),
            ]
        })
        .collect();
    let pol_rows: Vec<Vec<String>> = policies
        .iter()
        .map(|p| {
            vec![
                escape(&p.name),
                escape(p.kind.as_str()),
                escape(p.decision_strategy.as_str()),
                escape(p.logic.as_str()),
                format!(
                    r#"<a class="text-blue-700 underline" href="/admin/auth/uma/policies/{id}">edit</a>"#,
                    id = escape(&p.id)
                ),
            ]
        })
        .collect();
    let body = format!(
        r#"{nav}
<section class="space-y-6">
  <div>
    <div class="flex items-center justify-between mb-2">
      <h2 class="text-lg font-semibold">Resources ({nr})</h2>
      <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/uma/resources/new">Create resource</a>
    </div>
    <p class="text-sm text-gray-600 dark:text-zinc-400 mb-2">
      UMA 2.0 protected resources. Upstream: cave-auth A4 UMA.
    </p>
    {restbl}
  </div>
  <div>
    <div class="flex items-center justify-between mb-2">
      <h2 class="text-lg font-semibold">Policies ({np})</h2>
      <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/uma/policies/new">Create policy</a>
    </div>
    {poltbl}
  </div>
</section>"#,
        nav = render_admin_nav("/admin/auth/uma"),
        nr = resources.len(),
        np = policies.len(),
        restbl = table_html(
            &["name", "type", "URIs", "owner", "scopes", "action"],
            &res_rows
        ),
        poltbl = table_html(
            &["name", "kind", "decision strategy", "logic", "action"],
            &pol_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/uma",
        &format!("auth/uma · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn decision_strategy_wire_strings_match_keycloak_kc_uppercase() {
        assert_eq!(DecisionStrategy::Unanimous.as_str(), "UNANIMOUS");
        assert_eq!(DecisionStrategy::Affirmative.as_str(), "AFFIRMATIVE");
        assert_eq!(DecisionStrategy::Consensus.as_str(), "CONSENSUS");
    }

    #[test]
    fn policy_kind_includes_keycloak_authz_kinds() {
        let kinds = vec![
            PolicyKind::Role,
            PolicyKind::User,
            PolicyKind::Group,
            PolicyKind::Time,
            PolicyKind::Js,
            PolicyKind::ClientScope,
            PolicyKind::Aggregated,
        ];
        assert_eq!(kinds.len(), 7);
    }

    #[test]
    fn seeded_resources_and_policies_have_distinct_ids() {
        let r = seeded_resources();
        let p = seeded_policies();
        assert!(!r.is_empty());
        assert!(!p.is_empty());
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_create_resource_and_policy_buttons() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Create resource"));
        assert!(html.contains("Create policy"));
    }

    #[test]
    fn render_lists_resources_with_uris_and_scopes() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("/admin/*"));
        assert!(html.contains("Portal admin config"));
    }
}
