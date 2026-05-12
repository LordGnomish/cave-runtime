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

/// Aggregate pod-status counts across the caller's view. Mirrors the
/// dashboard add-on's per-namespace stat cards (running / pending /
/// failed / total).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PodSummary {
    pub total: u32,
    pub running: u32,
    pub pending: u32,
    pub failed: u32,
    pub restart_hot: u32,
}

/// Threshold above which a pod's restart_count earns the "hot" badge
/// in the dashboard (matches the Kubernetes Dashboard's restart icon
/// behaviour — flag when ≥3 restarts).
pub const RESTART_HOT_THRESHOLD: u32 = 3;

pub fn pod_summary(pods: &[KubeletPod]) -> PodSummary {
    let mut s = PodSummary {
        total: pods.len() as u32,
        running: 0,
        pending: 0,
        failed: 0,
        restart_hot: 0,
    };
    for p in pods {
        match p.status {
            "Running" => s.running += 1,
            "Pending" => s.pending += 1,
            "Failed" => s.failed += 1,
            _ => {}
        }
        if p.restart_count >= RESTART_HOT_THRESHOLD {
            s.restart_hot += 1;
        }
    }
    s
}

/// Filter pods by status. Mirrors the dashboard's status-pill filter.
pub fn pods_with_status<'a>(pods: &'a [KubeletPod], status: &str) -> Vec<&'a KubeletPod> {
    pods.iter().filter(|p| p.status == status).collect()
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
    let summary = pod_summary(&pods);
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Kubernetes Dashboard per-node Pod view.
    Upstream: <a class="text-blue-700 underline" href="https://github.com/kubernetes/dashboard">github.com/kubernetes/dashboard</a>.
  </p>
  <div class="mb-4 grid grid-cols-5 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">TOTAL</div><div class="text-2xl font-bold">{total}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">RUNNING</div><div class="text-2xl font-bold text-green-700">{running}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">PENDING</div><div class="text-2xl font-bold text-yellow-700">{pending}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">FAILED</div><div class="text-2xl font-bold text-red-700">{failed}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">HOT (≥{thresh})</div><div class="text-2xl font-bold text-orange-700">{hot}</div></div>
  </div>
  <h2 class="text-lg font-semibold mb-2">Pods ({n})</h2>
  {tbl}
</section>"#,
        n = pods.len(),
        total = summary.total,
        running = summary.running,
        pending = summary.pending,
        failed = summary.failed,
        hot = summary.restart_hot,
        thresh = RESTART_HOT_THRESHOLD,
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
    fn pod_summary_counts_by_status() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/Summary.tsx",
            "Summary",
            "acme"
        );
        let pods = list_pods(&AdminState::seeded(), &ctx(&[Permission::KubeletRead])).unwrap();
        let s = pod_summary(&pods);
        assert_eq!(s.total, pods.len() as u32);
        assert_eq!(s.running + s.pending + s.failed, pods.iter().filter(|p| {
            matches!(p.status, "Running" | "Pending" | "Failed")
        }).count() as u32);
    }

    #[test]
    fn pods_with_status_filters_correctly() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/StatusFilter.tsx",
            "StatusFilter",
            "acme"
        );
        let pods = list_pods(&AdminState::seeded(), &ctx(&[Permission::KubeletRead])).unwrap();
        let running = pods_with_status(&pods, "Running");
        assert!(running.iter().all(|p| p.status == "Running"));
        // A made-up status returns empty (no Falsy default classification).
        let zombie = pods_with_status(&pods, "Zombie");
        assert!(zombie.is_empty());
    }

    #[test]
    fn restart_hot_badge_threshold_is_three() {
        // Construct synthetic pods around the threshold to ensure the
        // count is *strictly* `≥ RESTART_HOT_THRESHOLD`.
        use cave_kernel::ns::TenantId;
        let t = TenantId::new("t").unwrap();
        let pods = vec![
            KubeletPod { tenant: t.clone(), node: "n".into(), pod_name: "warm".into(), status: "Running", restart_count: 2 },
            KubeletPod { tenant: t.clone(), node: "n".into(), pod_name: "hot1".into(), status: "Running", restart_count: 3 },
            KubeletPod { tenant: t.clone(), node: "n".into(), pod_name: "hot2".into(), status: "Running", restart_count: 9 },
        ];
        let s = pod_summary(&pods);
        assert_eq!(s.restart_hot, 2);
    }

    #[test]
    fn render_includes_summary_cards_and_upstream_link() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/SummaryCards.tsx",
            "SummaryCards",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KubeletRead])).unwrap();
        assert!(html.contains("TOTAL"));
        assert!(html.contains("RUNNING"));
        assert!(html.contains("github.com/kubernetes/dashboard"));
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
