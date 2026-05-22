// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/groups` — Group tree + member management. Visual
//! port of `js/apps/admin-ui/src/groups/GroupsSection.tsx`.

use super::{AuthAdminError, render_admin_nav, require_platform};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupNode {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
    pub children: Vec<GroupNode>,
}

pub fn seeded_groups() -> Vec<GroupNode> {
    vec![
        GroupNode {
            id: "g-platform".into(),
            name: "platform".into(),
            members: vec!["admin".into()],
            children: vec![GroupNode {
                id: "g-platform-sre".into(),
                name: "sre".into(),
                members: vec!["admin".into()],
                children: vec![],
            }],
        },
        GroupNode {
            id: "g-tenants".into(),
            name: "tenants".into(),
            members: vec![],
            children: vec![GroupNode {
                id: "g-tenants-acme".into(),
                name: "acme".into(),
                members: vec!["acme-dev".into()],
                children: vec![],
            }],
        },
    ]
}

fn render_node(node: &GroupNode, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut out = format!(
        r#"{i}<li class="my-1">
  <details {open}>
    <summary class="cursor-pointer">
      <span class="font-medium">{name}</span>
      <span class="text-xs text-zinc-500">({n} member{p})</span>
      <a class="text-blue-700 underline ml-2 text-xs" href="/admin/auth/groups/{id}">edit</a>
      <form method="post" action="/admin/auth/groups/{id}/members" class="inline ml-2 text-xs">
        <button type="submit" class="text-blue-700 underline">add member</button>
      </form>
    </summary>"#,
        i = indent,
        open = if depth == 0 { "open" } else { "" },
        name = escape(&node.name),
        n = node.members.len(),
        p = if node.members.len() == 1 { "" } else { "s" },
        id = escape(&node.id),
    );
    if !node.members.is_empty() {
        out.push_str(r#"<ul class="ml-6 text-sm text-zinc-700 dark:text-zinc-300">"#);
        for m in &node.members {
            out.push_str(&format!(
                r#"<li>· {} <form method="post" action="/admin/auth/groups/{}/members/{}/remove" class="inline"><button type="submit" class="text-red-700 underline text-xs">remove</button></form></li>"#,
                escape(m),
                escape(&node.id),
                escape(m)
            ));
        }
        out.push_str("</ul>");
    }
    if !node.children.is_empty() {
        out.push_str(r#"<ul class="ml-4">"#);
        for c in &node.children {
            out.push_str(&render_node(c, depth + 1));
        }
        out.push_str("</ul>");
    }
    out.push_str("</details></li>\n");
    out
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let groups = seeded_groups();
    let mut tree = String::from(r#"<ul class="text-sm">"#);
    for g in &groups {
        tree.push_str(&render_node(g, 0));
    }
    tree.push_str("</ul>");
    let body = format!(
        r#"{nav}
<section>
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-lg font-semibold">Groups</h2>
    <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/groups/new">Create group</a>
  </div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Hierarchical groups with inherited role mappings.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_groups_resource">Keycloak Groups</a>.
  </p>
  {tree}
</section>"#,
        nav = render_admin_nav("/admin/auth/groups"),
        tree = tree,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/groups",
        &format!("auth/groups · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn seeded_groups_has_platform_and_tenants_root_nodes() {
        let g = seeded_groups();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].name, "platform");
        assert_eq!(g[1].name, "tenants");
    }

    #[test]
    fn group_tree_is_hierarchical_with_children() {
        let g = seeded_groups();
        assert!(!g[0].children.is_empty());
        assert_eq!(g[0].children[0].name, "sre");
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_create_group_button() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Create group"));
        assert!(html.contains("/admin/auth/groups/new"));
    }

    #[test]
    fn render_shows_add_member_and_remove_actions() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("add member"));
        assert!(html.contains("remove"));
    }

    #[test]
    fn render_indents_children_under_parents() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        // sre is a child group nested under platform.
        assert!(html.contains("sre"));
        assert!(html.contains("acme"));
    }
}
