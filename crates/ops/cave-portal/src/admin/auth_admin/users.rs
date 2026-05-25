// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/users` — User search / CRUD / impersonate /
//! reset-credentials. Visual port of
//! `js/apps/admin-ui/src/user/UsersSection.tsx`.

use super::{AuthAdminError, render_admin_nav, require_platform};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminUserRow {
    pub user_id: String,
    pub username: String,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub enabled: bool,
    pub email_verified: bool,
}

pub fn seeded_users() -> Vec<AdminUserRow> {
    vec![
        AdminUserRow {
            user_id: "u-admin".into(),
            username: "admin".into(),
            email: "admin@cave.local".into(),
            first_name: "Cluster".into(),
            last_name: "Admin".into(),
            enabled: true,
            email_verified: true,
        },
        AdminUserRow {
            user_id: "u-acme-dev".into(),
            username: "acme-dev".into(),
            email: "dev@acme.local".into(),
            first_name: "Acme".into(),
            last_name: "Developer".into(),
            enabled: true,
            email_verified: true,
        },
        AdminUserRow {
            user_id: "u-disabled".into(),
            username: "disabled-user".into(),
            email: "former@cave.local".into(),
            first_name: "".into(),
            last_name: "".into(),
            enabled: false,
            email_verified: false,
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let users = seeded_users();
    let rows: Vec<Vec<String>> = users
        .iter()
        .map(|u| {
            let actions = format!(
                r#"<a class="text-blue-700 underline mr-2" href="/admin/auth/users/{id}">edit</a>
<a class="text-blue-700 underline mr-2" href="/admin/auth/users/{id}/reset-credentials">reset</a>
<form method="post" action="/admin/auth/users/{id}/impersonate" class="inline">
  <button type="submit" class="text-amber-700 underline">impersonate</button>
</form>"#,
                id = escape(&u.user_id)
            );
            vec![
                escape(&u.username),
                escape(&u.email),
                escape(&u.first_name),
                escape(&u.last_name),
                if u.enabled {
                    r#"<span class="text-green-700">enabled</span>"#.into()
                } else {
                    r#"<span class="text-zinc-500">disabled</span>"#.into()
                },
                if u.email_verified {
                    r#"<span class="text-green-700">✓</span>"#.into()
                } else {
                    r#"<span class="text-amber-700">unverified</span>"#.into()
                },
                actions,
            ]
        })
        .collect();
    let body = format!(
        r#"{nav}
<section>
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-lg font-semibold">Users ({n})</h2>
    <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/users/new">Add user</a>
  </div>
  <form method="get" action="/admin/auth/users" class="flex gap-2 mb-3" role="search">
    <input class="rounded border-gray-300 dark:bg-zinc-800 flex-1" name="search" placeholder="Search by username or email" type="search">
    <button type="submit" class="px-3 py-1 rounded bg-zinc-200 dark:bg-zinc-700">Search</button>
  </form>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_users_resource">Keycloak Users</a>.
  </p>
  {tbl}
</section>"#,
        nav = render_admin_nav("/admin/auth/users"),
        n = users.len(),
        tbl = table_html(
            &[
                "username",
                "email",
                "first",
                "last",
                "status",
                "email verified",
                "actions"
            ],
            &rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/users",
        &format!("auth/users · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn seeded_users_contains_admin_and_dev_and_disabled() {
        let u = seeded_users();
        assert!(u.iter().any(|x| x.username == "admin" && x.enabled));
        assert!(u.iter().any(|x| !x.enabled));
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_add_user_button_and_search_box() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Add user"));
        assert!(html.contains(r#"name="search""#));
        assert!(html.contains(r#"role="search""#));
    }

    #[test]
    fn render_shows_impersonate_and_reset_actions() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("impersonate"));
        assert!(html.contains("reset"));
    }

    #[test]
    fn render_marks_disabled_users_distinctly() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("disabled"));
    }
}
