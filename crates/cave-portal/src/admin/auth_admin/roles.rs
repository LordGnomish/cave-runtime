// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/roles` — Realm roles + client roles + composite
//! roles. Visual port of `js/apps/admin-ui/src/realm-roles/RealmRolesSection.tsx`.

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};
use super::{render_admin_nav, require_platform, AuthAdminError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRow {
    pub name: String,
    pub description: String,
    pub composite: bool,
    pub container: RoleContainer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleContainer {
    Realm,
    Client(String),
}
impl RoleContainer {
    pub fn label(&self) -> String {
        match self {
            RoleContainer::Realm => "realm".to_string(),
            RoleContainer::Client(c) => format!("client:{c}"),
        }
    }
}

pub fn seeded_roles() -> Vec<RoleRow> {
    vec![
        RoleRow {
            name: "platform_admin".into(),
            description: "Full platform access".into(),
            composite: true,
            container: RoleContainer::Realm,
        },
        RoleRow {
            name: "tenant_admin".into(),
            description: "Tenant administrator".into(),
            composite: false,
            container: RoleContainer::Realm,
        },
        RoleRow {
            name: "developer".into(),
            description: "Service-account developer".into(),
            composite: false,
            container: RoleContainer::Realm,
        },
        RoleRow {
            name: "view-realm".into(),
            description: "${role_view-realm}".into(),
            composite: false,
            container: RoleContainer::Client("realm-management".into()),
        },
        RoleRow {
            name: "manage-clients".into(),
            description: "${role_manage-clients}".into(),
            composite: false,
            container: RoleContainer::Client("realm-management".into()),
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let roles = seeded_roles();
    let rows: Vec<Vec<String>> = roles
        .iter()
        .map(|r| {
            vec![
                escape(&r.name),
                escape(&r.description),
                if r.composite {
                    r#"<span class="text-amber-700">composite</span>"#.into()
                } else {
                    r#"<span class="text-zinc-500">single</span>"#.into()
                },
                escape(&r.container.label()),
                format!(
                    r#"<a class="text-blue-700 underline" href="/admin/auth/roles/{n}">edit</a>"#,
                    n = escape(&r.name)
                ),
            ]
        })
        .collect();
    let body = format!(
        r#"{nav}
<section>
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-lg font-semibold">Roles ({n})</h2>
    <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/roles/new">Create role</a>
  </div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Realm + client + composite roles.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_roles_resource">Keycloak Roles</a>.
  </p>
  {tbl}
</section>"#,
        nav = render_admin_nav("/admin/auth/roles"),
        n = roles.len(),
        tbl = table_html(&["name", "description", "kind", "container", "action"], &rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/roles",
        &format!("auth/roles · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn seeded_roles_split_into_realm_and_client_containers() {
        let r = seeded_roles();
        assert!(r.iter().any(|x| matches!(x.container, RoleContainer::Realm)));
        assert!(r.iter().any(|x| matches!(x.container, RoleContainer::Client(_))));
    }

    #[test]
    fn role_container_label_formats_client_prefix() {
        assert_eq!(RoleContainer::Realm.label(), "realm");
        assert_eq!(
            RoleContainer::Client("realm-management".into()).label(),
            "client:realm-management"
        );
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_composite_marker_for_composite_role() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains(">composite<"));
        assert!(html.contains("platform_admin"));
    }

    #[test]
    fn render_includes_create_role_button() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Create role"));
        assert!(html.contains("/admin/auth/roles/new"));
    }
}
