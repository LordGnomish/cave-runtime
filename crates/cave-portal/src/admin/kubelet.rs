//! `/admin/kubelet` view — per-node pod browser.
//!
//! Mirrors the kube-state-metrics-style pod table the Backstage Kubernetes
//! plugin renders for a single node, with a `restart` mutator that
//! requires `KubeletExec`.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, KubeletPod};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KubeletViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("pod {0} not found on this tenant")]
    PodNotFound(String),
}

pub fn list_pods(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<KubeletPod>, KubeletViewError> {
    ctx.authorise(Permission::KubeletRead)?;
    Ok(scope(&state.kubelet_pods.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn pods_on_node(
    state: &AdminState,
    ctx: &RequestCtx,
    node: &str,
) -> Result<Vec<KubeletPod>, KubeletViewError> {
    let all = list_pods(state, ctx)?;
    Ok(all.into_iter().filter(|p| p.node == node).collect())
}

/// Restart bumps the restart_count. Requires KubeletExec.
pub fn restart_pod(state: &AdminState, ctx: &RequestCtx, pod: &str) -> Result<u32, KubeletViewError> {
    ctx.authorise(Permission::KubeletExec)?;
    let mut pods = state.kubelet_pods.write().unwrap();
    let target = pods
        .iter_mut()
        .find(|p| p.tenant == ctx.tenant && p.pod_name == pod)
        .ok_or_else(|| KubeletViewError::PodNotFound(pod.into()))?;
    target.restart_count += 1;
    Ok(target.restart_count)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KubeletViewError> {
    let pods = list_pods(state, ctx)?;
    let rows: Vec<Vec<String>> = pods
        .iter()
        .map(|p| {
            vec![
                p.node.clone(),
                p.pod_name.clone(),
                p.status.into(),
                p.restart_count.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Pods ({n})</h2>{tbl}</section>"#,
        n = pods.len(),
        tbl = table(&["node", "pod", "status", "restarts"], &rows),
    );
    Ok(page_shell(
        &format!("kubelet · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/Pods/PodDrawer.tsx",
    "PodDrawer",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_pods_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/Pods.tsx",
            "PodList",
            "acme"
        );
        let s = AdminState::seeded();
        let p = list_pods(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn pods_on_node_filters() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/PodsByNode.tsx",
            "PodsByNode",
            "acme"
        );
        let s = AdminState::seeded();
        let p = pods_on_node(&s, &ctx(&[Permission::KubeletRead]), "node-a").unwrap();
        assert_eq!(p.len(), 2);
        assert!(p.iter().all(|x| x.node == "node-a"));
    }

    #[test]
    fn restart_pod_bumps_count_and_refuses_cross_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/PodActions.tsx",
            "RestartPod",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::KubeletRead, Permission::KubeletExec]);
        let new_count = restart_pod(&s, &c, "web-0").unwrap();
        assert_eq!(new_count, 1);
        // x-0 belongs to evil; from acme it must look "not found".
        assert!(matches!(
            restart_pod(&s, &c, "x-0").unwrap_err(),
            KubeletViewError::PodNotFound(_)
        ));
    }

    #[test]
    fn restart_pod_requires_exec_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "authorizeExec",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::KubeletRead]);
        assert!(restart_pod(&s, &c, "web-0").is_err());
    }

    #[test]
    fn render_excludes_evil_pods() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/PodsPage.tsx",
            "PodsPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        assert!(html.contains("Pods (3)"));
        assert!(html.contains("web-0"));
        assert!(!html.contains("x-0"));
    }
}
