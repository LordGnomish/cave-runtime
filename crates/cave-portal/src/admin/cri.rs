//! `/admin/cri` view — sandbox list + container inspect + exec terminal
//! placeholder (xterm.js wired client-side, no server-side runtime).
//!
//! Mirrors the pod / container panes Backstage's Kubernetes plugin
//! exposes (`PodsTable`, `ContainerInfo`).

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, htmx_button, page_shell_full, table};
use crate::admin::state::{scope, AdminState, CriContainer, CriSandbox};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CriViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("sandbox {0} not found for this tenant")]
    NotFound(String),
}

pub fn list_sandboxes(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<CriSandbox>, CriViewError> {
    ctx.authorise(Permission::CriRead)?;
    Ok(scope(&state.cri_sandboxes.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn inspect_sandbox(
    state: &AdminState,
    ctx: &RequestCtx,
    sandbox_id: &str,
) -> Result<(CriSandbox, Vec<CriContainer>), CriViewError> {
    ctx.authorise(Permission::CriRead)?;
    let sandboxes = state.cri_sandboxes.read().unwrap();
    let sb = sandboxes
        .iter()
        .find(|s| s.tenant == ctx.tenant && s.sandbox_id == sandbox_id)
        .cloned()
        .ok_or_else(|| CriViewError::NotFound(sandbox_id.into()))?;
    let containers = state.cri_containers.read().unwrap();
    let cs = containers
        .iter()
        .filter(|c| c.tenant == ctx.tenant && c.sandbox_id == sandbox_id)
        .cloned()
        .collect();
    Ok((sb, cs))
}

/// Render the exec terminal placeholder. The xterm.js client takes over
/// `<div id="exec-{cid}">` after the page loads.
pub fn render_exec_placeholder(ctx: &RequestCtx, container_id: &str) -> Result<String, CriViewError> {
    ctx.authorise(Permission::CriExec)?;
    let cid = escape(container_id);
    Ok(format!(
        r#"<div id="exec-{cid}" class="rounded border bg-black text-green-300 font-mono p-3"
     data-container-id="{cid}"
     data-tenant="{tenant}">
  <p class="text-gray-500">[connecting xterm.js to {cid}…]</p>
</div>"#,
        cid = cid,
        tenant = escape(ctx.tenant.as_str()),
    ))
}

pub fn render_list_page(state: &AdminState, ctx: &RequestCtx) -> Result<String, CriViewError> {
    let rows = list_sandboxes(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|s| vec![s.sandbox_id.clone(), s.pod_name.clone(), s.state.to_string()])
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Sandboxes ({n})</h2>{tbl}
<div class="mt-4">{btn}</div></section>"#,
        n = rows.len(),
        tbl = table(&["sandbox", "pod", "state"], &table_rows),
        btn = htmx_button("/admin/cri?refresh=1", "main", "Refresh"),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/cri",
        &format!("cri · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/Pods/PodsTable.tsx",
    "PodsTable",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_sandboxes_returns_only_owner_rows() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/PodsTable.tsx",
            "PodsTable",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = list_sandboxes(&state, &ctx(&[Permission::CriRead])).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|s| s.tenant.as_str() == "acme"));
    }

    #[test]
    fn inspect_returns_sandbox_plus_containers() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/Pods.tsx",
            "PodDetails",
            "acme"
        );
        let state = AdminState::seeded();
        let (sb, cs) = inspect_sandbox(&state, &ctx(&[Permission::CriRead]), "sb-1").unwrap();
        assert_eq!(sb.pod_name, "web-0");
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].image, "nginx:1.27");
    }

    #[test]
    fn inspect_refuses_foreign_sandbox_with_not_found() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/Pods.tsx",
            "PodDetails",
            "acme"
        );
        let state = AdminState::seeded();
        let err = inspect_sandbox(&state, &ctx(&[Permission::CriRead]), "sb-evil").unwrap_err();
        assert!(matches!(err, CriViewError::NotFound(_)));
    }

    #[test]
    fn exec_placeholder_requires_exec_permission() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Exec/ExecTerminal.tsx",
            "ExecTerminal",
            "acme"
        );
        let no_perm = ctx(&[Permission::CriRead]);
        assert!(render_exec_placeholder(&no_perm, "c-1").is_err());
        let with_exec = ctx(&[Permission::CriExec]);
        let html = render_exec_placeholder(&with_exec, "c-1").unwrap();
        assert!(html.contains(r#"id="exec-c-1""#));
        assert!(html.contains(r#"data-container-id="c-1""#));
        assert!(html.contains(r#"data-tenant="acme""#));
    }

    #[test]
    fn render_list_page_includes_count_and_htmx_button() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/PodsPage.tsx",
            "PodsPage",
            "acme"
        );
        let state = AdminState::seeded();
        let html = render_list_page(&state, &ctx(&[Permission::CriRead])).unwrap();
        assert!(html.contains("Sandboxes (2)"));
        assert!(html.contains("hx-get=\"/admin/cri?refresh=1\""));
        assert!(!html.contains("sb-evil"));
    }
}
