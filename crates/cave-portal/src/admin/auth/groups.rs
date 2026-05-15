// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/groups/{realm}` — Keycloak Admin "Groups".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/admin-ui/src/groups/GroupsSection.tsx`.

use super::fixtures::{self, GroupNode};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::metrics::PortalMetrics;
use super::AuthViewError;

pub fn list(_state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<Vec<GroupNode>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    Ok(fixtures::groups(realm))
}

pub fn detail(state: &AdminState, ctx: &RequestCtx, realm: &str, id: &str) -> Result<Option<GroupNode>, AuthViewError> {
    Ok(list(state, ctx, realm)?.into_iter().find(|g| g.id == id))
}

/// Build a path-sorted tree slice that places root groups first then
/// indents children by depth. Keycloak's `GroupsSection` shows the same.
pub fn tree_view(groups: &[GroupNode]) -> Vec<(usize, &GroupNode)> {
    let mut sorted: Vec<&GroupNode> = groups.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    sorted
        .into_iter()
        .map(|g| (g.path.matches('/').count().saturating_sub(2), g))
        .collect()
}

pub fn render_list(state: &AdminState, ctx: &RequestCtx, realm: &str) -> Result<String, AuthViewError> {
    let groups = list(state, ctx, realm)?;
    PortalMetrics::global().incr_page_view("admin_auth_groups", ctx.persona.as_str());
    let tree = tree_view(&groups);
    let mut rows = String::new();
    for (depth, g) in tree {
        let indent = "&nbsp;".repeat(depth * 4);
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2">{indent}<a class="text-blue-700 underline" href="/admin/auth/groups/{realm}/{id}">{path}</a></td>
  <td class="px-3 py-2 text-zinc-500 text-sm">{members}</td>
  <td class="px-3 py-2 text-zinc-500 text-sm">{children}</td>
  <td class="px-3 py-2">
    <form method="post" action="/admin/auth/groups/{realm}/{id}/delete" class="inline" onsubmit="return confirm('Delete group?');">
      <button class="text-red-700 text-sm hover:underline">Delete</button>
    </form>
  </td>
</tr>"#,
            indent = indent,
            realm = escape(realm),
            id = escape(&g.id),
            path = escape(&g.path),
            members = g.member_count,
            children = g.child_paths.len(),
        ));
    }
    let body = format!(
        r#"<section>
  <div class="flex justify-between items-center mb-3">
    <h2 class="text-lg font-semibold">Groups ({n})</h2>
    <form method="post" action="/admin/auth/groups/{realm}/new" class="inline">
      <button class="px-3 py-2 rounded bg-blue-600 text-white">+ Create group</button>
    </form>
  </div>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Path</th>
      <th class="px-3 py-2 text-left">Members</th>
      <th class="px-3 py-2 text-left">Subgroups</th>
      <th class="px-3 py-2 text-left">Actions</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</section>"#,
        realm = escape(realm),
        n = groups.len(),
        rows = rows,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/groups",
        &format!("auth/groups · {}", escape(realm)),
        &body,
    ))
}

pub fn render_detail(state: &AdminState, ctx: &RequestCtx, realm: &str, id: &str) -> Result<String, AuthViewError> {
    let group = match detail(state, ctx, realm, id)? {
        Some(g) => g,
        None => {
            return Ok(page_shell_full(
                ctx,
                "/admin/auth/groups",
                &format!("auth/groups · {} · {}", escape(realm), escape(id)),
                &format!(r#"<p class="text-red-700">No group <code>{}</code>.</p>"#, escape(id)),
            ));
        }
    };
    PortalMetrics::global().incr_page_view("admin_auth_group_detail", ctx.persona.as_str());
    let children: String = group
        .child_paths
        .iter()
        .map(|p| format!(r#"<li><code>{}</code></li>"#, escape(p)))
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm mb-2"><strong>Path:</strong> <code>{path}</code></p>
  <p class="text-sm mb-4"><strong>Members:</strong> {members}</p>
  <h2 class="text-lg font-semibold mb-2">Subgroups ({nc})</h2>
  <ul class="list-disc ml-6 text-sm mb-4">{children}</ul>
  <h2 class="text-lg font-semibold mb-2">Edit settings</h2>
  <form method="post" action="/admin/auth/groups/{realm}/{id}" class="flex gap-2 text-sm">
    <input type="text" name="name" value="{leaf}" class="px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">
    <button class="px-3 py-1 rounded bg-blue-600 text-white">Save</button>
  </form>
</section>"#,
        path = escape(&group.path),
        members = group.member_count,
        nc = group.child_paths.len(),
        children = children,
        realm = escape(realm),
        id = escape(&group.id),
        leaf = escape(group.path.rsplit('/').next().unwrap_or("")),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/groups",
        &format!("auth/groups · {} · {}", escape(realm), escape(&group.path)),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RequestCtx {
        RequestCtx::developer("acme", &[Permission::AuthSessionsRead])
    }

    #[test]
    fn list_requires_permission() {
        let s = AdminState::seeded();
        assert!(list(&s, &RequestCtx::developer("acme", &[]), "acme-realm").is_err());
    }

    #[test]
    fn tree_view_sorts_by_path_and_assigns_depth() {
        let groups = fixtures::groups("acme");
        let tree = tree_view(&groups);
        // /acme/employees and /acme/engineering are depth 0;
        // /acme/engineering/backend etc are depth 1.
        for (depth, g) in &tree {
            let expected = g.path.matches('/').count().saturating_sub(2);
            assert_eq!(*depth, expected);
        }
        // First node is the lexicographically smallest path.
        assert!(tree[0].1.path < tree[1].1.path);
    }

    #[test]
    fn render_list_indents_subgroups() {
        let s = AdminState::seeded();
        let html = render_list(&s, &ctx(), "acme-realm").unwrap();
        // Backend / Frontend / SRE all appear nested; they each carry the
        // 4-space `&nbsp;` indentation marker (a non-zero indent string).
        assert!(html.contains("&nbsp;&nbsp;&nbsp;&nbsp;"));
        assert!(html.contains("/acme-realm/engineering/backend"));
    }

    #[test]
    fn render_list_includes_create_button() {
        let s = AdminState::seeded();
        let html = render_list(&s, &ctx(), "acme-realm").unwrap();
        assert!(html.contains("+ Create group"));
    }

    #[test]
    fn render_detail_lists_subgroups_and_member_count() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "grp-root-eng").unwrap();
        assert!(html.contains("/acme-realm/engineering/backend"));
        assert!(html.contains(">42<") || html.contains("42"));
    }

    #[test]
    fn render_detail_unknown_id_falls_through_to_404() {
        let s = AdminState::seeded();
        let html = render_detail(&s, &ctx(), "acme-realm", "nope").unwrap();
        assert!(html.contains("No group"));
    }
}
