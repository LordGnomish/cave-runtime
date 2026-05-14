//! Pods tab — kube-state-metrics-style per-pod table plus `restart`
//! mutator. Mirrors the upstream Kubernetes Dashboard's
//! `Pod` list + drawer view.

use super::KubeletViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{scope, AdminState, KubeletPod};

pub fn list_pods(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<KubeletPod>, KubeletViewError> {
    ctx.authorise(Permission::KubeletRead)?;
    Ok(scope(&state.kubelet_pods.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
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
pub fn restart_pod(
    state: &AdminState,
    ctx: &RequestCtx,
    pod: &str,
) -> Result<u32, KubeletViewError> {
    ctx.authorise(Permission::KubeletExec)?;
    let mut pods = state.kubelet_pods.write().unwrap();
    let target = pods
        .iter_mut()
        .find(|p| p.tenant == ctx.tenant && p.pod_name == pod)
        .ok_or_else(|| KubeletViewError::PodNotFound(pod.into()))?;
    target.restart_count += 1;
    Ok(target.restart_count)
}

/// Aggregate pod-status counts across the caller's view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PodSummary {
    pub total: u32,
    pub running: u32,
    pub pending: u32,
    pub failed: u32,
    pub restart_hot: u32,
}

/// Threshold above which a pod's restart_count earns the "hot" badge
/// (matches upstream's restart icon threshold).
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

pub fn pods_with_status<'a>(pods: &'a [KubeletPod], status: &str) -> Vec<&'a KubeletPod> {
    pods.iter().filter(|p| p.status == status).collect()
}

pub fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, KubeletViewError> {
    let pods = list_pods(state, ctx)?;
    let summary = pod_summary(&pods);
    let rows: Vec<Vec<String>> = pods
        .iter()
        .map(|p| {
            vec![
                p.node.clone(),
                p.pod_name.clone(),
                p.status.into(),
                p.restart_count.to_string(),
                if p.restart_count >= RESTART_HOT_THRESHOLD {
                    "🔥".into()
                } else {
                    "".into()
                },
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kubelet-pods" class="mt-2">
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
        tbl = table(
            &["node", "pod", "status", "restarts", ""],
            &rows
        ),
    ))
}

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
        assert!(matches!(
            restart_pod(&s, &c, "x-0").unwrap_err(),
            KubeletViewError::PodNotFound(_)
        ));
    }

    #[test]
    fn restart_pod_requires_exec_perm() {
        let s = AdminState::seeded();
        let c = ctx(&[Permission::KubeletRead]);
        assert!(restart_pod(&s, &c, "web-0").is_err());
    }

    #[test]
    fn pod_summary_counts_by_status() {
        let pods = list_pods(&AdminState::seeded(), &ctx(&[Permission::KubeletRead])).unwrap();
        let s = pod_summary(&pods);
        assert_eq!(s.total, pods.len() as u32);
        assert_eq!(
            s.running + s.pending + s.failed,
            pods.iter()
                .filter(|p| matches!(p.status, "Running" | "Pending" | "Failed"))
                .count() as u32
        );
    }

    #[test]
    fn pods_with_status_filters_correctly() {
        let pods = list_pods(&AdminState::seeded(), &ctx(&[Permission::KubeletRead])).unwrap();
        let running = pods_with_status(&pods, "Running");
        assert!(running.iter().all(|p| p.status == "Running"));
        let zombie = pods_with_status(&pods, "Zombie");
        assert!(zombie.is_empty());
    }

    #[test]
    fn restart_hot_badge_threshold_is_three() {
        use cave_kernel::ns::TenantId;
        let t = TenantId::new("t").unwrap();
        let pods = vec![
            KubeletPod {
                tenant: t.clone(),
                node: "n".into(),
                pod_name: "warm".into(),
                status: "Running",
                restart_count: 2,
            },
            KubeletPod {
                tenant: t.clone(),
                node: "n".into(),
                pod_name: "hot1".into(),
                status: "Running",
                restart_count: 3,
            },
            KubeletPod {
                tenant: t,
                node: "n".into(),
                pod_name: "hot2".into(),
                status: "Running",
                restart_count: 9,
            },
        ];
        let s = pod_summary(&pods);
        assert_eq!(s.restart_hot, 2);
    }

    #[test]
    fn render_section_includes_summary_cards() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        assert!(html.contains("TOTAL"));
        assert!(html.contains("RUNNING"));
        assert!(html.contains("PENDING"));
        assert!(html.contains("FAILED"));
        assert!(html.contains(&format!("HOT (≥{})", RESTART_HOT_THRESHOLD)));
    }
}
